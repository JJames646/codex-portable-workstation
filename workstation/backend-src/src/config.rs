use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use getrandom::fill;
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;
use zeroize::Zeroizing;

use crate::model::Engine;
use crate::paths::WorkstationPaths;

const OFFICIAL_CODEX_PLUGINS: [&str; 4] = [
    "browser@openai-bundled",
    "computer-use@openai-bundled",
    "sites@openai-bundled",
    "visualize@openai-bundled",
];

pub fn prepare_clean_configs(paths: &WorkstationPaths) -> Result<()> {
    ensure_codex_config(paths)?;
    crate::locale::ensure_codex_locale(paths)?;
    ensure_claude_config(paths)?;
    ensure_openclaw_config(paths)?;
    crate::provider::apply_all_configured(paths)?;
    Ok(())
}

/// 只准备当前智能体需要的原生配置，不扫描或修复其他智能体、Skills 和插件。
pub fn prepare_engine_config(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    match engine {
        Engine::Codex => {
            ensure_codex_config(paths)?;
            crate::locale::ensure_codex_locale(paths)?;
        }
        Engine::Claude => ensure_claude_config(paths)?,
        Engine::OpenClaw => ensure_openclaw_config(paths)?,
    }
    Ok(())
}

pub fn ensure_codex_config(paths: &WorkstationPaths) -> Result<()> {
    let config_path = paths.codex_home.join("config.toml");
    let content = repair_portable_root_references(
        &read_optional_text(&config_path)?,
        &windows_config_path(&paths.root),
    );
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析 {}", config_path.display()))?
    };

    if !document.as_table().contains_key("projects") {
        document["projects"] = Item::Table(Table::new());
    }
    let projects = document["projects"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 projects 不是表结构"))?;

    let workspace_key = windows_config_path(&paths.workspace);
    let portable_root = windows_config_path(&paths.root);
    let stale_keys: Vec<String> = projects
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| {
            is_stale_portable_workspace(key, &workspace_key)
                || is_external_windows_project(key, &portable_root)
        })
        .collect();
    for stale_key in stale_keys {
        projects.remove(&stale_key);
    }

    let mut project = projects
        .get(&workspace_key)
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    project["trust_level"] = value("trusted");
    projects.insert(&workspace_key, Item::Table(project));

    ensure_codex_compatibility_in_document(&mut document)?;

    write_text_if_changed(&config_path, &document.to_string())
}

