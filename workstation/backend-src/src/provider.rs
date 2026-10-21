use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use getrandom::fill;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;
use zeroize::Zeroizing;

use crate::model::Engine;
use crate::paths::WorkstationPaths;
use crate::vault::PortableVault;
use crate::windows_process::replace_file_atomic;

pub const BRIDGE_PORT: u16 = 17832;
pub(crate) const CODEX_MODEL_CATALOG: &str = "yunshu-model-catalog.json";
const PROVIDER_SCHEMA_VERSION: u32 = 3;
const OPENCLAW_LOCALE_MARKER: &str = "yunshu-phase2-zh-cn";
const OPENCLAW_MIN_TIMEOUT_SECONDS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    Auto,
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

impl ProviderProtocol {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Responses => "responses",
            Self::ChatCompletions => "chat-completions",
            Self::AnthropicMessages => "anthropic-messages",
        }
    }

    pub fn resolve(self, engine: Engine) -> Self {
        match self {
            Self::Auto => match engine {
                Engine::Codex => Self::Responses,
                Engine::Claude => Self::AnthropicMessages,
                Engine::OpenClaw => Self::ChatCompletions,
            },
            protocol => protocol,
        }
    }
}

impl FromStr for ProviderProtocol {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "automatic" | "自动" | "自动识别" => Ok(Self::Auto),
            "responses" | "response" | "openai-responses" => Ok(Self::Responses),
            "chat" | "chat-completions" | "openai-chat" | "openai-completions" => {
                Ok(Self::ChatCompletions)
            }
            "anthropic" | "messages" | "anthropic-messages" => Ok(Self::AnthropicMessages),
            other => bail!("不支持的供应商协议：{other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModels {
    pub default: String,
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
    pub subagent: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<String>,
}

impl ProviderModels {
    pub fn from_default(model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            default: model.clone(),
            opus: model.clone(),
            sonnet: model.clone(),
            haiku: model,
            subagent: "inherit".to_string(),
            available: Vec::new(),
        }
    }

    fn normalized(mut self, legacy_model: &str) -> Self {
        let fallback = legacy_model.trim();
        if self.default.trim().is_empty() {
            self.default = fallback.to_string();
        }
        for value in [&mut self.opus, &mut self.sonnet, &mut self.haiku] {
            if value.trim().is_empty() {
                *value = self.default.clone();
            }
        }
        if self.subagent.trim().is_empty() {
            self.subagent = "inherit".to_string();
        }
        self
    }
}

impl Default for ProviderModels {
    fn default() -> Self {
        Self::from_default(String::new())
    }
}

