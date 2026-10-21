use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use yunshu_workstation_runtime::locale_init::{self, CodexLocaleSeedState};
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::vault::PortableVault;
use yunshu_workstation_runtime::windows_process::replace_file_atomic;

use crate::chat;
use crate::factory_reset::{self, FactorySeedIdentity};
use crate::imagegen;

const INITIALIZATION_SCHEMA_VERSION: u32 = 1;
const JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializationMarker {
    schema_version: u32,
    installation_id_sha256: String,
    factory_manifest_sha256: String,
    codex_locale_manifest_sha256: String,
    completed_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirstUseJournal {
    schema_version: u32,
    operation_id: String,
    state: FirstUseJournalState,
    factory_manifest_sha256: String,
    codex_locale_manifest_sha256: String,
    started_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum FirstUseJournalState {
    Running,
    ReadyToCommit,
}

pub(crate) fn ensure_initialized(paths: &WorkstationPaths) -> Result<()> {
    if initialization_marker(paths).is_file() {
        return verify_completion_marker(paths);
    }

    let has_journal = journal_file(paths).is_file();
    if !has_journal && let Some(path) = first_existing_user_data(paths)? {
        bail!(
            "初始化状态异常：缺少完成标记，但已存在用户数据 {}。为避免覆盖，工作站已停止初始化",
            path.display()
        );
    }

    let factory = factory_reset::verify_factory_seed_identity(paths)
        .context("首次使用初始化无法验证出厂资源")?;
    let locale_manifest_sha256 = verify_locale_seed(paths)?;

    if has_journal {
        if recover_interrupted_transaction(paths, &factory, &locale_manifest_sha256)? {
            return verify_completion_marker(paths);
        }
    }

    execute_transaction(paths, &factory, &locale_manifest_sha256)
}

fn execute_transaction(
    paths: &WorkstationPaths,
    factory: &FactorySeedIdentity,
    locale_manifest_sha256: &str,
) -> Result<()> {
    let operation_id = Uuid::new_v4().to_string();
    let mut journal = FirstUseJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        operation_id,
        state: FirstUseJournalState::Running,
        factory_manifest_sha256: factory.manifest_sha256.clone(),
        codex_locale_manifest_sha256: locale_manifest_sha256.to_string(),
        started_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    write_json_atomic(&journal_file(paths), &journal)?;

    let restored = factory_reset::restore_factory_seed_defaults(paths)?;
    if restored.manifest_sha256 != factory.manifest_sha256 {
        bail!("首次使用初始化期间出厂清单发生变化");
    }

    let locale_result = locale_init::apply_seed_portable_only(paths)
        .context("首次使用初始化无法写入 Codex 中文默认状态")?;
    if let Some(backup) = locale_result.backup_path {
        remove_transaction_path(paths, &backup)?;
    }

    let installation_id = paths.ensure_installation_id()?;
    let vault = PortableVault::open(paths)?;
    vault.save(paths)?;
    chat::initialize_storage(paths)?;
    imagegen::initialize_storage(paths)?;
    verify_empty_workstation(paths)?;

    let verified_factory = factory_reset::verify_factory_seed_identity(paths)?;
    if verified_factory.manifest_sha256 != factory.manifest_sha256 {
        bail!("首次使用初始化后出厂种子哈希发生变化");
    }
    let verified_locale = verify_locale_seed(paths)?;
    if verified_locale != locale_manifest_sha256 {
        bail!("首次使用初始化后 Codex 中文种子哈希发生变化");
    }

    journal.state = FirstUseJournalState::ReadyToCommit;
    journal.updated_at = Utc::now().to_rfc3339();
    write_json_atomic(&journal_file(paths), &journal)?;

    write_completion_marker(paths, factory, locale_manifest_sha256, &installation_id)
}

fn write_completion_marker(
    paths: &WorkstationPaths,
    factory: &FactorySeedIdentity,
    locale_manifest_sha256: &str,
    installation_id: &str,
) -> Result<()> {
    let marker = InitializationMarker {
        schema_version: INITIALIZATION_SCHEMA_VERSION,
        installation_id_sha256: sha256_bytes(installation_id.as_bytes()),
        factory_manifest_sha256: factory.manifest_sha256.clone(),
        codex_locale_manifest_sha256: locale_manifest_sha256.to_owned(),
        completed_at: Utc::now().to_rfc3339(),
    };
    // 完成标记必须是事务最后一次写入；它存在后，后续启动不得再次覆盖用户数据。
    write_json_atomic(&initialization_marker(paths), &marker)
}

fn verify_completion_marker(paths: &WorkstationPaths) -> Result<()> {
    let marker: InitializationMarker =
        read_json(&initialization_marker(paths)).context("首次使用初始化完成标记无法解析")?;
    if marker.schema_version != INITIALIZATION_SCHEMA_VERSION {
        bail!("首次使用初始化完成标记版本不支持");
    }
    let installation_id = paths
        .read_installation_id()
        .ok_or_else(|| anyhow!("初始化标记存在，但工作站安装 ID 丢失"))?;
    if !marker
        .installation_id_sha256
        .eq_ignore_ascii_case(&sha256_bytes(installation_id.as_bytes()))
    {
        bail!("初始化标记与工作站安装 ID 不匹配");
    }
    for hash in [
        &marker.factory_manifest_sha256,
        &marker.codex_locale_manifest_sha256,
    ] {
        if !valid_sha256(hash) {
            bail!("首次使用初始化完成标记包含无效哈希");
        }
    }
    Ok(())
}

fn verify_locale_seed(paths: &WorkstationPaths) -> Result<String> {
    let status = locale_init::inspect_seed(paths);
    if status.state != CodexLocaleSeedState::Ready {
        bail!("Codex 中文种子未就绪：{}", status.message);
    }
    sha256_file(&status.manifest_path)
}

fn recover_interrupted_transaction(
    paths: &WorkstationPaths,
    factory: &FactorySeedIdentity,
    locale_manifest_sha256: &str,
) -> Result<bool> {
    let journal: FirstUseJournal =
        read_json(&journal_file(paths)).context("首次使用初始化事务日志无法解析")?;
    if journal.schema_version != JOURNAL_SCHEMA_VERSION
        || journal.factory_manifest_sha256 != factory.manifest_sha256
        || journal.codex_locale_manifest_sha256 != locale_manifest_sha256
        || !matches!(
            journal.state,
            FirstUseJournalState::Running | FirstUseJournalState::ReadyToCommit
        )
    {
        bail!("检测到无法确认归属的首次初始化残留，已拒绝自动清理");
    }
    match journal.state {
        FirstUseJournalState::Running => {
            cleanup_transaction_outputs(paths)?;
            Ok(false)
        }
        FirstUseJournalState::ReadyToCommit => {
            let installation_id = paths
                .read_installation_id()
                .ok_or_else(|| anyhow!("首次初始化待提交事务缺少工作站安装 ID"))?;
            if !paths.vault_key_file().is_file() || !paths.vault_file().is_file() {
                bail!("首次初始化待提交事务缺少便携凭据库");
            }
            PortableVault::open(paths).context("首次初始化待提交事务的凭据库无效")?;
            verify_empty_workstation(paths)?;
            write_completion_marker(paths, factory, locale_manifest_sha256, &installation_id)?;
            Ok(true)
        }
    }
}

fn cleanup_transaction_outputs(paths: &WorkstationPaths) -> Result<()> {
    for path in transaction_directories(paths) {
        remove_transaction_path(paths, &path)?;
    }
    for path in transaction_files(paths) {
        remove_transaction_path(paths, &path)?;
    }
    clear_runtime_except_journal(paths)?;
    let locale_backups = paths.backups.join("locale-init");
    remove_transaction_path(paths, &locale_backups)?;
    Ok(())
}

fn first_existing_user_data(paths: &WorkstationPaths) -> Result<Option<PathBuf>> {
    for file in transaction_files(paths) {
        if file.exists() {
            return Ok(Some(file));
        }
    }
    for directory in transaction_directories(paths) {
        if directory == paths.codex_home && is_known_empty_codex_scaffold(paths)? {
            continue;
        }
        if directory.is_dir() && fs::read_dir(&directory)?.next().is_some() {
            return Ok(Some(directory));
        }
    }
    if paths.runtime_state.is_dir() {
        for entry in fs::read_dir(&paths.runtime_state)? {
            let entry = entry?;
            if entry.file_name() != "first-use" {
                return Ok(Some(entry.path()));
            }
        }
    }
    Ok(None)
}

fn is_known_empty_codex_scaffold(paths: &WorkstationPaths) -> Result<bool> {
    let root = &paths.codex_home;
    if !root.is_dir() {
        return Ok(false);
    }
    let root_metadata = fs::symlink_metadata(root)?;
    if windows_reparse_point(&root_metadata) {
        return Ok(false);
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = fs::symlink_metadata(entry.path())?;
        if windows_reparse_point(&metadata) || !metadata.is_dir() {
            return Ok(false);
        }
        match name.to_str() {
            Some("skills") if is_empty_directory(&entry.path())? => {}
            Some("plugins") if is_empty_plugins_scaffold(&entry.path())? => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn is_empty_directory(path: &Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir()
        && !windows_reparse_point(&metadata)
        && fs::read_dir(path)?.next().is_none())
}

fn is_empty_plugins_scaffold(path: &Path) -> Result<bool> {
    let mut entries = fs::read_dir(path)?;
    let Some(cache) = entries.next().transpose()? else {
        return Ok(false);
    };
    if entries.next().is_some() || cache.file_name() != std::ffi::OsStr::new("cache") {
        return Ok(false);
    }
    let cache_metadata = fs::symlink_metadata(cache.path())?;
    if !cache_metadata.is_dir() || windows_reparse_point(&cache_metadata) {
        return Ok(false);
    }
    let mut cache_entries = fs::read_dir(cache.path())?;
    let Some(plugins) = cache_entries.next().transpose()? else {
        return Ok(false);
    };
    if cache_entries.next().is_some()
        || plugins.file_name() != std::ffi::OsStr::new("yunshu_plugins")
    {
        return Ok(false);
    }
    is_empty_directory(&plugins.path())
}

fn transaction_directories(paths: &WorkstationPaths) -> Vec<PathBuf> {
    vec![
        paths.codex_home.clone(),
        paths.codex_browser_data.clone(),
        paths.claude_home.clone(),
        paths.openclaw_home.clone(),
        paths.data.join("chat"),
        paths.data.join("integrations").join("image-studio"),
        paths.data.join("ui"),
        paths.data.join("skills"),
        paths.data.join("plugins"),
        paths.data.join("license"),
        paths.secrets.clone(),
        paths.provider_config.clone(),
        paths.workspace.clone(),
        paths.cache.clone(),
        paths.logs.clone(),
    ]
}

fn transaction_files(paths: &WorkstationPaths) -> Vec<PathBuf> {
    vec![
        paths.installation_id_file(),
        paths.factory_generation_file(),
        initialization_marker(paths),
        paths.config.join("codex-proxy.json"),
        paths.config.join("codex-init-proxy.json"),
        paths.config.join("codex-init-proxy-once.json"),
    ]
}

fn clear_runtime_except_journal(paths: &WorkstationPaths) -> Result<()> {
    if !paths.runtime_state.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&paths.runtime_state)? {
        let entry = entry?;
        if entry.file_name() == "first-use" {
            continue;
        }
        remove_transaction_path(paths, &entry.path())?;
    }
    Ok(())
}

fn verify_empty_workstation(paths: &WorkstationPaths) -> Result<()> {
    for file in [
        paths.data.join("license").join("entitlement.json"),
        paths.data.join("license").join("installation.json"),
    ] {
        if file.exists() {
            bail!("首次初始化不应携带授权数据：{}", file.display());
        }
    }
    if paths.provider_config.is_dir() && fs::read_dir(&paths.provider_config)?.next().is_some() {
        bail!("首次初始化不应携带三引擎供应商配置");
    }
    verify_database_empty(&paths.data.join("chat").join("chat.sqlite"), "chat_threads")?;
    verify_database_empty(
        &paths
            .data
            .join("integrations")
            .join("image-studio")
            .join("history.sqlite"),
        "imagegen_jobs",
    )?;
    Ok(())
}

fn verify_database_empty(database: &Path, table: &str) -> Result<()> {
    let connection = Connection::open(database)
        .with_context(|| format!("无法检查空白数据库 {}", database.display()))?;
    let count: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    if count != 0 {
        bail!("首次初始化数据库不是空白状态：{table}");
    }
    Ok(())
}

fn remove_transaction_path(paths: &WorkstationPaths, path: &Path) -> Result<()> {
    if !path.starts_with(&paths.root) {
        bail!("首次初始化拒绝操作产品目录外路径：{}", path.display());
    }
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || windows_reparse_point(&metadata) {
        bail!("首次初始化拒绝操作符号链接：{}", path.display());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        bail!("首次初始化遇到不支持的文件类型：{}", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn initialization_marker(paths: &WorkstationPaths) -> PathBuf {
    paths.data.join("initialization.json")
}

fn journal_file(paths: &WorkstationPaths) -> PathBuf {
    paths.runtime_state.join("first-use").join("operation.json")
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(Into::into)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, path)
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_bytes(&fs::read(path)?))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn completion_marker_prevents_second_initialization() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 路径"));
        fs::create_dir_all(&paths.data).unwrap();
        let installation_id = paths.ensure_installation_id().unwrap();
        let marker = InitializationMarker {
            schema_version: INITIALIZATION_SCHEMA_VERSION,
            installation_id_sha256: sha256_bytes(installation_id.as_bytes()),
            factory_manifest_sha256: "a".repeat(64),
            codex_locale_manifest_sha256: "b".repeat(64),
            completed_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&initialization_marker(&paths), &marker).unwrap();
        fs::create_dir_all(&paths.workspace).unwrap();
        fs::write(paths.workspace.join("用户项目.txt"), b"keep").unwrap();

        ensure_initialized(&paths).unwrap();

        assert_eq!(
            fs::read(paths.workspace.join("用户项目.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn missing_marker_with_existing_user_data_is_rejected_before_seed_lookup() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        fs::create_dir_all(&paths.workspace).unwrap();
        fs::write(paths.workspace.join("project.txt"), b"personal").unwrap();

        let existing = first_existing_user_data(&paths).unwrap();

        assert_eq!(existing.as_deref(), Some(paths.workspace.as_path()));
    }

    #[test]
    fn known_empty_codex_scaffold_is_not_user_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        fs::create_dir_all(paths.codex_home.join("skills")).unwrap();
        fs::create_dir_all(paths.plugins_install_root.clone()).unwrap();

        assert!(is_known_empty_codex_scaffold(&paths).unwrap());
        assert_eq!(first_existing_user_data(&paths).unwrap(), None);
    }

    #[test]
    fn codex_scaffold_with_file_is_user_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        fs::create_dir_all(paths.codex_home.join("skills")).unwrap();
        fs::create_dir_all(paths.plugins_install_root.clone()).unwrap();
        fs::write(
            paths.codex_home.join("skills").join("user-skill.md"),
            b"personal",
        )
        .unwrap();

        assert!(!is_known_empty_codex_scaffold(&paths).unwrap());
        assert_eq!(
            first_existing_user_data(&paths).unwrap(),
            Some(paths.codex_home.clone())
        );
    }

    #[test]
    fn codex_scaffold_with_unknown_directory_is_user_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        fs::create_dir_all(paths.codex_home.join("skills")).unwrap();
        fs::create_dir_all(paths.plugins_install_root.clone()).unwrap();
        fs::create_dir_all(paths.codex_home.join("unknown")).unwrap();

        assert!(!is_known_empty_codex_scaffold(&paths).unwrap());
        assert_eq!(
            first_existing_user_data(&paths).unwrap(),
            Some(paths.codex_home.clone())
        );
    }

    #[test]
    fn marker_rejects_changed_installation_identity() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        fs::create_dir_all(&paths.data).unwrap();
        let marker = InitializationMarker {
            schema_version: INITIALIZATION_SCHEMA_VERSION,
            installation_id_sha256: "a".repeat(64),
            factory_manifest_sha256: "b".repeat(64),
            codex_locale_manifest_sha256: "c".repeat(64),
            completed_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&initialization_marker(&paths), &marker).unwrap();
        fs::write(
            paths.installation_id_file(),
            "0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();

        assert!(
            verify_completion_marker(&paths)
                .unwrap_err()
                .to_string()
                .contains("安装 ID 不匹配")
        );
    }

    #[test]
    fn ready_to_commit_journal_finishes_without_cleaning_initialized_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        let installation_id = paths.ensure_installation_id().unwrap();
        let vault = PortableVault::open(&paths).unwrap();
        vault.save(&paths).unwrap();
        chat::initialize_storage(&paths).unwrap();
        imagegen::initialize_storage(&paths).unwrap();
        fs::create_dir_all(&paths.workspace).unwrap();
        let sentinel = paths.workspace.join("project.txt");
        fs::write(&sentinel, b"keep").unwrap();
        let factory = FactorySeedIdentity {
            manifest_sha256: "a".repeat(64),
        };
        let locale_hash = "b".repeat(64);
        let journal = FirstUseJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            operation_id: Uuid::new_v4().to_string(),
            state: FirstUseJournalState::ReadyToCommit,
            factory_manifest_sha256: factory.manifest_sha256.clone(),
            codex_locale_manifest_sha256: locale_hash.clone(),
            started_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&journal_file(&paths), &journal).unwrap();

        assert!(recover_interrupted_transaction(&paths, &factory, &locale_hash).unwrap());
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(
            paths.read_installation_id().as_deref(),
            Some(installation_id.as_str())
        );
        verify_completion_marker(&paths).unwrap();
    }

    #[test]
    fn ready_to_commit_journal_refuses_nonempty_user_state_without_cleaning() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        paths.ensure_installation_id().unwrap();
        let vault = PortableVault::open(&paths).unwrap();
        vault.save(&paths).unwrap();
        chat::initialize_storage(&paths).unwrap();
        imagegen::initialize_storage(&paths).unwrap();
        fs::create_dir_all(&paths.provider_config).unwrap();
        let provider = paths.provider_config.join("codex.json");
        fs::write(&provider, b"personal").unwrap();
        let factory = FactorySeedIdentity {
            manifest_sha256: "c".repeat(64),
        };
        let locale_hash = "d".repeat(64);
        let journal = FirstUseJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            operation_id: Uuid::new_v4().to_string(),
            state: FirstUseJournalState::ReadyToCommit,
            factory_manifest_sha256: factory.manifest_sha256.clone(),
            codex_locale_manifest_sha256: locale_hash.clone(),
            started_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&journal_file(&paths), &journal).unwrap();

        assert!(recover_interrupted_transaction(&paths, &factory, &locale_hash).is_err());
        assert_eq!(fs::read(&provider).unwrap(), b"personal");
        assert!(!initialization_marker(&paths).exists());
    }

    #[test]
    #[ignore = "requires YUNSHU_STAGE7_TEST_ROOT pointing to a disposable full candidate copy"]
    fn isolated_stage7_candidate_initializes_once_and_preserves_seed() {
        let root = std::env::var_os("YUNSHU_STAGE7_TEST_ROOT")
            .map(PathBuf::from)
            .expect("YUNSHU_STAGE7_TEST_ROOT is required");
        let paths = WorkstationPaths::from_root(root);
        let factory_manifest = paths.root.join("Factory/manifest.json");
        let locale_manifest = paths.root.join("Factory/Seeds/Codex/manifest.json");
        let factory_before = sha256_file(&factory_manifest).unwrap();
        let locale_before = sha256_file(&locale_manifest).unwrap();

        ensure_initialized(&paths).unwrap();

        assert!(initialization_marker(&paths).is_file());
        assert!(paths.installation_id_file().is_file());
        assert!(paths.vault_key_file().is_file());
        assert!(paths.vault_file().is_file());
        verify_empty_workstation(&paths).unwrap();
        assert_eq!(sha256_file(&factory_manifest).unwrap(), factory_before);
        assert_eq!(sha256_file(&locale_manifest).unwrap(), locale_before);

        fs::create_dir_all(&paths.workspace).unwrap();
        let sentinel = paths.workspace.join("FIRST-USE-SECOND-START.txt");
        fs::write(&sentinel, b"keep").unwrap();
        ensure_initialized(&paths).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(sha256_file(&factory_manifest).unwrap(), factory_before);
        assert_eq!(sha256_file(&locale_manifest).unwrap(), locale_before);
    }
}
