use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::paths::WorkstationPaths;
use crate::skills::metadata::{new_operation_id, now_unix_ms, write_json_atomic};
use crate::skills::package::{copy_tree, hash_directory};
use crate::windows_process::replace_file_atomic;

use super::catalog;
use super::{MARKETPLACE_NAME, PLUGINS_SCHEMA_VERSION};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistry {
    pub schema_version: u32,
    pub records: Vec<PluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub id: String,
    pub version: String,
    pub marketplace: String,
    pub install_path: String,
    pub package_sha256: String,
    pub installed_at_unix_ms: u128,
    pub authentication: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallResult {
    pub id: String,
    pub version: String,
    pub installed: bool,
    pub reinstalled: bool,
    pub install_path: String,
    pub package_sha256: String,
    pub authentication: String,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginTransaction {
    schema_version: u32,
    transaction_id: String,
    status: String,
    plugin_id: String,
    version: String,
    target_path: String,
    backup_path: String,
    config_path: String,
    config_backup_path: String,
    config_existed: bool,
    registry_path: String,
    registry_backup_path: String,
    registry_existed: bool,
    created_at_unix_ms: u128,
    message: Option<String>,
}

struct InstallLock {
    path: PathBuf,
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn read_registry(paths: &WorkstationPaths) -> Result<PluginRegistry> {
    if !paths.plugins_registry.is_file() {
        return Ok(PluginRegistry {
            schema_version: PLUGINS_SCHEMA_VERSION,
            records: Vec::new(),
        });
    }
    let bytes = fs::read(&paths.plugins_registry)
        .with_context(|| format!("无法读取 {}", paths.plugins_registry.display()))?;
    let mut registry: PluginRegistry = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析 {}", paths.plugins_registry.display()))?;
    if registry.schema_version == 0 {
        registry.schema_version = PLUGINS_SCHEMA_VERSION;
    }
    if registry.schema_version > PLUGINS_SCHEMA_VERSION {
        bail!("插件注册表 schemaVersion 过新：{}", registry.schema_version);
    }
    Ok(registry)
}

pub fn installation_is_valid(
    paths: &WorkstationPaths,
    id: &str,
    registry_version: Option<&str>,
) -> bool {
    let Some(version) = registry_version else {
        return false;
    };
    let target = paths.plugins_install_root.join(id).join(version);
    target.join(".codex-plugin").join("plugin.json").is_file() && codex_plugin_enabled(paths, id)
}

pub fn ensure_codex_marketplace(paths: &WorkstationPaths, snapshot: &Path) -> Result<()> {
    let config_path = paths.codex_home.join("config.toml");
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析 {}", config_path.display()))?
    };
    ensure_table(&mut document, "marketplaces")?;
    let marketplaces = document["marketplaces"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 marketplaces 不是表结构"))?;
    let mut marketplace = marketplaces
        .get(MARKETPLACE_NAME)
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    marketplace["source_type"] = value("local");
    marketplace["source"] = value(windows_path(snapshot));
    marketplaces.insert(MARKETPLACE_NAME, Item::Table(marketplace));
    write_text_atomic(&config_path, &document.to_string())
}

pub fn codex_marketplace_is_current(paths: &WorkstationPaths, snapshot: &Path) -> bool {
    let config_path = paths.codex_home.join("config.toml");
    let Ok(content) = fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(document) = content.parse::<DocumentMut>() else {
        return false;
    };
    let Some(marketplace) = document
        .as_table()
        .get("marketplaces")
        .and_then(Item::as_table)
        .and_then(|marketplaces| marketplaces.get(MARKETPLACE_NAME))
        .and_then(Item::as_table)
    else {
        return false;
    };
    let expected_source = windows_path(snapshot);
    marketplace.get("source_type").and_then(Item::as_str) == Some("local")
        && marketplace.get("source").and_then(Item::as_str) == Some(expected_source.as_str())
}

pub fn install(paths: &WorkstationPaths, id: &str) -> Result<PluginInstallResult> {
    let _lock = acquire_lock(paths)?;
    repair_pending_transactions(paths)?;
    let (source, version, snapshot) = catalog::source_for(paths, id)?;
    validate_path_component(&version, "插件版本")?;
    catalog::ensure_portable_marketplace_metadata(&snapshot)?;

    let detail = catalog::detail(paths, id)?;
    let transaction_id = new_operation_id("plugin-install")?;
    let transaction_file = paths
        .plugins_transactions
        .join(format!("{transaction_id}.json"));
    let stage = paths
        .plugins_staging
        .join(format!("{transaction_id}-{id}-{version}"));
    let target = paths.plugins_install_root.join(id).join(&version);
    let backup = paths
        .plugins_backups
        .join(&transaction_id)
        .join(id)
        .join(&version);
    let config_path = paths.codex_home.join("config.toml");
    let config_backup = paths
        .plugins_backups
        .join(&transaction_id)
        .join("config.toml.before");
    let registry_backup = paths
        .plugins_backups
        .join(&transaction_id)
        .join("registry.json.before");
    let reinstalled = target.exists();
    let mut transaction = PluginTransaction {
        schema_version: PLUGINS_SCHEMA_VERSION,
        transaction_id: transaction_id.clone(),
        status: "pending".to_string(),
        plugin_id: id.to_string(),
        version: version.clone(),
        target_path: target.to_string_lossy().into_owned(),
        backup_path: backup.to_string_lossy().into_owned(),
        config_path: config_path.to_string_lossy().into_owned(),
        config_backup_path: config_backup.to_string_lossy().into_owned(),
        config_existed: config_path.is_file(),
        registry_path: paths.plugins_registry.to_string_lossy().into_owned(),
        registry_backup_path: registry_backup.to_string_lossy().into_owned(),
        registry_existed: paths.plugins_registry.is_file(),
        created_at_unix_ms: now_unix_ms(),
        message: Some("正在校验移动盘预置插件".to_string()),
    };
    write_json_atomic(&transaction_file, &transaction)?;

    let result = (|| -> Result<PluginInstallResult> {
        if let Some(parent) = config_backup.parent() {
            fs::create_dir_all(parent)?;
        }
        backup_file_if_present(&config_path, &config_backup)?;
        backup_file_if_present(&paths.plugins_registry, &registry_backup)?;

        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        copy_tree(&source, &stage)?;
        let package_sha256 = hash_directory(&stage)?;
        let staged_manifest = stage.join(".codex-plugin").join("plugin.json");
        if !staged_manifest.is_file() {
            bail!("校验失败：插件缺少 .codex-plugin/plugin.json");
        }

        if target.exists() {
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&target, &backup).with_context(|| {
                format!(
                    "无法备份现有插件 {} -> {}",
                    target.display(),
                    backup.display()
                )
            })?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&stage, &target)
            .with_context(|| format!("无法提交插件 {} -> {}", stage.display(), target.display()))?;

        ensure_codex_marketplace(paths, &snapshot)?;
        set_plugin_enabled(paths, id, true)?;

        let mut registry = read_registry(paths)?;
        registry.records.retain(|record| record.id != id);
        registry.records.push(PluginRecord {
            id: id.to_string(),
            version: version.clone(),
            marketplace: MARKETPLACE_NAME.to_string(),
            install_path: target.to_string_lossy().into_owned(),
            package_sha256: package_sha256.clone(),
            installed_at_unix_ms: now_unix_ms(),
            authentication: detail.authentication.clone(),
        });
        registry
            .records
            .sort_by(|left, right| left.id.cmp(&right.id));
        write_json_atomic(&paths.plugins_registry, &registry)?;

        transaction.status = "committed".to_string();
        transaction.message = Some("安装完成".to_string());
        write_json_atomic(&transaction_file, &transaction)?;
        Ok(PluginInstallResult {
            id: id.to_string(),
            version: version.clone(),
            installed: true,
            reinstalled,
            install_path: target.to_string_lossy().into_owned(),
            package_sha256,
            authentication: detail.authentication.clone(),
            next_action: if detail.authentication.eq_ignore_ascii_case("ON_INSTALL") {
                "已安装；请启动 Codex 后在插件页面连接对应第三方账号。".to_string()
            } else {
                "已安装；重新进入 Codex 会话后即可使用。".to_string()
            },
        })
    })();

    if let Err(error) = result {
        rollback_transaction(paths, &transaction)?;
        transaction.status = "rolled-back".to_string();
        transaction.message = Some(error.to_string());
        write_json_atomic(&transaction_file, &transaction)?;
        return Err(error);
    }
    result
}

pub fn repair(paths: &WorkstationPaths) -> Result<serde_json::Value> {
    fs::create_dir_all(&paths.plugins_transactions)?;
    fs::create_dir_all(&paths.plugins_staging)?;
    let repaired = repair_pending_transactions(paths)?;
    if let Ok(snapshot) = catalog::locate_snapshot(paths) {
        catalog::ensure_portable_marketplace_metadata(&snapshot)?;
        ensure_codex_marketplace(paths, &snapshot)?;
    }
    Ok(serde_json::json!({
        "repairedTransactions": repaired,
        "marketplace": MARKETPLACE_NAME
    }))
}

fn repair_pending_transactions(paths: &WorkstationPaths) -> Result<usize> {
    if !paths.plugins_transactions.is_dir() {
        return Ok(0);
    }
    let mut repaired = 0;
    for entry in fs::read_dir(&paths.plugins_transactions)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(mut transaction) = serde_json::from_slice::<PluginTransaction>(&bytes) else {
            continue;
        };
        if transaction.status != "pending" {
            continue;
        }
        rollback_transaction(paths, &transaction)?;
        transaction.status = "repaired".to_string();
        transaction.message = Some("已恢复安装前状态".to_string());
        write_json_atomic(&path, &transaction)?;
        repaired += 1;
    }
    // staging 只保存未提交的临时副本；没有 pending 事务时可以安全移除。
    for entry in fs::read_dir(&paths.plugins_staging)? {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(repaired)
}

fn rollback_transaction(paths: &WorkstationPaths, transaction: &PluginTransaction) -> Result<()> {
    let target = checked_transaction_path(paths, &transaction.target_path)?;
    let backup = checked_transaction_path(paths, &transaction.backup_path)?;
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("无法移除未完成插件 {}", target.display()))?;
    }
    if backup.is_dir() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&backup, &target)?;
    }
    restore_file(
        paths,
        &transaction.config_path,
        &transaction.config_backup_path,
        transaction.config_existed,
    )?;
    restore_file(
        paths,
        &transaction.registry_path,
        &transaction.registry_backup_path,
        transaction.registry_existed,
    )
}

