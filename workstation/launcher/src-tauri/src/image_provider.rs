use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use url::Url;
use uuid::Uuid;
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::vault::PortableVault;
use yunshu_workstation_runtime::windows_process::replace_file_atomic;
use zeroize::Zeroizing;

use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};

const IMAGE_PROVIDER_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderConfig {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    pub credential_ref: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_streaming")]
    pub streaming: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageProviderStore {
    schema_version: u32,
    active_provider_id: Option<String>,
    providers: Vec<ImageProviderConfig>,
}

impl Default for ImageProviderStore {
    fn default() -> Self {
        Self {
            schema_version: IMAGE_PROVIDER_SCHEMA_VERSION,
            active_provider_id: None,
            providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderView {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub credential_present: bool,
    pub timeout_seconds: u64,
    pub streaming: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderState {
    pub configured: bool,
    pub active_provider_id: Option<String>,
    pub providers: Vec<ImageProviderView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderSaveInput {
    pub id: Option<String>,
    pub display_name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    pub timeout_seconds: Option<u64>,
    pub streaming: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderReferenceInput {
    pub provider_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderDiscoverInput {
    pub provider_id: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderModelsResult {
    pub models: Vec<String>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProviderProbeResult {
    pub success: bool,
    pub status_code: Option<u16>,
    pub message: String,
    pub elapsed_ms: u128,
}

#[tauri::command]
pub fn image_provider_list(state: State<'_, AppState>) -> CommandResult<ImageProviderState> {
    provider_state(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn image_provider_save(
    state: State<'_, AppState>,
    input: ImageProviderSaveInput,
) -> CommandResult<ImageProviderState> {
    save_provider(&state.paths, input).map_err(CommandError::from)
}

#[tauri::command]
pub fn image_provider_set_active(
    state: State<'_, AppState>,
    input: ImageProviderReferenceInput,
) -> CommandResult<ImageProviderState> {
    set_active_provider(&state.paths, &input.provider_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn image_provider_discover_models(
    state: State<'_, AppState>,
    input: ImageProviderDiscoverInput,
) -> CommandResult<ImageProviderModelsResult> {
    discover_models(&state.paths, input)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn image_provider_test(
    state: State<'_, AppState>,
    input: ImageProviderReferenceInput,
) -> CommandResult<ImageProviderProbeResult> {
    probe_provider(&state.paths, &input.provider_id)
        .await
        .map_err(CommandError::from)
}

pub fn provider_state(paths: &WorkstationPaths) -> Result<ImageProviderState> {
    let store = load_store(paths)?;
    let vault = PortableVault::open(paths)?;
    let providers = store
        .providers
        .iter()
        .map(|provider| ImageProviderView {
            id: provider.id.clone(),
            display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            default_model: provider.default_model.clone(),
            models: provider.models.clone(),
            credential_present: vault.contains(&provider.credential_ref),
            timeout_seconds: provider.timeout_seconds,
            streaming: provider.streaming,
            enabled: provider.enabled,
        })
        .collect::<Vec<_>>();
    let configured = providers
        .iter()
        .any(|provider| provider.enabled && provider.credential_present);
    Ok(ImageProviderState {
        configured,
        active_provider_id: store.active_provider_id,
        providers,
    })
}

pub fn load_provider(paths: &WorkstationPaths, provider_id: &str) -> Result<ImageProviderConfig> {
    let store = load_store(paths)?;
    let selected_id = if provider_id.trim().is_empty() {
        store
            .active_provider_id
            .as_deref()
            .ok_or_else(|| anyhow!("请先配置 AI 生图专用供应商"))?
    } else {
        provider_id.trim()
    };
    store
        .providers
        .into_iter()
        .find(|provider| provider.id == selected_id && provider.enabled)
        .ok_or_else(|| anyhow!("没有找到该 AI 生图供应商"))
}

pub fn provider_secret(
    paths: &WorkstationPaths,
    config: &ImageProviderConfig,
) -> Result<Zeroizing<String>> {
    PortableVault::open(paths)?.get(&config.credential_ref)
}

fn save_provider(
    paths: &WorkstationPaths,
    input: ImageProviderSaveInput,
) -> Result<ImageProviderState> {
    let display_name = input.display_name.trim();
    let default_model = input.default_model.trim();
    if display_name.is_empty() {
        bail!("供应商名称不能为空");
    }
    if default_model.is_empty() {
        bail!("图片模型不能为空");
    }
    let base_url = normalize_base_url(&input.base_url)?;
    let timeout_seconds = input.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if !(30..=900).contains(&timeout_seconds) {
        bail!("超时时间必须在30到900秒之间");
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
        .unwrap_or_else(|| format!("provider-image-{provider_id}"));
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
    let config = ImageProviderConfig {
        id: provider_id.clone(),
        display_name: display_name.to_string(),
        base_url,
        default_model: default_model.to_string(),
        models,
        credential_ref,
        timeout_seconds,
        streaming: input.streaming.unwrap_or(false),
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

fn set_active_provider(paths: &WorkstationPaths, provider_id: &str) -> Result<ImageProviderState> {
    let mut store = load_store(paths)?;
    if !store
        .providers
        .iter()
        .any(|provider| provider.id == provider_id && provider.enabled)
    {
        bail!("没有找到可切换的 AI 生图供应商");
    }
    store.active_provider_id = Some(provider_id.to_string());
    save_store(paths, &store)?;
    provider_state(paths)
}

async fn discover_models(
    paths: &WorkstationPaths,
    input: ImageProviderDiscoverInput,
) -> Result<ImageProviderModelsResult> {
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
    let api_key = resolve_secret(paths, configured.as_ref(), input.api_key)?;
    let timeout_seconds = input.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建图片模型发现客户端")?;
    let started = Instant::now();
    let response = client
        .get(endpoint)
        .bearer_auth(api_key.as_str())
        .send()
        .await
        .context("无法连接供应商模型接口")?;
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
    Ok(ImageProviderModelsResult {
        models,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

async fn probe_provider(
    paths: &WorkstationPaths,
    provider_id: &str,
) -> Result<ImageProviderProbeResult> {
    let config = load_provider(paths, provider_id)?;
    let secret = provider_secret(paths, &config)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(config.timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建图片供应商测试客户端")?;
    let started = Instant::now();
    let response = client
        .get(models_endpoint(&config.base_url)?)
        .header(header::ACCEPT, "application/json")
        .bearer_auth(secret.as_str())
        .send()
        .await
        .context("无法连接图片供应商")?;
    let status = response.status();
    Ok(ImageProviderProbeResult {
        success: status.is_success(),
        status_code: Some(status.as_u16()),
        message: if status.is_success() {
            "鉴权和模型接口连接正常；生图接口会在首次真实生成时验证。".to_string()
        } else {
            format!("供应商模型接口返回 HTTP {}", status.as_u16())
        },
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn normalize_base_url(input: &str) -> Result<String> {
    let mut url = Url::parse(input.trim()).context("供应商 Base URL 格式不正确")?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("供应商 Base URL 不能包含账号或密码");
    }
    let allowed = url.scheme() == "https"
        || (url.scheme() == "http"
            && url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<IpAddr>()
                        .ok()
                        .is_some_and(|address| address.is_loopback())
            }));
    if !allowed || url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        bail!("供应商 Base URL 只允许 HTTPS，或本机 loopback HTTP 地址");
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn models_endpoint(base_url: &str) -> Result<Url> {
    endpoint_with_suffix(base_url, "models")
}

pub fn endpoint_with_suffix(base_url: &str, suffix: &str) -> Result<Url> {
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
    configured: Option<&ImageProviderConfig>,
    input: Option<String>,
) -> Result<Zeroizing<String>> {
    if let Some(value) = input.filter(|value| !value.trim().is_empty()) {
        return Ok(Zeroizing::new(value.trim().to_string()));
    }
    let provider = configured.ok_or_else(|| anyhow!("请填写 API Key 或先保存供应商"))?;
    provider_secret(paths, provider)
}

fn store_path(paths: &WorkstationPaths) -> PathBuf {
    paths
        .data
        .join("integrations")
        .join("image-studio")
        .join("config")
        .join("providers.json")
}

fn load_store(paths: &WorkstationPaths) -> Result<ImageProviderStore> {
    let path = store_path(paths);
    if !path.is_file() {
        return Ok(ImageProviderStore::default());
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("无法读取 AI 生图供应商配置 {}", path.display()))?;
    let store: ImageProviderStore = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析 AI 生图供应商配置 {}", path.display()))?;
    if store.schema_version != IMAGE_PROVIDER_SCHEMA_VERSION {
        bail!("AI 生图供应商配置版本不兼容");
    }
    Ok(store)
}

fn save_store(paths: &WorkstationPaths, store: &ImageProviderStore) -> Result<()> {
    let path = store_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建 AI 生图配置目录 {}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(store)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入 AI 生图供应商临时配置 {}", temporary.display()))?;
    replace_file_atomic(&temporary, &path)
}

const fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

const fn default_streaming() -> bool {
    false
}

const fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_config_is_portable_and_does_not_expose_secret() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let state = save_provider(
            &paths,
            ImageProviderSaveInput {
                id: None,
                display_name: "图片供应商".to_string(),
                base_url: "https://example.com/v1".to_string(),
                api_key: Some("sk-image-secret".to_string()),
                default_model: "gpt-image-test".to_string(),
                models: vec!["gpt-image-test".to_string()],
                timeout_seconds: None,
                streaming: Some(true),
            },
        )
        .unwrap();
        assert!(state.configured);
        let bytes = fs::read(store_path(&paths)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-image-secret"));
        assert!(store_path(&paths).starts_with(&paths.data));
    }

    #[test]
    fn endpoints_preserve_existing_v1_prefix() {
        assert_eq!(
            endpoint_with_suffix("https://example.com/v1", "images/generations")
                .unwrap()
                .as_str(),
            "https://example.com/v1/images/generations"
        );
    }

    #[test]
    fn provider_rejects_unsafe_base_urls() {
        assert!(normalize_base_url("http://example.com/v1").is_err());
        assert!(normalize_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(normalize_base_url("https://user:pass@example.com/v1").is_err());
    }
}