pub(crate) fn ensure_codex_compatibility_in_document(document: &mut DocumentMut) -> Result<()> {
    ensure_toml_table(document.as_table_mut(), "desktop")?;
    let desktop = document["desktop"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 desktop 不是表结构"))?;
    desktop["localeOverride"] = value("zh-CN");

    ensure_toml_table(document.as_table_mut(), "features")?;
    let features = document["features"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 features 不是表结构"))?;
    // 样本只关闭 JS REPL，并保留官方 Apps、Computer Use 和 Integrations 能力。
    // 旧版本工作站曾写入 apps=false 规避单个动态工具兼容问题；这里移除该工作站
    // 生成的全局开关，避免整组官方功能被隐藏。若上游再次出现兼容问题，应在协议层
    // 精确处理具体工具，不能继续关闭整个 Apps 功能。
    if features
        .get("apps")
        .and_then(Item::as_value)
        .and_then(|item| item.as_bool())
        == Some(false)
    {
        features.remove("apps");
    }
    features["js_repl"] = value(false);

    // 这四项是官方桌面端随包 marketplace 的能力开关。E 盘曾因
    // openai-bundled 快照不完整而丢失这些配置，导致 Computer Use、浏览器、
    // Sites 和可视化全部显示叉号。这里只恢复官方开关，不伪造 node_repl 的
    // 动态管道；动态 MCP 配置仍由官方桌面端在加载完整 marketplace 后生成。
    ensure_toml_table(document.as_table_mut(), "plugins")?;
    let plugins = document["plugins"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config.toml 的 plugins 不是表结构"))?;
    for key in OFFICIAL_CODEX_PLUGINS {
        let mut plugin = plugins
            .get(key)
            .and_then(Item::as_table)
            .cloned()
            .unwrap_or_default();
        plugin["enabled"] = value(true);
        plugins.insert(key, Item::Table(plugin));
    }
    Ok(())
}

fn ensure_toml_table(table: &mut Table, key: &str) -> Result<()> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    if !table.get(key).is_some_and(Item::is_table) {
        bail!("config.toml 的 {key} 不是表结构");
    }
    Ok(())
}

pub fn ensure_claude_config(paths: &WorkstationPaths) -> Result<()> {
    let settings_path = paths.claude_home.join("settings.json");
    let mut root = read_json_object(&settings_path)?;
    let env_value = root
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let env = env_value
        .as_object_mut()
        .ok_or_else(|| anyhow!("Claude settings.json 的 env 不是对象"))?;
    env.insert(
        "DISABLE_AUTOUPDATER".to_string(),
        Value::String("1".to_string()),
    );
    root.insert("language".to_string(), Value::String("zh-CN".to_string()));
    write_json_pretty(&settings_path, &Value::Object(root))
}

pub fn ensure_openclaw_config(paths: &WorkstationPaths) -> Result<()> {
    let config_path = paths.openclaw_home.join("openclaw.json");
    let mut root = read_json_object(&config_path)?;

    let gateway = root
        .entry("gateway".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 gateway 不是对象"))?;
    gateway.insert("mode".to_string(), Value::String("local".to_string()));
    gateway.insert("bind".to_string(), Value::String("loopback".to_string()));
    let auth = gateway
        .entry("auth".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 gateway.auth 不是对象"))?;
    auth.insert("mode".to_string(), Value::String("token".to_string()));
    if !auth.get("token").is_some_and(Value::is_string) {
        auth.insert("token".to_string(), Value::String(generate_local_token()?));
    }

    let agents = root
        .entry("agents".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 agents 不是对象"))?;
    let defaults = agents
        .entry("defaults".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 agents.defaults 不是对象"))?;
    defaults.insert(
        "workspace".to_string(),
        Value::String(windows_config_path(&paths.workspace)),
    );
    defaults.insert(
        "timeoutSeconds".to_string(),
        Value::Number(serde_json::Number::from(180_u64)),
    );

    let update = root
        .entry("update".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 update 不是对象"))?;
    update.insert("checkOnStart".to_string(), Value::Bool(false));

    write_json_pretty(&config_path, &Value::Object(root))
}

/// 把本次分配的本地网关端口写回随盘配置。
///
/// OpenClaw 的 `dashboard` 命令会从该配置读取网关地址并安全携带当前 token
/// 打开 Control UI，因此端口必须与实际受控进程保持一致。
pub fn set_openclaw_gateway_port(paths: &WorkstationPaths, port: u16) -> Result<()> {
    if port == 0 {
        return Err(anyhow!("OpenClaw网关端口不能为0"));
    }
    let config_path = paths.openclaw_home.join("openclaw.json");
    let mut root = read_json_object(&config_path)?;
    let gateway = root
        .entry("gateway".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenClaw openclaw.json 的 gateway 不是对象"))?;
    gateway.insert(
        "port".to_string(),
        Value::Number(serde_json::Number::from(port)),
    );
    write_json_pretty(&config_path, &Value::Object(root))
}

/// 在 Rust 进程内生成 OpenClaw Control UI 地址。
///
/// URL 包含本地 Gateway Token，只能直接交给操作系统浏览器：
/// 不得序列化给前端、不得写入日志，也不得放入子进程命令行。
pub fn openclaw_control_url(
    paths: &WorkstationPaths,
    expected_port: u16,
) -> Result<Zeroizing<String>> {
    if expected_port == 0 {
        bail!("OpenClaw网关端口不能为0");
    }
    let config_path = paths.openclaw_home.join("openclaw.json");
    let root = read_json_object(&config_path)?;
    let gateway = root
        .get("gateway")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("OpenClaw配置缺少 gateway"))?;
    if gateway.get("bind").and_then(Value::as_str) != Some("loopback") {
        bail!("OpenClaw Control UI 只允许通过本机回环地址打开");
    }
    if gateway.get("port").and_then(Value::as_u64) != Some(u64::from(expected_port)) {
        bail!("OpenClaw配置端口与当前受控网关不一致");
    }
    let auth = gateway
        .get("auth")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("OpenClaw配置缺少 gateway.auth"))?;
    if auth.get("mode").and_then(Value::as_str) != Some("token") {
        bail!("OpenClaw Control UI 未启用本地 Token 验证");
    }
    let token = auth
        .get("token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("OpenClaw本地 Gateway Token 不完整"))?;

    let base_path = gateway
        .get("controlUi")
        .and_then(Value::as_object)
        .and_then(|control_ui| control_ui.get("basePath"))
        .and_then(Value::as_str)
        .unwrap_or("/");
    let normalized_path = if base_path.trim().is_empty() {
        "/".to_string()
    } else {
        format!("/{}", base_path.trim().trim_matches('/'))
    };
    let mut url = Url::parse(&format!("http://127.0.0.1:{expected_port}/"))?;
    url.set_path(&normalized_path);
    // 当前 token 由本项目生成为十六进制字符串，不含 URL 保留字符。
    url.set_fragment(Some(&format!("token={token}")));
    Ok(Zeroizing::new(url.to_string()))
}

fn generate_local_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes).map_err(|error| anyhow!("无法生成OpenClaw本地Gateway Token：{error}"))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut token, "{byte:02x}")?;
    }
    Ok(token)
}

fn windows_config_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

/// 修复样本配置中遗留的旧盘符。只替换产品自身的顶层目录，不处理任意用户路径，
/// 避免误改用户在其他磁盘上的真实项目。
fn repair_portable_root_references(text: &str, current_root: &str) -> String {
    let current_root = current_root.trim_end_matches('\\');
    let current_drive = current_root.get(..2).map(str::to_ascii_lowercase);
    let portable_directories = [
        "backend",
        "cache",
        "claude",
        "codex",
        "config",
        "data",
        "docs",
        "openclaw",
        "plugins",
        "runtimes",
        "skills",
        "workspace",
    ];
    let mut repaired = text.to_string();

    for drive in b'A'..=b'Z' {
        let drive_prefix = format!("{}:", drive as char);
        if current_drive
            .as_deref()
            .is_some_and(|current| current == drive_prefix.to_ascii_lowercase())
        {
            continue;
        }
        for directory in portable_directories {
            let literal = format!("{drive_prefix}\\{directory}\\");
            let literal_replacement = format!("{current_root}\\{directory}\\");
            repaired = repaired.replace(&literal, &literal_replacement);
            repaired = repaired.replace(&literal.to_ascii_lowercase(), &literal_replacement);

            let escaped = format!("{drive_prefix}\\\\{directory}\\\\");
            let escaped_root = current_root.replace('\\', "\\\\");
            let escaped_replacement = format!("{escaped_root}\\\\{directory}\\\\");
            repaired = repaired.replace(&escaped, &escaped_replacement);
            repaired = repaired.replace(&escaped.to_ascii_lowercase(), &escaped_replacement);
        }
    }
    repaired
}

fn is_stale_portable_workspace(existing: &str, current: &str) -> bool {
    let existing_lower = existing.replace('/', "\\").to_lowercase();
    let current_lower = current.replace('/', "\\").to_lowercase();
    existing_lower != current_lower
        && (existing_lower.ends_with("\\workspace")
            || existing_lower.ends_with("\\data\\documents"))
}

/// 便携工作站不保留其他电脑上的绝对项目路径。否则 Codex 会在启动和输入时预热
/// 不存在的 C:/D: 项目，造成 thread/start 长时间阻塞。相对路径继续保留，当前移动盘
/// 根目录下的项目也继续保留。
fn is_external_windows_project(existing: &str, portable_root: &str) -> bool {
    let existing = existing.replace('/', "\\").to_lowercase();
    let root = portable_root
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase();
    let is_absolute_windows_path =
        existing.as_bytes().get(1) == Some(&b':') && existing.as_bytes().get(2) == Some(&b'\\');
    is_absolute_windows_path && existing != root && !existing.starts_with(&format!("{root}\\"))
}

fn read_optional_text(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let bytes = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    let value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("{} 的顶层不是JSON对象", path.display()))
}

fn write_json_pretty(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_bytes_if_changed(path, &bytes)
}

fn write_text_if_changed(path: &Path, text: &str) -> Result<()> {
    write_bytes_if_changed(path, text.as_bytes())
}

fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        let existing = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
        if existing == bytes {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建配置目录 {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("无法写入 {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn codex_project_trust_moves_with_the_root() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("中文 工作站");
        let paths = WorkstationPaths::from_root(root.clone());
        paths.prepare_layout().unwrap();
        fs::write(
            paths.codex_home.join("config.toml"),
            "[projects.'F:\\\\old\\\\workspace']\ntrust_level = \"trusted\"\n",
        )
        .unwrap();

        ensure_codex_config(&paths).unwrap();

        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(config.contains("trust_level = \"trusted\""));
        assert!(config.to_lowercase().contains("workspace"));
        assert!(!config.contains("F:\\\\old"));
    }

    #[test]
    fn codex_external_machine_project_is_removed_from_portable_config() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("中文 工作站");
        let paths = WorkstationPaths::from_root(root);
        paths.prepare_layout().unwrap();
        fs::write(
            paths.codex_home.join("config.toml"),
            concat!(
                "[projects.'C:\\Users\\Someone\\Documents\\old-project']\n",
                "trust_level = \"trusted\"\n",
            ),
        )
        .unwrap();

        ensure_codex_config(&paths).unwrap();

        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(!config.to_ascii_lowercase().contains("old-project"));
        assert!(config.to_lowercase().contains("workspace"));
    }

    #[test]
    fn codex_portable_runtime_paths_move_with_the_root() {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("中文 工作站").join("dev2");
        let paths = WorkstationPaths::from_root(root.clone());
        paths.prepare_layout().unwrap();
        fs::write(
            paths.codex_home.join("config.toml"),
            concat!(
                "notify = ['F:\\data\\.codex\\notify.exe']\n",
                "[marketplaces.openai-bundled]\n",
                "source = 'F:\\data\\.codex\\plugins'\n",
                "[mcp_servers.node_repl]\n",
                "command = \"F:\\\\runtimes\\\\node\\\\node.exe\"\n",
                "[mcp_servers.node_repl.env]\n",
                "CODEX_HOME = 'F:\\data\\.codex'\n",
            ),
        )
        .unwrap();

        ensure_codex_config(&paths).unwrap();

        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(!config.to_ascii_lowercase().contains("f:\\"));
        assert!(config.contains(&windows_config_path(&root)));
        assert!(config.contains("localeOverride = \"zh-CN\""));
        assert!(config.contains("js_repl = false"));
        assert!(!config.contains("apps = false"));
    }

    #[test]
    fn codex_legacy_apps_override_is_removed_for_sample_compatible_features() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        fs::write(
            paths.codex_home.join("config.toml"),
            "[features]\napps = false\n",
        )
        .unwrap();

        ensure_codex_config(&paths).unwrap();

        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(!config.contains("apps = false"));
        assert!(config.contains("js_repl = false"));
        for plugin in OFFICIAL_CODEX_PLUGINS {
            assert!(config.contains(plugin));
        }
        assert_eq!(config.matches("enabled = true").count(), 4);
    }

    #[test]
    fn openclaw_defaults_to_loopback_without_update_check() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();

        ensure_openclaw_config(&paths).unwrap();

        let value: Value =
            serde_json::from_slice(&fs::read(paths.openclaw_home.join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(value["gateway"]["bind"], "loopback");
        assert_eq!(value["gateway"]["auth"]["mode"], "token");
        assert_eq!(
            value["gateway"]["auth"]["token"].as_str().unwrap().len(),
            64
        );
        assert_eq!(value["update"]["checkOnStart"], false);
        assert_eq!(value["agents"]["defaults"]["timeoutSeconds"], 180);
    }

    #[test]
    fn openclaw_gateway_port_is_persisted_without_replacing_the_token() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        ensure_openclaw_config(&paths).unwrap();

        let before: Value =
            serde_json::from_slice(&fs::read(paths.openclaw_home.join("openclaw.json")).unwrap())
                .unwrap();
        let token = before["gateway"]["auth"]["token"]
            .as_str()
            .unwrap()
            .to_string();

        set_openclaw_gateway_port(&paths, 23456).unwrap();

        let after: Value =
            serde_json::from_slice(&fs::read(paths.openclaw_home.join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(after["gateway"]["port"], 23456);
        assert_eq!(after["gateway"]["auth"]["token"], token);
    }

    #[test]
    fn openclaw_control_url_uses_loopback_port_and_configured_base_path() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        ensure_openclaw_config(&paths).unwrap();
        set_openclaw_gateway_port(&paths, 23456).unwrap();

        let config_path = paths.openclaw_home.join("openclaw.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        value["gateway"]["controlUi"] = serde_json::json!({ "basePath": "/control/" });
        write_json_pretty(&config_path, &value).unwrap();

        let url = openclaw_control_url(&paths, 23456).unwrap();
        assert!(url.starts_with("http://127.0.0.1:23456/control#token="));
        assert!(!url.contains("/control/#"));
    }
}