pub(crate) fn codex_model_ids(config: &ProviderConfig) -> Vec<String> {
    let mut ids = Vec::new();
    for id in std::iter::once(&config.model).chain(&config.models.available) {
        let id = id.trim();
        if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWorkspace {
    pub choose_on_launch: bool,
    pub relative_path: String,
}

impl Default for ProviderWorkspace {
    fn default() -> Self {
        Self {
            choose_on_launch: false,
            relative_path: "workspace".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub schema_version: u32,
    pub engine: Engine,
    pub display_name: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub models: ProviderModels,
    pub protocol: ProviderProtocol,
    #[serde(default = "default_resolved_protocol")]
    pub resolved_protocol: ProviderProtocol,
    #[serde(default)]
    pub workspace: ProviderWorkspace,
    pub credential_ref: String,
    pub local_token_ref: String,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigureProvider {
    pub engine: Engine,
    pub display_name: String,
    pub base_url: String,
    pub model: String,
    pub models: Option<ProviderModels>,
    pub protocol: ProviderProtocol,
    pub workspace: Option<ProviderWorkspace>,
    pub timeout_seconds: u64,
    pub api_key: Option<Zeroizing<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub engine: Engine,
    pub configured: bool,
    pub enabled: bool,
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub models: Option<ProviderModels>,
    pub protocol: Option<ProviderProtocol>,
    pub resolved_protocol: Option<ProviderProtocol>,
    pub workspace: Option<ProviderWorkspace>,
    pub credential_present: bool,
    pub local_token_present: bool,
    pub applied: bool,
    pub config_path: PathBuf,
}

#[derive(Debug)]
pub struct DiscoverProviderModels {
    pub engine: Engine,
    pub base_url: String,
    pub protocol: ProviderProtocol,
    pub timeout_seconds: u64,
    pub api_key: Option<Zeroizing<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscoveryResult {
    pub engine: Engine,
    pub endpoint: String,
    pub models: Vec<String>,
    pub elapsed_ms: u128,
}

pub fn configure(paths: &WorkstationPaths, input: ConfigureProvider) -> Result<ProviderStatus> {
    validate_config_input(&input)?;
    paths.prepare_layout()?;
    let credential_ref = format!("provider-{}", input.engine.slug());
    let local_token_ref = format!("local-{}", input.engine.slug());
    let mut vault = PortableVault::open(paths)?;
    if let Some(api_key) = input.api_key {
        if api_key.trim().is_empty() {
            bail!("API Key不能为空");
        }
        vault.set(credential_ref.clone(), api_key)?;
    } else if !vault.contains(&credential_ref) {
        bail!("首次保存供应商时必须填写API Key");
    }
    if !vault.contains(&local_token_ref) {
        vault.set(local_token_ref.clone(), Zeroizing::new(generate_token()?))?;
    }
    vault.save(paths)?;

    let models = input
        .models
        .unwrap_or_else(|| ProviderModels::from_default(input.model.trim()))
        .normalized(input.model.trim());
    let config = ProviderConfig {
        schema_version: PROVIDER_SCHEMA_VERSION,
        engine: input.engine,
        display_name: input.display_name.trim().to_string(),
        base_url: normalize_base_url(&input.base_url)?,
        model: models.default.clone(),
        models,
        protocol: input.protocol,
        resolved_protocol: input.protocol.resolve(input.engine),
        workspace: input.workspace.unwrap_or_default(),
        credential_ref,
        local_token_ref,
        timeout_seconds: input.timeout_seconds,
        enabled: true,
    };
    write_public_config(paths, &config)?;
    apply(paths, input.engine)?;
    status(paths, input.engine)
}

pub fn load(paths: &WorkstationPaths, engine: Engine) -> Result<ProviderConfig> {
    let path = paths.provider_file(engine.slug());
    let bytes = fs::read(&path).with_context(|| {
        format!(
            "{}供应商尚未配置，请先运行 provider configure {}",
            engine,
            engine.slug()
        )
    })?;
    let mut config: ProviderConfig = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析供应商配置 {}", path.display()))?;
    if config.engine != engine {
        bail!("{}中的引擎标识与文件名不一致", path.display());
    }
    if !matches!(config.schema_version, 2 | PROVIDER_SCHEMA_VERSION) {
        bail!("{}供应商配置版本不兼容：{}", engine, config.schema_version);
    }
    let migrated = config.schema_version == 2;
    config.models = config.models.normalized(&config.model);
    config.model = config.models.default.clone();
    config.resolved_protocol = config.protocol.resolve(engine);
    if config.workspace.relative_path.trim().is_empty() {
        config.workspace = ProviderWorkspace::default();
    }
    if migrated {
        backup_legacy_provider_config(paths, engine, &bytes)?;
        config.schema_version = PROVIDER_SCHEMA_VERSION;
        write_public_config(paths, &config)?;
    }
    Ok(config)
}

pub fn apply_all_configured(paths: &WorkstationPaths) -> Result<()> {
    for engine in Engine::ALL {
        if paths.provider_file(engine.slug()).is_file() {
            apply(paths, engine)?;
        }
    }
    Ok(())
}

pub fn apply(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    let config = load(paths, engine)?;
    if !config.enabled {
        bail!("{engine}供应商已停用");
    }
    validate_public_config(&config)?;
    match engine {
        Engine::Codex => apply_codex(paths, &config),
        Engine::Claude => apply_claude(paths, &config),
        Engine::OpenClaw => {
            apply_openclaw(paths, &config)?;
            ensure_openclaw_locale(paths)
        }
    }
}

/// 判断当前引擎是否真的需要本地协议桥。
///
/// 上游协议与智能体原生协议一致时直接连接供应商，避免在慢U盘上额外启动
/// sidecar、写桥接日志和增加一次本地转发。OpenClaw自身支持这里列出的三种
/// Provider协议，所以始终可以直连；Codex和Claude仅在跨协议时使用桥接。
pub fn requires_bridge(paths: &WorkstationPaths, engine: Engine) -> Result<bool> {
    Ok(config_requires_bridge(&load(paths, engine)?))
}

pub fn config_requires_bridge(config: &ProviderConfig) -> bool {
    match config.engine {
        Engine::Codex => config.resolved_protocol != ProviderProtocol::Responses,
        Engine::Claude => config.resolved_protocol != ProviderProtocol::AnthropicMessages,
        Engine::OpenClaw => false,
    }
}

pub fn status(paths: &WorkstationPaths, engine: Engine) -> Result<ProviderStatus> {
    let config_path = paths.provider_file(engine.slug());
    let config = load(paths, engine).ok();
    let vault = PortableVault::open(paths)?;
    let credential_present = config
        .as_ref()
        .is_some_and(|config| vault.contains(&config.credential_ref));
    let local_token_present = config
        .as_ref()
        .is_some_and(|config| vault.contains(&config.local_token_ref));
    let applied = config
        .as_ref()
        .is_some_and(|config| engine_config_applied(paths, config));
    Ok(ProviderStatus {
        engine,
        configured: config.is_some(),
        enabled: config.as_ref().is_some_and(|config| config.enabled),
        display_name: config.as_ref().map(|config| config.display_name.clone()),
        base_url: config.as_ref().map(|config| config.base_url.clone()),
        model: config.as_ref().map(|config| config.model.clone()),
        models: config.as_ref().map(|config| config.models.clone()),
        protocol: config.as_ref().map(|config| config.protocol),
        resolved_protocol: config.as_ref().map(|config| config.resolved_protocol),
        workspace: config.as_ref().map(|config| config.workspace.clone()),
        credential_present,
        local_token_present,
        applied,
        config_path,
    })
}

pub async fn discover_models(
    paths: &WorkstationPaths,
    input: DiscoverProviderModels,
) -> Result<ModelDiscoveryResult> {
    let configured = load(paths, input.engine).ok();
    let base_url = if input.base_url.trim().is_empty() {
        configured
            .as_ref()
            .map(|config| config.base_url.clone())
            .ok_or_else(|| anyhow!("请先填写供应商 API 地址"))?
    } else {
        normalize_base_url(&input.base_url)?
    };
    let protocol = input.protocol.resolve(input.engine);
    let api_key = match input.api_key {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            let config = configured
                .as_ref()
                .ok_or_else(|| anyhow!("请填写 API Key 或先保存供应商配置"))?;
            upstream_secret(paths, config)?
        }
    };
    if !(5..=600).contains(&input.timeout_seconds) {
        bail!("模型发现超时时间必须在5到600秒之间");
    }
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(input.timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建模型发现客户端")?;
    let mut request = client.get(endpoint.clone());
    match protocol {
        ProviderProtocol::AnthropicMessages => {
            request = request
                .header("x-api-key", api_key.as_str())
                .header("anthropic-version", "2023-06-01");
        }
        ProviderProtocol::Responses | ProviderProtocol::ChatCompletions => {
            request = request.bearer_auth(api_key.as_str());
        }
        ProviderProtocol::Auto => unreachable!("协议在发起请求前已经解析"),
    }
    let started = std::time::Instant::now();
    let response = request.send().await.context("无法连接供应商模型接口")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("无法读取供应商模型接口响应")?;
    if !status.is_success() {
        bail!("模型接口返回 HTTP {}", status.as_u16());
    }
    let value: Value = serde_json::from_slice(&bytes).context("模型接口没有返回有效 JSON")?;
    let mut models = extract_model_ids(&value);
    models.sort();
    models.dedup();
    if models.is_empty() {
        bail!("模型接口返回成功，但没有识别到模型 ID");
    }
    Ok(ModelDiscoveryResult {
        engine: input.engine,
        endpoint: endpoint.to_string(),
        models,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub fn status_all(paths: &WorkstationPaths) -> Result<Vec<ProviderStatus>> {
    Engine::ALL
        .into_iter()
        .map(|engine| status(paths, engine))
        .collect()
}

pub fn upstream_secret(
    paths: &WorkstationPaths,
    config: &ProviderConfig,
) -> Result<Zeroizing<String>> {
    PortableVault::open(paths)?.get(&config.credential_ref)
}

pub fn local_token(paths: &WorkstationPaths, config: &ProviderConfig) -> Result<Zeroizing<String>> {
    PortableVault::open(paths)?.get(&config.local_token_ref)
}

pub fn launch_environment(
    paths: &WorkstationPaths,
    engine: Engine,
) -> Result<Vec<(OsString, OsString)>> {
    let config = load(paths, engine)?;
    if !config.enabled {
        bail!("{engine}供应商已停用");
    }
    let bridged = config_requires_bridge(&config);
    let token = if bridged {
        local_token(paths, &config)?
    } else {
        upstream_secret(paths, &config)?
    };
    let token_env = if bridged {
        local_token_env(engine)
    } else {
        upstream_token_env(engine)
    };
    let mut environment = vec![(OsString::from(token_env), OsString::from(token.as_str()))];
    match engine {
        Engine::Codex => {
            environment.push((
                OsString::from("OPENAI_API_KEY"),
                OsString::from(token.as_str()),
            ));
        }
        Engine::Claude => {
            environment.push((
                OsString::from("ANTHROPIC_AUTH_TOKEN"),
                OsString::from(token.as_str()),
            ));
            environment.push((
                OsString::from("ANTHROPIC_API_KEY"),
                OsString::from(token.as_str()),
            ));
        }
        Engine::OpenClaw => {
            environment.push((
                OsString::from("OPENAI_API_KEY"),
                OsString::from(token.as_str()),
            ));
            environment.push((
                OsString::from("ANTHROPIC_API_KEY"),
                OsString::from(token.as_str()),
            ));
        }
    }
    Ok(environment)
}

pub fn local_token_env(engine: Engine) -> &'static str {
    match engine {
        Engine::Codex => "YUNSHU_CODEX_LOCAL_TOKEN",
        Engine::Claude => "YUNSHU_CLAUDE_LOCAL_TOKEN",
        Engine::OpenClaw => "YUNSHU_OPENCLAW_LOCAL_TOKEN",
    }
}

pub fn upstream_token_env(engine: Engine) -> &'static str {
    match engine {
        Engine::Codex => "YUNSHU_CODEX_API_KEY",
        Engine::Claude => "YUNSHU_CLAUDE_API_KEY",
        Engine::OpenClaw => "YUNSHU_OPENCLAW_API_KEY",
    }
}

fn validate_config_input(input: &ConfigureProvider) -> Result<()> {
    if input.display_name.trim().is_empty() {
        bail!("供应商名称不能为空");
    }
    if input.model.trim().is_empty() {
        bail!("模型名称不能为空");
    }
    if !(5..=600).contains(&input.timeout_seconds) {
        bail!("超时时间必须在5到600秒之间");
    }
    normalize_base_url(&input.base_url)?;
    if let Some(workspace) = &input.workspace {
        validate_workspace_relative_path(&workspace.relative_path)?;
    }
    Ok(())
}

fn validate_public_config(config: &ProviderConfig) -> Result<()> {
    if config.model.trim().is_empty() || config.display_name.trim().is_empty() {
        bail!("{}供应商配置缺少名称或模型", config.engine);
    }
    normalize_base_url(&config.base_url)?;
    Ok(())
}

fn normalize_base_url(input: &str) -> Result<String> {
    let mut url = Url::parse(input.trim()).context("供应商Base URL格式不正确")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("供应商Base URL只允许http或https");
    }
    if url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        bail!("供应商Base URL必须包含主机，且不能带查询参数或片段");
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn write_public_config(paths: &WorkstationPaths, config: &ProviderConfig) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(config)?;
    bytes.push(b'\n');
    write_atomic(&paths.provider_file(config.engine.slug()), &bytes)
}

fn apply_codex(paths: &WorkstationPaths, config: &ProviderConfig) -> Result<()> {
    let path = paths.codex_home.join("config.toml");
    let content = read_optional_text(&path)?;
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析 {}", path.display()))?
    };
    document["model_provider"] = value("yunshu-codex");
    document["model"] = value(config.model.clone());
    // 只展示当前供应商保存的 ID；开放推理选择不代表上游已验证支持这些档位。
    let catalog_path = paths.codex_home.join(CODEX_MODEL_CATALOG);
    let bundled = bundled_codex_models(paths)?;
    let models: Vec<Value> = codex_model_ids(config)
        .into_iter()
        .enumerate()
        .map(|(index, id)| codex_catalog_entry(&id, index, &bundled))
        .collect();
    write_json(&catalog_path, &serde_json::json!({"models": models}))?;
    document["model_catalog_json"] = value(catalog_path.to_string_lossy().as_ref());
    document["preferred_auth_method"] = value("apikey");
    ensure_toml_table(document.as_table_mut(), "model_providers")?;
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex model_providers不是表结构"))?;
    let mut provider = Table::new();
    provider["name"] = value(config.display_name.clone());
    let bridged = config_requires_bridge(config);
    provider["base_url"] = value(if bridged {
        format!("http://127.0.0.1:{BRIDGE_PORT}/codex/v1")
    } else {
        config.base_url.clone()
    });
    provider["wire_api"] = value("responses");
    provider["env_key"] = value(if bridged {
        local_token_env(Engine::Codex)
    } else {
        upstream_token_env(Engine::Codex)
    });
    provider["requires_openai_auth"] = value(false);
    providers.insert("yunshu-codex", Item::Table(provider));
    crate::config::ensure_codex_compatibility_in_document(&mut document)?;
    write_atomic(&path, document.to_string().as_bytes())
}

fn bundled_codex_models(paths: &WorkstationPaths) -> Result<Vec<Value>> {
    if !paths.codex_cli.is_file() {
        return Ok(Vec::new());
    }
    // --bundled 只读当前二进制自带信息，不刷新上游，也不传递供应商凭据。
    let mut command = std::process::Command::new(&paths.codex_cli);
    command
        .args(["debug", "models", "--bundled"])
        .env("CODEX_HOME", &paths.codex_home);
    crate::windows_process::configure_command(&mut command, false);
    let output = command.output().context("无法读取 Codex 内置模型信息")?;
    if !output.status.success() {
        bail!("Codex 内置模型信息读取失败，未替换模型目录");
    }
    let catalog: Value =
        serde_json::from_slice(&output.stdout).context("Codex 内置模型信息格式不正确")?;
    catalog["models"]
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("Codex 内置模型信息缺少 models"))
}

fn codex_catalog_entry(id: &str, index: usize, bundled: &[Value]) -> Value {
    let mut entry = bundled.iter().find(|model| model["slug"] == id).cloned()
        .unwrap_or_else(|| serde_json::json!({
            "slug": id, "display_name": id, "description": id,
            "visibility": "list", "supported_in_api": true, "priority": index,
            "default_reasoning_level": "high",
            "supported_reasoning_levels": (["low", "medium", "high", "xhigh", "max", "ultra"]
                .map(|effort| serde_json::json!({"effort": effort, "description": effort}))),
            "shell_type": "shell_command", "availability_nux": null, "upgrade": null,
            "base_instructions": bundled.first().and_then(|model| model["base_instructions"].as_str())
                .unwrap_or("You are Codex, a coding agent. Follow the user's instructions and use the available tools to complete the task."),
            "supports_reasoning_summaries": true, "supports_reasoning_summary_parameter": false,
            "default_reasoning_summary": "none", "support_verbosity": false,
            "supports_parallel_tool_calls": false, "supports_image_detail_original": false,
            "input_modalities": ["text"], "experimental_supported_tools": [],
            "context_window": null, "max_context_window": null,
            "truncation_policy": {"mode": "tokens", "limit": 10000}
        }));
    if let Some(instructions) = entry["base_instructions"].as_str() {
        // 只移除继承的固定型号声明，不改写工具、安全和协作指令。
        entry["base_instructions"] = Value::String(instructions.replacen(
            "You are Codex, an agent based on GPT-5.",
            "You are Codex, a coding agent.",
            1,
        ));
    }
    entry["visibility"] = Value::String("list".into());
    entry["supported_in_api"] = Value::Bool(true);
    entry["priority"] = Value::from(index);
    entry
}

fn apply_claude(paths: &WorkstationPaths, config: &ProviderConfig) -> Result<()> {
    let path = paths.claude_home.join("settings.json");
    let mut root = read_json_object(&path)?;
    let env = ensure_json_object(&mut root, "env")?;
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        Value::String(if config_requires_bridge(config) {
            format!("http://127.0.0.1:{BRIDGE_PORT}/claude")
        } else {
            anthropic_client_base_url(&config.base_url)
        }),
    );
    env.insert(
        "ANTHROPIC_MODEL".to_string(),
        Value::String(config.models.default.clone()),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
        Value::String(config.models.sonnet.clone()),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
        Value::String(config.models.opus.clone()),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
        Value::String(config.models.haiku.clone()),
    );
    env.insert(
        "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
        Value::String(if config.models.subagent == "inherit" {
            config.models.default.clone()
        } else {
            config.models.subagent.clone()
        }),
    );
    env.insert(
        "DISABLE_AUTOUPDATER".to_string(),
        Value::String("1".to_string()),
    );
    root.insert("language".to_string(), Value::String("zh-CN".to_string()));
    write_json(&path, &Value::Object(root))
}

fn apply_openclaw(paths: &WorkstationPaths, config: &ProviderConfig) -> Result<()> {
    let path = paths.openclaw_home.join("openclaw.json");
    let mut root = read_json_object(&path)?;
    let models = ensure_json_object(&mut root, "models")?;
    let providers = ensure_json_object(models, "providers")?;
    let api = match config.resolved_protocol {
        ProviderProtocol::Auto => unreachable!("已保存配置必须包含解析后的协议"),
        ProviderProtocol::Responses => "openai-responses",
        ProviderProtocol::ChatCompletions => "openai-completions",
        ProviderProtocol::AnthropicMessages => "anthropic-messages",
    };
    providers.insert(
        "yunshu-openclaw".to_string(),
        serde_json::json!({
            "baseUrl": config.base_url,
            "api": api,
            "apiKey": format!("${{{}}}", upstream_token_env(Engine::OpenClaw)),
            // OpenClaw 自身的 provider transport 默认约 30 秒会中止慢请求。
            // 该值必须与工作站供应商超时同步，否则上游完成时下游已经断开。
            "timeoutSeconds": config.timeout_seconds.max(OPENCLAW_MIN_TIMEOUT_SECONDS),
            "models": [{
                "id": config.model,
                "name": config.model,
            }]
        }),
    );
    let agents = ensure_json_object(&mut root, "agents")?;
    let defaults = ensure_json_object(agents, "defaults")?;
    defaults.insert(
        "timeoutSeconds".to_string(),
        Value::Number(serde_json::Number::from(
            config.timeout_seconds.max(OPENCLAW_MIN_TIMEOUT_SECONDS),
        )),
    );
    let model = ensure_json_object(defaults, "model")?;
    model.insert(
        "primary".to_string(),
        Value::String(format!("yunshu-openclaw/{}", config.model)),
    );
    write_json(&path, &Value::Object(root))
}

pub fn workspace_for_launch(paths: &WorkstationPaths, engine: Engine) -> Result<Option<PathBuf>> {
    if engine != Engine::Claude {
        return Ok(None);
    }
    let config = load(paths, engine)?;
    validate_workspace_relative_path(&config.workspace.relative_path)?;
    let relative = PathBuf::from(config.workspace.relative_path.replace('\\', "/"));
    let workspace = paths.root.join(relative);
    fs::create_dir_all(&workspace)
        .with_context(|| format!("无法创建 Claude Code 工作目录 {}", workspace.display()))?;
    Ok(Some(workspace))
}

fn validate_workspace_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value.trim());
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("Claude Code 工作目录必须是工作站根目录内的相对路径");
    }
    Ok(())
}