fn restore_file(paths: &WorkstationPaths, target: &str, backup: &str, existed: bool) -> Result<()> {
    let target = checked_transaction_path(paths, target)?;
    let backup = checked_transaction_path(paths, backup)?;
    if existed {
        if backup.is_file() {
            fs::copy(&backup, &target)?;
        }
    } else if target.is_file() {
        fs::remove_file(target)?;
    }
    Ok(())
}

fn checked_transaction_path(paths: &WorkstationPaths, value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || !path.starts_with(&paths.root) {
        bail!("拒绝恢复工作站目录之外的路径：{value}");
    }
    Ok(path)
}

fn acquire_lock(paths: &WorkstationPaths) -> Result<InstallLock> {
    fs::create_dir_all(&paths.plugins_transactions)?;
    let path = paths.plugins_transactions.join(".install.lock");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| "已有插件安装任务正在运行，请等待完成后重试")?;
    Ok(InstallLock { path })
}

fn set_plugin_enabled(paths: &WorkstationPaths, id: &str, enabled: bool) -> Result<()> {
    let config_path = paths.codex_home.join("config.toml");
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析 {}", config_path.display()))?
    };
    ensure_table(&mut document, "plugins")?;
    let plugins = document["plugins"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 plugins 不是表结构"))?;
    let key = format!("{id}@{MARKETPLACE_NAME}");
    let mut plugin = plugins
        .get(&key)
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    plugin["enabled"] = value(enabled);
    plugins.insert(&key, Item::Table(plugin));
    write_text_atomic(&config_path, &document.to_string())
}

