use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths::WorkstationPaths;

use super::MARKETPLACE_NAME;

#[derive(Debug, Clone, Deserialize)]
struct Marketplace {
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    plugins: Vec<MarketplaceEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketplaceEntry {
    name: String,
    source: MarketplaceSource,
    #[serde(default)]
    policy: MarketplacePolicy,
    #[serde(default)]
    category: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketplaceSource {
    source: String,
    path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MarketplacePolicy {
    #[serde(default)]
    installation: String,
    #[serde(default)]
    authentication: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChineseOverlay {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    long_description: String,
    #[serde(default)]
    category: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ChineseOverlayFile {
    #[serde(default)]
    plugins: HashMap<String, ChineseOverlay>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCatalogItem {
    pub id: String,
    pub display_name: String,
    pub short_description: String,
    pub long_description: String,
    pub category: String,
    pub capabilities: Vec<String>,
    pub version: String,
    pub authentication: String,
    pub authentication_label: String,
    pub installation: String,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub payload_mode: &'static str,
    pub developer_name: String,
    pub brand_color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCatalogResponse {
    pub items: Vec<PluginCatalogItem>,
    pub total: usize,
    pub page: u32,
    pub page_size: u32,
    pub categories: Vec<String>,
    pub payload_mode: &'static str,
}

pub fn locate_snapshot(paths: &WorkstationPaths) -> Result<PathBuf> {
    let family = paths.plugins_payloads.join("openai-curated");
    if !family.is_dir() {
        bail!("尚未部署 Codex 插件离线快照：{}", family.display());
    }
    let mut candidates = fs::read_dir(&family)
        .with_context(|| format!("无法读取插件快照目录 {}", family.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|value| value.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| marketplace_path(path).is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| anyhow!("插件快照缺少 .agents/plugins/marketplace.json"))
}

pub fn marketplace_path(snapshot: &Path) -> PathBuf {
    snapshot
        .join(".agents")
        .join("plugins")
        .join("marketplace.json")
}

pub fn ensure_portable_marketplace_metadata(snapshot: &Path) -> Result<()> {
    let path = marketplace_path(snapshot);
    let bytes = fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?;
    let mut value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("marketplace.json 顶层不是对象"))?;
    let current = object.get("name").and_then(Value::as_str);
    if current == Some(MARKETPLACE_NAME) {
        return Ok(());
    }
    object.insert(
        "name".to_string(),
        Value::String(MARKETPLACE_NAME.to_string()),
    );
    let mut output = serde_json::to_vec_pretty(&value)?;
    output.push(b'\n');
    fs::write(&path, output).with_context(|| format!("无法写入 {}", path.display()))
}

pub fn list(
    paths: &WorkstationPaths,
    query: Option<&str>,
    category: Option<&str>,
    page: u32,
    page_size: u32,
) -> Result<PluginCatalogResponse> {
    let snapshot = locate_snapshot(paths)?;
    let marketplace = read_marketplace(&snapshot)?;
    let overlays = read_overlays(paths);
    let registry = super::installer::read_registry(paths)?;
    let query = query.unwrap_or_default().trim().to_lowercase();
    let category_filter = category.unwrap_or_default().trim().to_lowercase();
    let mut categories = Vec::new();
    let mut items = Vec::new();
    for entry in marketplace.plugins {
        if !entry.policy.installation.is_empty()
            && !entry.policy.installation.eq_ignore_ascii_case("AVAILABLE")
        {
            continue;
        }
        let source = resolve_plugin_source(&snapshot, &entry)?;
        let manifest = read_manifest(&source, &entry.name)?;
        let overlay = overlays
            .plugins
            .get(&entry.name)
            .cloned()
            .unwrap_or_default();
        let upstream_interface = manifest.get("interface").and_then(Value::as_object);
        let display_name = non_empty(&overlay.display_name)
            .or_else(|| field(upstream_interface, "displayName"))
            .unwrap_or_else(|| entry.name.clone());
        let upstream_category =
            field(upstream_interface, "category").unwrap_or_else(|| entry.category.clone());
        let category_name =
            non_empty(&overlay.category).unwrap_or_else(|| translate_category(&upstream_category));
        if !categories.contains(&category_name) {
            categories.push(category_name.clone());
        }
        let short_description = non_empty(&overlay.short_description)
            .unwrap_or_else(|| format!("{display_name} 的 Codex 官方插件，可从移动盘一键安装。"));
        let long_description = non_empty(&overlay.long_description).unwrap_or_else(|| {
            format!(
                "为 Codex 增加 {display_name} 相关能力。插件主体已预置在移动盘，安装过程不依赖在线下载；实际可用范围由插件清单和第三方账号授权决定。"
            )
        });
        let capabilities = upstream_interface
            .and_then(|interface| interface.get("capabilities"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(translate_capability)
                    .collect()
            })
            .unwrap_or_default();
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0")
            .to_string();
        let installed_record = registry
            .records
            .iter()
            .find(|record| record.id == entry.name);
        let installed = super::installer::installation_is_valid(
            paths,
            &entry.name,
            installed_record.map(|record| record.version.as_str()),
        );
        let item = PluginCatalogItem {
            id: entry.name.clone(),
            display_name,
            short_description,
            long_description,
            category: category_name,
            capabilities,
            version,
            authentication: entry.policy.authentication.clone(),
            authentication_label: translate_authentication(&entry.policy.authentication),
            installation: if installed {
                "installed".to_string()
            } else {
                "available".to_string()
            },
            installed,
            installed_version: installed_record.map(|record| record.version.clone()),
            payload_mode: "usb-preloaded",
            developer_name: field(upstream_interface, "developerName")
                .unwrap_or_else(|| "OpenAI / 上游项目".to_string()),
            brand_color: field(upstream_interface, "brandColor"),
        };
        let matches_query = query.is_empty()
            || item.id.to_lowercase().contains(&query)
            || item.display_name.to_lowercase().contains(&query)
            || item.short_description.to_lowercase().contains(&query);
        let matches_category =
            category_filter.is_empty() || item.category.to_lowercase() == category_filter;
        if matches_query && matches_category {
            items.push(item);
        }
    }
    categories.sort();
    items.sort_by(|left, right| {
        right
            .installed
            .cmp(&left.installed)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    let total = items.len();
    let page = page.max(1);
    let page_size = page_size.clamp(1, 500);
    let start = ((page - 1) as usize).saturating_mul(page_size as usize);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect();
    Ok(PluginCatalogResponse {
        items,
        total,
        page,
        page_size,
        categories,
        payload_mode: "usb-preloaded",
    })
}

pub fn detail(paths: &WorkstationPaths, id: &str) -> Result<PluginCatalogItem> {
    validate_plugin_id(id)?;
    let response = list(paths, Some(id), None, 1, 500)?;
    response
        .items
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| anyhow!("插件不存在：{id}"))
}

pub fn source_for(paths: &WorkstationPaths, id: &str) -> Result<(PathBuf, String, PathBuf)> {
    validate_plugin_id(id)?;
    let snapshot = locate_snapshot(paths)?;
    let marketplace = read_marketplace(&snapshot)?;
    let entry = marketplace
        .plugins
        .into_iter()
        .find(|entry| entry.name == id)
        .ok_or_else(|| anyhow!("插件不在当前离线目录中：{id}"))?;
    let source = resolve_plugin_source(&snapshot, &entry)?;
    let manifest = read_manifest(&source, id)?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.0")
        .to_string();
    Ok((source, version, snapshot))
}

fn read_marketplace(snapshot: &Path) -> Result<Marketplace> {
    let path = marketplace_path(snapshot);
    let bytes = fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))
}

fn read_overlays(paths: &WorkstationPaths) -> ChineseOverlayFile {
    fs::read(&paths.plugins_i18n)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn resolve_plugin_source(snapshot: &Path, entry: &MarketplaceEntry) -> Result<PathBuf> {
    if !entry.source.source.eq_ignore_ascii_case("local") {
        bail!("插件 {} 不是本地预置来源", entry.name);
    }
    validate_relative_source(&entry.source.path)?;
    // 上游 marketplace 的 ./plugins/x 是以仓库快照根目录为基准。
    let source = snapshot.join(entry.source.path.trim_start_matches("./"));
    let canonical_snapshot = snapshot
        .canonicalize()
        .with_context(|| format!("无法解析快照路径 {}", snapshot.display()))?;
    let canonical_source = source
        .canonicalize()
        .with_context(|| format!("插件主体不存在 {}", source.display()))?;
    if !canonical_source.starts_with(&canonical_snapshot) {
        bail!("插件源越出移动盘快照：{}", entry.source.path);
    }
    Ok(canonical_source)
}

fn validate_relative_source(value: &str) -> Result<()> {
    let path = Path::new(value);
    if path.is_absolute()
        || value.contains('\\')
        || value.contains(':')
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("插件源路径不安全：{value}");
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || id
            .chars()
            .any(|value| !(value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.')))
    {
        bail!("插件ID不合法：{id}");
    }
    Ok(())
}

fn read_manifest(source: &Path, expected_id: &str) -> Result<Value> {
    let path = source.join(".codex-plugin").join("plugin.json");
    let bytes = fs::read(&path).with_context(|| format!("插件缺少 {}", path.display()))?;
    let value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?;
    let actual = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actual != expected_id {
        bail!("插件清单名称不匹配：期望 {expected_id}，实际 {actual}");
    }
    Ok(value)
}

fn field(object: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<String> {
    object?
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn translate_category(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "communication" => "沟通协作",
        "productivity" => "效率工具",
        "developer tools" | "development" => "开发工具",
        "creative" | "design" => "创作设计",
        "data" | "analytics" => "数据分析",
        "research" => "研究检索",
        _ => "其他插件",
    }
    .to_string()
}

fn translate_capability(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "interactive" => "交互",
        "write" => "可写入",
        "read" => "只读",
        "search" => "检索",
        other => other,
    }
    .to_string()
}

fn translate_authentication(value: &str) -> String {
    match value.to_ascii_uppercase().as_str() {
        "ON_INSTALL" => "安装后需在 Codex 中连接账号",
        "REQUIRED" => "需要连接第三方账号",
        "NONE" | "" => "无需额外登录",
        _ => "可能需要在 Codex 中完成授权",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn unsafe_marketplace_paths_are_rejected() {
        assert!(validate_relative_source("../outside").is_err());
        assert!(validate_relative_source(r"C:\outside").is_err());
        assert!(validate_relative_source("./plugins/gmail").is_ok());
    }

    #[test]
    fn local_catalog_uses_chinese_fallback_and_search() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 dev2"));
        paths.prepare_layout().unwrap();
        super::super::prepare(&paths).unwrap();
        let snapshot = paths
            .plugins_payloads
            .join("openai-curated")
            .join("test-commit");
        fs::create_dir_all(snapshot.join(".agents").join("plugins")).unwrap();
        fs::create_dir_all(snapshot.join("plugins").join("gmail").join(".codex-plugin")).unwrap();
        fs::write(
            marketplace_path(&snapshot),
            r#"{"name":"openai-curated","plugins":[{"name":"gmail","source":{"source":"local","path":"./plugins/gmail"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Communication"}]}"#,
        )
        .unwrap();
        fs::write(
            snapshot
                .join("plugins")
                .join("gmail")
                .join(".codex-plugin")
                .join("plugin.json"),
            r#"{"name":"gmail","version":"0.1.3","interface":{"displayName":"Gmail","shortDescription":"Read mail","capabilities":["Interactive","Write"]}}"#,
        )
        .unwrap();

        let response = list(&paths, Some("gmail"), None, 1, 20).unwrap();
        assert_eq!(response.total, 1);
        assert!(response.items[0].short_description.contains("移动盘"));
        assert_eq!(response.items[0].category, "沟通协作");
        assert_eq!(
            response.items[0].authentication_label,
            "安装后需在 Codex 中连接账号"
        );
    }
}