fn default_resolved_protocol() -> ProviderProtocol {
    ProviderProtocol::ChatCompletions
}

fn anthropic_client_base_url(base_url: &str) -> String {
    base_url
        .strip_suffix("/v1")
        .unwrap_or(base_url)
        .trim_end_matches('/')
        .to_string()
}

fn models_endpoint(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).context("供应商Base URL格式不正确")?;
    let path = url.path().trim_end_matches('/');
    let next = if path.ends_with("/v1") {
        format!("{path}/models")
    } else {
        format!("{path}/v1/models")
    };
    url.set_path(&next);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn extract_model_ids(value: &Value) -> Vec<String> {
    let arrays = [
        value.get("data").and_then(Value::as_array),
        value.get("models").and_then(Value::as_array),
    ];
    arrays
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|item| {
            item.get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn backup_legacy_provider_config(
    paths: &WorkstationPaths,
    engine: Engine,
    bytes: &[u8],
) -> Result<()> {
    let backup = paths
        .backups
        .join("providers")
        .join(format!("{}-schema-v2.json", engine.slug()));
    if backup.is_file() {
        return Ok(());
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建供应商备份目录 {}", parent.display()))?;
    }
    fs::write(&backup, bytes).with_context(|| format!("无法备份旧供应商配置 {}", backup.display()))
}

fn ensure_openclaw_locale(paths: &WorkstationPaths) -> Result<()> {
    let candidates = [
        paths
            .root
            .join("openclaw/app/core/node_modules/openclaw/dist/control-ui/index.html"),
        paths
            .root
            .join("openclaw/app/core/node_modules/clawdbot/dist/control-ui/index.html"),
    ];
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let original = fs::read_to_string(&path)
            .with_context(|| format!("无法读取OpenClaw界面 {}", path.display()))?;
        if original.contains(OPENCLAW_LOCALE_MARKER) {
            continue;
        }
        let backup = localization_backup_path(paths, &path)?;
        if !backup.is_file() {
            write_atomic(&backup, original.as_bytes())?;
        }
        let script = format!(
            r#"<script id="{OPENCLAW_LOCALE_MARKER}">try{{document.documentElement.lang="zh-CN";localStorage.setItem("openclaw.i18n.locale","zh-CN");}}catch(_error){{}}</script>"#
        );
        let localized = if original.contains("</head>") {
            original.replacen("</head>", &format!("{script}</head>"), 1)
        } else {
            format!("{script}{original}")
        };
        write_atomic(&path, localized.as_bytes())?;
    }
    Ok(())
}

fn localization_backup_path(paths: &WorkstationPaths, source: &Path) -> Result<PathBuf> {
    let relative = source
        .strip_prefix(&paths.root)
        .with_context(|| format!("本地化文件不在工作站根目录：{}", source.display()))?;
    Ok(paths
        .backups
        .join("phase2-localization")
        .join(relative)
        .with_extension("html.phase1-original"))
}

fn engine_config_applied(paths: &WorkstationPaths, config: &ProviderConfig) -> bool {
    match config.engine {
        Engine::Codex => {
            let expected_base_url = if config_requires_bridge(config) {
                format!("http://127.0.0.1:{BRIDGE_PORT}/codex/v1")
            } else {
                config.base_url.clone()
            };
            let expected_env_key = if config_requires_bridge(config) {
                local_token_env(Engine::Codex)
            } else {
                upstream_token_env(Engine::Codex)
            };
            read_optional_text(&paths.codex_home.join("config.toml")).is_ok_and(|text| {
                text.contains("yunshu-codex")
                    && text.contains(&config.model)
                    && text.contains(&expected_base_url)
                    && text.contains(expected_env_key)
                    && text.contains("localeOverride")
            })
        }
        Engine::Claude => {
            let expected_base_url = if config_requires_bridge(config) {
                format!("http://127.0.0.1:{BRIDGE_PORT}/claude")
            } else {
                anthropic_client_base_url(&config.base_url)
            };
            read_json_object(&paths.claude_home.join("settings.json")).is_ok_and(|root| {
                root.get("language").and_then(Value::as_str) == Some("zh-CN")
                    && root
                        .get("env")
                        .and_then(Value::as_object)
                        .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                        .and_then(Value::as_str)
                        == Some(expected_base_url.as_str())
                    && root
                        .get("env")
                        .and_then(Value::as_object)
                        .and_then(|env| env.get("ANTHROPIC_MODEL"))
                        .and_then(Value::as_str)
                        == Some(config.models.default.as_str())
                    && root
                        .get("env")
                        .and_then(Value::as_object)
                        .and_then(|env| env.get("ANTHROPIC_DEFAULT_OPUS_MODEL"))
                        .and_then(Value::as_str)
                        == Some(config.models.opus.as_str())
            })
        }
        Engine::OpenClaw => read_optional_text(&paths.openclaw_home.join("openclaw.json"))
            .is_ok_and(|text| {
                text.contains("yunshu-openclaw")
                    && text.contains(&config.model)
                    && text.contains(&config.base_url)
                    && text.contains(upstream_token_env(Engine::OpenClaw))
            }),
    }
}

fn ensure_toml_table(table: &mut Table, key: &str) -> Result<()> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    if !table.get(key).is_some_and(Item::is_table) {
        bail!("{key}不是TOML表结构");
    }
    Ok(())
}