fn codex_plugin_enabled(paths: &WorkstationPaths, id: &str) -> bool {
    let Ok(content) = fs::read_to_string(paths.codex_home.join("config.toml")) else {
        return false;
    };
    let Ok(document) = content.parse::<DocumentMut>() else {
        return false;
    };
    let key = format!("{id}@{MARKETPLACE_NAME}");
    document["plugins"][&key]["enabled"].as_bool() == Some(true)
}

fn ensure_table(document: &mut DocumentMut, key: &str) -> Result<()> {
    if !document.as_table().contains_key(key) {
        document[key] = Item::Table(Table::new());
    }
    if !document[key].is_table() {
        bail!("config.toml 的 {key} 不是表结构");
    }
    Ok(())
}

fn write_text_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content.as_bytes())
        .with_context(|| format!("无法写入 {}", temporary.display()))?;
    replace_file_atomic(&temporary, path)
}

fn backup_file_if_present(source: &Path, backup: &Path) -> Result<()> {
    if !source.is_file() {
        return Ok(());
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, backup)
        .with_context(|| format!("无法备份 {} -> {}", source.display(), backup.display()))?;
    Ok(())
}

fn validate_path_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.chars().any(|character| {
            matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        })
    {
        bail!("{label}不能作为安全目录名：{value}");
    }
    Ok(())
}

