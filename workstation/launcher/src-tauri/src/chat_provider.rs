use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use url::Url;
use uuid::Uuid;
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::provider::ProviderProtocol;
use yunshu_workstation_runtime::vault::PortableVault;
use yunshu_workstation_runtime::windows_process::replace_file_atomic;
use zeroize::Zeroizing;

use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};

const CHAT_PROVIDER_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_SECONDS: u64 = 180;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMAGE_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderConfig {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    pub protocol: ProviderProtocol,
    pub credential_ref: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatProviderStore {
    schema_version: u32,
    active_provider_id: Option<String>,
    providers: Vec<ChatProviderConfig>,
}

impl Default for ChatProviderStore {
    fn default() -> Self {
        Self {
            schema_version: CHAT_PROVIDER_SCHEMA_VERSION,
            active_provider_id: None,
            providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderView {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub protocol: ProviderProtocol,
    pub credential_present: bool,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderState {
    pub configured: bool,
    pub active_provider_id: Option<String>,
    pub providers: Vec<ChatProviderView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderSaveInput {
    pub id: Option<String>,
    pub display_name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    pub protocol: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderReferenceInput {
    pub provider_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderDiscoverInput {
    pub provider_id: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub protocol: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatProviderModelsResult {
    pub models: Vec<String>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone)]
pub struct ChatRequestAttachment {
    pub file_name: String,
    pub media_type: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ChatRequestMessage {
    pub role: String,
    pub content: String,
    pub attachments: Vec<ChatRequestAttachment>,
}

#[tauri::command]
pub fn chat_provider_list(state: State<'_, AppState>) -> CommandResult<ChatProviderState> {
    provider_state(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_provider_save(
    state: State<'_, AppState>,
    input: ChatProviderSaveInput,
) -> CommandResult<ChatProviderState> {
    save_provider(&state.paths, input).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_provider_set_active(
    state: State<'_, AppState>,
    input: ChatProviderReferenceInput,
) -> CommandResult<ChatProviderState> {
    set_active_provider(&state.paths, &input.provider_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn chat_provider_discover_models(
    state: State<'_, AppState>,
    input: ChatProviderDiscoverInput,
) -> CommandResult<ChatProviderModelsResult> {
    discover_models(&state.paths, input)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn chat_provider_test(
    state: State<'_, AppState>,
    input: ChatProviderReferenceInput,
) -> CommandResult<()> {
    let config = load_provider(&state.paths, &input.provider_id).map_err(CommandError::from)?;
    request_provider_text(
        &state.paths,
        &config,
        &[ChatRequestMessage {
            role: "user".to_string(),
            content: "请只回复 OK".to_string(),
            attachments: Vec::new(),
        }],
    )
    .await
    .map(|_| ())
    .map_err(CommandError::from)
}

pub fn provider_state(paths: &WorkstationPaths) -> Result<ChatProviderState> {
    let store = load_store(paths)?;
    let vault = PortableVault::open(paths)?;
    let providers = store
        .providers
        .iter()
        .map(|provider| ChatProviderView {
            id: provider.id.clone(),
            display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            default_model: provider.default_model.clone(),
            models: provider.models.clone(),
            protocol: provider.protocol,
            credential_present: vault.contains(&provider.credential_ref),
            timeout_seconds: provider.timeout_seconds,
            enabled: provider.enabled,
        })
        .collect::<Vec<_>>();
    let configured = providers
        .iter()
        .any(|provider| provider.enabled && provider.credential_present);
    Ok(ChatProviderState {
        configured,
        active_provider_id: store.active_provider_id,
        providers,
    })
}

pub fn load_active_provider(paths: &WorkstationPaths) -> Result<ChatProviderConfig> {
    let store = load_store(paths)?;
    let active_id = store
        .active_provider_id
        .as_deref()
        .ok_or_else(|| anyhow!("请先配置 AI 对话专用供应商"))?;
    store
        .providers
        .into_iter()
        .find(|provider| provider.id == active_id && provider.enabled)
        .ok_or_else(|| anyhow!("AI 对话当前供应商不存在或已停用"))
}

pub fn load_provider(paths: &WorkstationPaths, provider_id: &str) -> Result<ChatProviderConfig> {
    load_store(paths)?
        .providers
        .into_iter()
        .find(|provider| provider.id == provider_id && provider.enabled)
        .ok_or_else(|| anyhow!("没有找到该 AI 对话供应商"))
}

fn save_provider(
    paths: &WorkstationPaths,
    input: ChatProviderSaveInput,
) -> Result<ChatProviderState> {
    let display_name = input.display_name.trim();
    let default_model = input.default_model.trim();
    if display_name.is_empty() {
        bail!("供应商名称不能为空");
    }
    if default_model.is_empty() {
        bail!("默认模型不能为空");
    }
    let base_url = normalize_base_url(&input.base_url)?;
    let protocol = ProviderProtocol::from_str(&input.protocol)?;
    let timeout_seconds = input.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(5..=600).contains(&timeout_seconds) {
        bail!("超时时间必须在5到600秒之间");
    }

    let mut store = load_store(paths)?;
    let provider_id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing = store
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned();
    let credential_ref = existing
        .as_ref()
        .map(|provider| provider.credential_ref.clone())
        .unwrap_or_else(|| format!("provider-chat-{provider_id}"));
    let mut vault = PortableVault::open(paths)?;
    match input.api_key {
        Some(api_key) if !api_key.trim().is_empty() => {
            vault.set(
                credential_ref.clone(),
                Zeroizing::new(api_key.trim().to_string()),
            )?;
            vault.save(paths)?;
        }
        _ if !vault.contains(&credential_ref) => bail!("首次保存供应商时必须填写 API Key"),
        _ => {}
    }

    let mut models = input
        .models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if !models.iter().any(|model| model == default_model) {
        models.insert(0, default_model.to_string());
    }
    models.sort();
    models.dedup();
    let config = ChatProviderConfig {
        id: provider_id.clone(),
        display_name: display_name.to_string(),
        base_url,
        default_model: default_model.to_string(),
        models,
        protocol,
        credential_ref,
        timeout_seconds,
        enabled: true,
    };
    if let Some(index) = store
        .providers
        .iter()
        .position(|provider| provider.id == provider_id)
    {
        store.providers[index] = config;
    } else {
        store.providers.push(config);
    }
    store.active_provider_id = Some(provider_id);
    save_store(paths, &store)?;
    provider_state(paths)
}

#[cfg(test)]
pub(crate) fn save_provider_for_test(
    paths: &WorkstationPaths,
    input: ChatProviderSaveInput,
) -> Result<ChatProviderState> {
    save_provider(paths, input)
}

fn set_active_provider(paths: &WorkstationPaths, provider_id: &str) -> Result<ChatProviderState> {
    let mut store = load_store(paths)?;
    if !store
        .providers
        .iter()
        .any(|provider| provider.id == provider_id && provider.enabled)
    {
        bail!("没有找到可切换的 AI 对话供应商");
    }
    store.active_provider_id = Some(provider_id.to_string());
    save_store(paths, &store)?;
    provider_state(paths)
}

async fn discover_models(
    paths: &WorkstationPaths,
    input: ChatProviderDiscoverInput,
) -> Result<ChatProviderModelsResult> {
    let configured = input
        .provider_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .and_then(|provider_id| load_provider(paths, provider_id).ok());
    let base_url = if input.base_url.trim().is_empty() {
        configured
            .as_ref()
            .map(|provider| provider.base_url.clone())
            .ok_or_else(|| anyhow!("请先填写供应商 API 地址"))?
    } else {
        normalize_base_url(&input.base_url)?
    };
    let protocol = ProviderProtocol::from_str(&input.protocol)?;
    let api_key = resolve_secret(paths, configured.as_ref(), input.api_key)?;
    let timeout_seconds = input.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(5..=600).contains(&timeout_seconds) {
        bail!("超时时间必须在5到600秒之间");
    }
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建模型发现客户端")?;
    let mut request = client.get(endpoint);
    request = if resolved_protocol(protocol) == ProviderProtocol::AnthropicMessages {
        request
            .header("x-api-key", api_key.as_str())
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(api_key.as_str())
    };
    let started = Instant::now();
    let response = request.send().await.context("无法连接供应商模型接口")?;
    let status = response.status();
    let bytes = response.bytes().await.context("无法读取模型接口响应")?;
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
    Ok(ChatProviderModelsResult {
        models,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub async fn request_provider_text(
    paths: &WorkstationPaths,
    config: &ChatProviderConfig,
    messages: &[ChatRequestMessage],
) -> Result<String> {
    if !config.enabled {
        bail!("AI 对话当前供应商已停用");
    }
    let api_key = PortableVault::open(paths)?.get(&config.credential_ref)?;
    let protocol = resolved_protocol(config.protocol);
    let (endpoint, body) = build_request(config, protocol, messages)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(config.timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建 AI 对话客户端")?;
    let mut request = client
        .post(endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .json(&body);
    request = if protocol == ProviderProtocol::AnthropicMessages {
        request
            .header("x-api-key", api_key.as_str())
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(api_key.as_str())
    };
    let response = request.send().await.context("无法连接 AI 对话供应商")?;
    let status = response.status();
    let bytes = response.bytes().await.context("无法读取 AI 对话响应")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        bail!("AI 对话响应超过 8MB 限制");
    }
    if !status.is_success() {
        bail!("AI 对话供应商返回 HTTP {}", status.as_u16());
    }
    let body: Value = serde_json::from_slice(&bytes).context("AI 对话供应商返回了无效 JSON")?;
    extract_assistant_text(protocol, &body)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("AI 对话供应商没有返回可显示的文本内容"))
}

fn build_request(
    config: &ChatProviderConfig,
    protocol: ProviderProtocol,
    messages: &[ChatRequestMessage],
) -> Result<(Url, Value)> {
    let endpoint = completion_endpoint(&config.base_url, protocol)?;
    let body = match protocol {
        ProviderProtocol::Responses => json!({
            "model": config.default_model,
            "input": messages.iter().map(response_message).collect::<Result<Vec<_>>>()?,
            "max_output_tokens": 4096,
            "stream": false
        }),
        ProviderProtocol::ChatCompletions => json!({
            "model": config.default_model,
            "messages": messages.iter().map(chat_completion_message).collect::<Result<Vec<_>>>()?,
            "max_tokens": 4096,
            "stream": false
        }),
        ProviderProtocol::AnthropicMessages => json!({
            "model": config.default_model,
            "messages": messages.iter().map(anthropic_message).collect::<Result<Vec<_>>>()?,
            "max_tokens": 4096,
            "stream": false
        }),
        ProviderProtocol::Auto => unreachable!("协议已解析"),
    };
    Ok((endpoint, body))
}

fn response_message(message: &ChatRequestMessage) -> Result<Value> {
    let mut content = Vec::new();
    if !message.content.trim().is_empty() {
        content.push(json!({"type": "input_text", "text": message.content}));
    }
    for attachment in &message.attachments {
        if attachment.media_type.starts_with("image/") {
            content.push(json!({"type": "input_image", "image_url": image_data_url(attachment)?}));
        } else {
            content.push(json!({"type": "input_text", "text": text_attachment(attachment)?}));
        }
    }
    Ok(json!({"role": message.role, "content": content}))
}

fn chat_completion_message(message: &ChatRequestMessage) -> Result<Value> {
    if message.attachments.is_empty() {
        return Ok(json!({"role": message.role, "content": message.content}));
    }
    let mut content = Vec::new();
    if !message.content.trim().is_empty() {
        content.push(json!({"type": "text", "text": message.content}));
    }
    for attachment in &message.attachments {
        if attachment.media_type.starts_with("image/") {
            content.push(
                json!({"type": "image_url", "image_url": {"url": image_data_url(attachment)?}}),
            );
        } else {
            content.push(json!({"type": "text", "text": text_attachment(attachment)?}));
        }
    }
    Ok(json!({"role": message.role, "content": content}))
}

fn anthropic_message(message: &ChatRequestMessage) -> Result<Value> {
    let mut content = Vec::new();
    if !message.content.trim().is_empty() {
        content.push(json!({"type": "text", "text": message.content}));
    }
    for attachment in &message.attachments {
        if attachment.media_type.starts_with("image/") {
            let bytes = read_attachment(attachment, MAX_IMAGE_ATTACHMENT_BYTES)?;
            content.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": attachment.media_type,
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes)
                }
            }));
        } else {
            content.push(json!({"type": "text", "text": text_attachment(attachment)?}));
        }
    }
    Ok(json!({"role": message.role, "content": content}))
}

fn image_data_url(attachment: &ChatRequestAttachment) -> Result<String> {
    let bytes = read_attachment(attachment, MAX_IMAGE_ATTACHMENT_BYTES)?;
    Ok(format!(
        "data:{};base64,{}",
        attachment.media_type,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn text_attachment(attachment: &ChatRequestAttachment) -> Result<String> {
    let bytes = read_attachment(attachment, MAX_TEXT_ATTACHMENT_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("{} 不是 UTF-8 文本文件", attachment.file_name))?;
    Ok(format!("[附件：{}]\n{}", attachment.file_name, text))
}

fn read_attachment(attachment: &ChatRequestAttachment, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(&attachment.path)
        .with_context(|| format!("无法读取附件 {}", attachment.file_name))?;
    if metadata.len() > limit {
        bail!("{} 超过允许的大小限制", attachment.file_name);
    }
    fs::read(&attachment.path).with_context(|| format!("无法读取附件 {}", attachment.file_name))
}

fn extract_assistant_text(protocol: ProviderProtocol, body: &Value) -> Option<String> {
    match protocol {
        ProviderProtocol::Responses | ProviderProtocol::Auto => {
            if let Some(text) = body.get("output_text").and_then(Value::as_str) {
                return Some(text.to_string());
            }
            let pieces = body
                .get("output")?
                .as_array()?
                .iter()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|block| {
                    block
                        .get("text")
                        .or_else(|| block.get("output_text"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>();
            (!pieces.is_empty()).then(|| pieces.join("\n"))
        }
        ProviderProtocol::ChatCompletions => body
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_string),
        ProviderProtocol::AnthropicMessages => {
            let pieces = body
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>();
            (!pieces.is_empty()).then(|| pieces.join("\n"))
        }
    }
}

fn resolved_protocol(protocol: ProviderProtocol) -> ProviderProtocol {
    match protocol {
        ProviderProtocol::Auto => ProviderProtocol::ChatCompletions,
        protocol => protocol,
    }
}

fn completion_endpoint(base_url: &str, protocol: ProviderProtocol) -> Result<Url> {
    let suffix = match protocol {
        ProviderProtocol::Responses => "responses",
        ProviderProtocol::ChatCompletions => "chat/completions",
        ProviderProtocol::AnthropicMessages => "messages",
        ProviderProtocol::Auto => unreachable!("协议已解析"),
    };
    endpoint_with_suffix(base_url, suffix)
}

fn models_endpoint(base_url: &str) -> Result<Url> {
    endpoint_with_suffix(base_url, "models")
}

fn endpoint_with_suffix(base_url: &str, suffix: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).context("供应商 Base URL 格式不正确")?;
    let path = url.path().trim_end_matches('/');
    let next = if path.ends_with("/v1") {
        format!("{path}/{suffix}")
    } else {
        format!("{path}/v1/{suffix}")
    };
    url.set_path(&next);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn normalize_base_url(input: &str) -> Result<String> {
    let mut url = Url::parse(input.trim()).context("供应商 Base URL 格式不正确")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("供应商 Base URL 只允许 http 或 https");
    }
    if url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        bail!("供应商 Base URL 必须包含主机，且不能带查询参数或片段");
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn extract_model_ids(value: &Value) -> Vec<String> {
    [
        value.get("data").and_then(Value::as_array),
        value.get("models").and_then(Value::as_array),
    ]
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

fn resolve_secret(
    paths: &WorkstationPaths,
    configured: Option<&ChatProviderConfig>,
    input: Option<String>,
) -> Result<Zeroizing<String>> {
    if let Some(value) = input.filter(|value| !value.trim().is_empty()) {
        return Ok(Zeroizing::new(value.trim().to_string()));
    }
    let provider = configured.ok_or_else(|| anyhow!("请填写 API Key 或先保存供应商"))?;
    PortableVault::open(paths)?.get(&provider.credential_ref)
}

fn store_path(paths: &WorkstationPaths) -> PathBuf {
    paths
        .data
        .join("chat")
        .join("config")
        .join("providers.json")
}

fn load_store(paths: &WorkstationPaths) -> Result<ChatProviderStore> {
    let path = store_path(paths);
    if !path.is_file() {
        return Ok(ChatProviderStore::default());
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("无法读取 AI 对话供应商配置 {}", path.display()))?;
    let store: ChatProviderStore = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析 AI 对话供应商配置 {}", path.display()))?;
    if store.schema_version != CHAT_PROVIDER_SCHEMA_VERSION {
        bail!("AI 对话供应商配置版本不兼容");
    }
    Ok(store)
}

fn save_store(paths: &WorkstationPaths, store: &ChatProviderStore) -> Result<()> {
    let path = store_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建 AI 对话配置目录 {}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(store)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入 AI 对话供应商临时配置 {}", temporary.display()))?;
    replace_file_atomic(&temporary, &path)
}

const fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

const fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_provider_is_portable_and_secret_is_not_public() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let state = save_provider(
            &paths,
            ChatProviderSaveInput {
                id: None,
                display_name: "测试供应商".to_string(),
                base_url: "https://example.com/v1".to_string(),
                api_key: Some("sk-chat-secret".to_string()),
                default_model: "model-a".to_string(),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                protocol: "chat-completions".to_string(),
                timeout_seconds: Some(30),
            },
        )
        .unwrap();
        assert!(state.configured);
        let bytes = fs::read(store_path(&paths)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-chat-secret"));
        assert!(store_path(&paths).starts_with(&paths.data));
    }

    #[test]
    fn endpoints_preserve_existing_v1_prefix() {
        assert_eq!(
            completion_endpoint("https://example.com/v1", ProviderProtocol::Responses)
                .unwrap()
                .as_str(),
            "https://example.com/v1/responses"
        );
        assert_eq!(
            models_endpoint("https://example.com/api").unwrap().as_str(),
            "https://example.com/api/v1/models"
        );
    }
}