fn ensure_json_object<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    parent
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{key}不是JSON对象"))
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.is_file() {
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
        .ok_or_else(|| anyhow!("{}顶层不是JSON对象", path.display()))
}

fn read_optional_text(path: &Path) -> Result<String> {
    if !path.is_file() {
        return Ok(String::new());
    }
    fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入临时文件 {}", temporary.display()))?;
    replace_file_atomic(&temporary, path)
}

fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes).map_err(|error| anyhow!("无法生成本地访问令牌：{error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn configure_test_provider(
        paths: &WorkstationPaths,
        engine: Engine,
        key: &str,
    ) -> ProviderStatus {
        configure(
            paths,
            ConfigureProvider {
                engine,
                display_name: format!("{engine}测试供应商"),
                base_url: "https://example.invalid/v1".to_string(),
                model: format!("model-{}", engine.slug()),
                models: None,
                protocol: match engine {
                    Engine::Codex => ProviderProtocol::Responses,
                    Engine::Claude => ProviderProtocol::AnthropicMessages,
                    Engine::OpenClaw => ProviderProtocol::ChatCompletions,
                },
                workspace: None,
                timeout_seconds: 60,
                api_key: Some(Zeroizing::new(key.to_string())),
            },
        )
        .unwrap()
    }

    #[test]
    fn catalog_keeps_exact_known_capabilities_without_guessing_unknown_models() {
        let known = serde_json::json!({"slug": "known-model", "base_instructions": "original instructions",
            "supported_reasoning_levels": [{"effort": "high"}], "context_window": 123456,
            "input_modalities": ["text", "image"], "model_messages": {"original": true}});
        let entry = codex_catalog_entry("known-model", 0, &[known.clone()]);
        for key in [
            "base_instructions",
            "supported_reasoning_levels",
            "context_window",
            "input_modalities",
            "model_messages",
        ] {
            assert_eq!(entry[key], known[key]);
        }
        let unknown = codex_catalog_entry("DeepSeek/CaseSensitive", 1, &[known]);
        assert_eq!(unknown["slug"], "DeepSeek/CaseSensitive");
        let efforts: Vec<_> = unknown["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level["effort"].as_str().unwrap())
            .collect();
        assert_eq!(efforts, ["low", "medium", "high", "xhigh", "max", "ultra"]);
        assert_eq!(unknown["default_reasoning_level"], "high");
        assert_eq!(unknown["supports_reasoning_summaries"], true);
        assert_eq!(unknown["supports_reasoning_summary_parameter"], false);
        assert_eq!(unknown["input_modalities"], serde_json::json!(["text"]));
        assert_eq!(unknown["context_window"], Value::Null);
        assert_eq!(unknown["base_instructions"], "original instructions");
        let legacy: ProviderModels = serde_json::from_value(serde_json::json!({
            "default": "old", "opus": "old", "sonnet": "old", "haiku": "old", "subagent": "inherit"
        }))
        .unwrap();
        assert!(legacy.available.is_empty());
    }

    #[test]
    fn catalog_removes_only_the_inherited_fixed_model_identity() {
        let instructions =
            "You are Codex, an agent based on GPT-5. Keep all tools and safety instructions.";
        let bundled = [serde_json::json!({"slug":"known", "base_instructions":instructions})];
        for id in ["known", "gpt-6-astra"] {
            assert_eq!(
                codex_catalog_entry(id, 0, &bundled)["base_instructions"],
                "You are Codex, a coding agent. Keep all tools and safety instructions."
            );
        }
    }

    #[test]
    fn codex_discovered_models_are_applied_as_a_catalog() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        configure_test_provider(&paths, Engine::Codex, "test-only-key");
        let path = paths.provider_file("codex");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["models"]["available"] =
            serde_json::json!(["gpt-custom", "DeepSeek/test", "gpt-custom"]);
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        apply(&paths, Engine::Codex).unwrap();
        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        let document = config.parse::<DocumentMut>().unwrap();
        let catalog_path = document
            .get("model_catalog_json")
            .and_then(Item::as_str)
            .expect("saved models must reach Codex's model catalog");
        let catalog: Value = serde_json::from_slice(&fs::read(catalog_path).unwrap()).unwrap();
        let ids: Vec<_> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["model-codex", "gpt-custom", "DeepSeek/test"]);
        assert!(
            !serde_json::to_string(&catalog)
                .unwrap()
                .contains("test-only-key")
        );
    }

    #[test]
    fn three_provider_credentials_are_independent_and_not_public() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        for (engine, key) in [
            (Engine::Codex, "sk-codex-secret"),
            (Engine::Claude, "sk-claude-secret"),
            (Engine::OpenClaw, "sk-openclaw-secret"),
        ] {
            let status = configure_test_provider(&paths, engine, key);
            assert!(status.configured);
            assert!(status.credential_present);
            let public = fs::read_to_string(paths.provider_file(engine.slug())).unwrap();
            assert!(!public.contains(key));
        }
        let vault = PortableVault::open(&paths).unwrap();
        assert_eq!(
            vault.get("provider-codex").unwrap().as_str(),
            "sk-codex-secret"
        );
        assert_eq!(
            vault.get("provider-claude").unwrap().as_str(),
            "sk-claude-secret"
        );
        assert_eq!(
            vault.get("provider-openclaw").unwrap().as_str(),
            "sk-openclaw-secret"
        );
    }

    #[test]
    fn applying_native_provider_configs_sets_chinese_locale_and_direct_routes() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        for engine in Engine::ALL {
            configure_test_provider(&paths, engine, &format!("secret-{}", engine.slug()));
        }

        let codex = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(codex.contains("localeOverride = \"zh-CN\""));
        assert!(codex.contains("js_repl = false"));
        assert!(!codex.contains("apps = false"));
        assert!(codex.contains("https://example.invalid/v1"));
        assert!(codex.contains("YUNSHU_CODEX_API_KEY"));
        assert!(!codex.contains("127.0.0.1:17832"));
        let claude: Value =
            serde_json::from_slice(&fs::read(paths.claude_home.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(claude["language"], "zh-CN");
        assert_eq!(
            claude["env"]["ANTHROPIC_BASE_URL"],
            "https://example.invalid"
        );
        let openclaw: Value =
            serde_json::from_slice(&fs::read(paths.openclaw_home.join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(
            openclaw["models"]["providers"]["yunshu-openclaw"]["baseUrl"],
            "https://example.invalid/v1"
        );
        assert_eq!(
            openclaw["models"]["providers"]["yunshu-openclaw"]["apiKey"],
            "${YUNSHU_OPENCLAW_API_KEY}"
        );
        assert_eq!(
            openclaw["models"]["providers"]["yunshu-openclaw"]["timeoutSeconds"],
            180
        );
        assert_eq!(openclaw["agents"]["defaults"]["timeoutSeconds"], 180);
        for engine in Engine::ALL {
            assert!(!requires_bridge(&paths, engine).unwrap());
            assert!(status(&paths, engine).unwrap().applied);
        }
    }

    #[test]
    fn cross_protocol_codex_and_claude_use_bridge_only_when_required() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();

        for (engine, protocol) in [
            (Engine::Codex, ProviderProtocol::ChatCompletions),
            (Engine::Claude, ProviderProtocol::Responses),
            (Engine::OpenClaw, ProviderProtocol::Responses),
        ] {
            configure(
                &paths,
                ConfigureProvider {
                    engine,
                    display_name: format!("{engine}跨协议测试"),
                    base_url: "https://example.invalid/v1".to_string(),
                    model: "test-model".to_string(),
                    models: None,
                    protocol,
                    workspace: None,
                    timeout_seconds: 60,
                    api_key: Some(Zeroizing::new(format!("secret-{}", engine.slug()))),
                },
            )
            .unwrap();
        }

        assert!(requires_bridge(&paths, Engine::Codex).unwrap());
        assert!(requires_bridge(&paths, Engine::Claude).unwrap());
        assert!(!requires_bridge(&paths, Engine::OpenClaw).unwrap());
        let codex = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(codex.contains("127.0.0.1:17832/codex/v1"));
        assert!(codex.contains("YUNSHU_CODEX_LOCAL_TOKEN"));
        let claude: Value =
            serde_json::from_slice(&fs::read(paths.claude_home.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            claude["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:17832/claude"
        );
        let openclaw: Value =
            serde_json::from_slice(&fs::read(paths.openclaw_home.join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(
            openclaw["models"]["providers"]["yunshu-openclaw"]["baseUrl"],
            "https://example.invalid/v1"
        );
    }

    #[test]
    fn launch_environment_uses_upstream_or_local_token_for_selected_route() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        configure_test_provider(&paths, Engine::Codex, "direct-secret");

        let direct = launch_environment(&paths, Engine::Codex).unwrap();
        assert!(
            direct
                .iter()
                .any(|(key, value)| { key == "YUNSHU_CODEX_API_KEY" && value == "direct-secret" })
        );

        configure(
            &paths,
            ConfigureProvider {
                engine: Engine::Codex,
                display_name: "Codex桥接测试".to_string(),
                base_url: "https://example.invalid/v1".to_string(),
                model: "test-model".to_string(),
                models: None,
                protocol: ProviderProtocol::ChatCompletions,
                workspace: None,
                timeout_seconds: 60,
                api_key: None,
            },
        )
        .unwrap();
        let bridged = launch_environment(&paths, Engine::Codex).unwrap();
        assert!(
            bridged.iter().any(|(key, value)| {
                key == "YUNSHU_CODEX_LOCAL_TOKEN" && value != "direct-secret"
            })
        );
    }

    #[test]
    fn changing_one_provider_does_not_rewrite_other_public_configs() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        for engine in Engine::ALL {
            configure_test_provider(&paths, engine, &format!("secret-{}", engine.slug()));
        }
        let claude_before = fs::read(paths.provider_file("claude")).unwrap();
        let openclaw_before = fs::read(paths.provider_file("openclaw")).unwrap();

        configure(
            &paths,
            ConfigureProvider {
                engine: Engine::Codex,
                display_name: "新的Codex供应商".to_string(),
                base_url: "https://new.example.invalid/v1".to_string(),
                model: "new-codex-model".to_string(),
                models: None,
                protocol: ProviderProtocol::ChatCompletions,
                workspace: None,
                timeout_seconds: 90,
                api_key: Some(Zeroizing::new("new-codex-secret".to_string())),
            },
        )
        .unwrap();

        assert_eq!(
            fs::read(paths.provider_file("claude")).unwrap(),
            claude_before
        );
        assert_eq!(
            fs::read(paths.provider_file("openclaw")).unwrap(),
            openclaw_before
        );
        let vault = PortableVault::open(&paths).unwrap();
        assert_eq!(
            vault.get("provider-claude").unwrap().as_str(),
            "secret-claude"
        );
        assert_eq!(
            vault.get("provider-openclaw").unwrap().as_str(),
            "secret-openclaw"
        );
    }

    #[test]
    fn schema_v2_migrates_to_v3_with_backup_and_model_roles() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        let legacy = serde_json::json!({
            "schemaVersion": 2,
            "engine": "claude",
            "displayName": "旧供应商",
            "baseUrl": "https://example.invalid/v1",
            "model": "legacy-sonnet",
            "protocol": "anthropic-messages",
            "credentialRef": "provider-claude",
            "localTokenRef": "local-claude",
            "timeoutSeconds": 60,
            "enabled": true
        });
        fs::write(
            paths.provider_file("claude"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let migrated = load(&paths, Engine::Claude).unwrap();

        assert_eq!(migrated.schema_version, 3);
        assert_eq!(migrated.models.default, "legacy-sonnet");
        assert_eq!(migrated.models.opus, "legacy-sonnet");
        assert_eq!(migrated.models.subagent, "inherit");
        assert_eq!(
            migrated.resolved_protocol,
            ProviderProtocol::AnthropicMessages
        );
        assert!(
            paths
                .backups
                .join("providers")
                .join("claude-schema-v2.json")
                .is_file()
        );
        let public: Value =
            serde_json::from_slice(&fs::read(paths.provider_file("claude")).unwrap()).unwrap();
        assert_eq!(public["schemaVersion"], 3);
        assert_eq!(public["models"]["default"], "legacy-sonnet");
    }

    #[test]
    fn claude_model_roles_and_portable_workspace_are_applied_independently() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        configure(
            &paths,
            ConfigureProvider {
                engine: Engine::Claude,
                display_name: "Claude测试供应商".to_string(),
                base_url: "https://example.invalid/v1".to_string(),
                model: "default-model".to_string(),
                models: Some(ProviderModels {
                    default: "default-model".to_string(),
                    opus: "opus-model".to_string(),
                    sonnet: "sonnet-model".to_string(),
                    haiku: "haiku-model".to_string(),
                    subagent: "subagent-model".to_string(),
                    available: Vec::new(),
                }),
                protocol: ProviderProtocol::Auto,
                workspace: Some(ProviderWorkspace {
                    choose_on_launch: true,
                    relative_path: "workspace/Claude 项目".to_string(),
                }),
                timeout_seconds: 60,
                api_key: Some(Zeroizing::new("secret".to_string())),
            },
        )
        .unwrap();

        let settings: Value =
            serde_json::from_slice(&fs::read(paths.claude_home.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "default-model");
        assert_eq!(
            settings["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            "opus-model"
        );
        assert_eq!(
            settings["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            "sonnet-model"
        );
        assert_eq!(
            settings["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "haiku-model"
        );
        assert_eq!(
            settings["env"]["CLAUDE_CODE_SUBAGENT_MODEL"],
            "subagent-model"
        );
        assert_eq!(
            workspace_for_launch(&paths, Engine::Claude).unwrap(),
            Some(paths.root.join("workspace").join("Claude 项目"))
        );
    }

    #[test]
    fn model_discovery_parser_accepts_openai_and_alternate_lists() {
        assert_eq!(
            extract_model_ids(&serde_json::json!({
                "data": [{"id": "model-a"}, {"id": "model-b"}]
            })),
            vec!["model-a".to_string(), "model-b".to_string()]
        );
        assert_eq!(
            extract_model_ids(&serde_json::json!({
                "models": [{"name": "model-c"}]
            })),
            vec!["model-c".to_string()]
        );
    }
}