fn windows_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::catalog::marketplace_path;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, WorkstationPaths) {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 插件测试"));
        paths.prepare_layout().unwrap();
        crate::config::ensure_codex_config(&paths).unwrap();
        let snapshot = paths
            .plugins_payloads
            .join("openai-curated")
            .join("test-commit");
        fs::create_dir_all(snapshot.join(".agents").join("plugins")).unwrap();
        let plugin = snapshot.join("plugins").join("gmail").join(".codex-plugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            marketplace_path(&snapshot),
            r#"{"name":"openai-curated","plugins":[{"name":"gmail","source":{"source":"local","path":"./plugins/gmail"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Communication"}]}"#,
        )
        .unwrap();
        fs::write(
            plugin.join("plugin.json"),
            r#"{"name":"gmail","version":"0.1.3","interface":{"displayName":"Gmail"}}"#,
        )
        .unwrap();
        (temporary, paths)
    }

    #[test]
    fn marketplace_status_returns_false_when_yunshu_entry_is_missing_without_writing() {
        let (_temporary, paths) = fixture();
        let config_path = paths.codex_home.join("config.toml");
        fs::write(
            &config_path,
            r#"[marketplaces.openai-bundled]
source_type = "local"
source = "bundled"
"#,
        )
        .unwrap();
        let before = fs::read(&config_path).unwrap();
        let snapshot = paths
            .plugins_payloads
            .join("openai-curated")
            .join("test-commit");

        assert!(!codex_marketplace_is_current(&paths, &snapshot));
        assert_eq!(fs::read(config_path).unwrap(), before);
    }

    #[test]
    fn marketplace_status_returns_true_for_current_yunshu_entry() {
        let (_temporary, paths) = fixture();
        let snapshot = paths
            .plugins_payloads
            .join("openai-curated")
            .join("test-commit");

        ensure_codex_marketplace(&paths, &snapshot).unwrap();

        assert!(codex_marketplace_is_current(&paths, &snapshot));
    }

    #[test]
    fn install_writes_portable_cache_registry_and_codex_config() {
        let (_temporary, paths) = fixture();
        super::super::prepare(&paths).unwrap();

        let result = install(&paths, "gmail").unwrap();

        assert!(result.installed);
        assert!(result.next_action.contains("连接"));
        assert!(
            paths
                .plugins_install_root
                .join("gmail")
                .join("0.1.3")
                .join(".codex-plugin")
                .join("plugin.json")
                .is_file()
        );
        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(config.contains("yunshu_plugins"));
        assert!(config.contains("gmail@yunshu_plugins"));
        assert_eq!(read_registry(&paths).unwrap().records.len(), 1);
        assert!(installation_is_valid(&paths, "gmail", Some("0.1.3")));
    }

    #[test]
    fn reinstall_is_idempotent_and_preserves_one_registry_record() {
        let (_temporary, paths) = fixture();
        super::super::prepare(&paths).unwrap();
        install(&paths, "gmail").unwrap();
        let second = install(&paths, "gmail").unwrap();
        assert!(second.reinstalled);
        assert_eq!(read_registry(&paths).unwrap().records.len(), 1);
    }

    #[test]
    fn failed_reinstall_restores_previous_plugin_payload() {
        let (_temporary, paths) = fixture();
        super::super::prepare(&paths).unwrap();
        install(&paths, "gmail").unwrap();
        let target = paths
            .plugins_install_root
            .join("gmail")
            .join("0.1.3")
            .join(".codex-plugin")
            .join("plugin.json");
        let before = fs::read(&target).unwrap();

        let source_manifest = paths
            .plugins_payloads
            .join("openai-curated")
            .join("test-commit")
            .join("plugins")
            .join("gmail")
            .join(".codex-plugin")
            .join("plugin.json");
        fs::write(
            &source_manifest,
            r#"{"name":"gmail","version":"0.1.3","description":"changed"}"#,
        )
        .unwrap();
        fs::write(paths.codex_home.join("config.toml"), "not = [valid").unwrap();

        assert!(install(&paths, "gmail").is_err());
        assert_eq!(fs::read(target).unwrap(), before);
    }
}
