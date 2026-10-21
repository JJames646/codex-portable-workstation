use std::collections::HashMap;
use std::fs;
use std::io::{self, Cursor, Write as _};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{DynamicImage, GenericImageView, ImageFormat};
use reqwest::Client;
use reqwest::multipart::{Form, Part};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;
use yunshu_workstation_runtime::paths::WorkstationPaths;

use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};
use crate::image_provider::{
    ImageProviderConfig, ImageProviderState, endpoint_with_suffix, load_provider, provider_secret,
    provider_state,
};
use crate::imagegen_repository::{ImageAssetRecord, ImageJob, ImageRepository, NewImageJob};

const MAX_PROMPT_CHARS: usize = 8_000;
const MAX_REFERENCE_BYTES: usize = 20 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const MAX_HISTORY_JOBS: usize = 80;

#[derive(Default)]
pub struct ImageGenerationRuntime {
    requests: Mutex<HashMap<String, watch::Sender<bool>>>,
    recovered: Mutex<bool>,
}

impl ImageGenerationRuntime {
    fn start(&self, job_id: &str) -> Result<watch::Receiver<bool>> {
        let (sender, receiver) = watch::channel(false);
        let mut requests = self
            .requests
            .lock()
            .map_err(|_| anyhow!("AI 生图任务状态异常，请重启工作站后重试"))?;
        if requests.contains_key(job_id) {
            bail!("AI 生图任务已经在运行");
        }
        requests.insert(job_id.to_string(), sender);
        Ok(receiver)
    }

    fn finish(&self, job_id: &str) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.remove(job_id);
        }
    }

    fn cancel(&self, job_id: &str) -> bool {
        self.requests
            .lock()
            .ok()
            .and_then(|requests| requests.get(job_id).cloned())
            .is_some_and(|sender| sender.send(true).is_ok())
    }

    pub(crate) fn cancel_all(&self) -> usize {
        let senders = self
            .requests
            .lock()
            .map(|requests| requests.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for sender in &senders {
            let _ = sender.send(true);
        }
        senders.len()
    }

    pub(crate) fn active_count(&self) -> usize {
        self.requests
            .lock()
            .map(|requests| requests.len())
            .unwrap_or(0)
    }

    fn recover_once(&self, repository: &ImageRepository) -> Result<()> {
        let mut recovered = self
            .recovered
            .lock()
            .map_err(|_| anyhow!("AI 生图恢复状态异常"))?;
        if !*recovered {
            repository.recover_interrupted_jobs()?;
            *recovered = true;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageJobDto {
    #[serde(flatten)]
    pub job: ImageJob,
    pub asset_previews: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationBootstrap {
    pub database_path: String,
    pub data_dir: String,
    pub provider_state: ImageProviderState,
    pub jobs: Vec<ImageJobDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerateInput {
    pub provider_id: String,
    pub mode: String,
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    pub size: String,
    pub quality: String,
    pub count: u8,
    #[serde(default)]
    pub reference_data_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHistoryInput {
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIdInput {
    pub job_id: Option<String>,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ImageGenerationEvent {
    Started {
        job_id: String,
    },
    Progress {
        job_id: String,
        message: String,
    },
    Preview {
        job_id: String,
        index: usize,
        image_data_url: String,
    },
    Completed {
        job: ImageJobDto,
    },
    Cancelled {
        job_id: String,
    },
    Failed {
        job_id: String,
        message: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAssetContent {
    pub asset_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub data_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReferenceContent {
    pub job_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub data_url: String,
}

#[derive(Debug)]
enum RunError {
    Cancelled,
    Failed(String),
}

impl From<anyhow::Error> for RunError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

struct ImagePaths {
    root: PathBuf,
    database: PathBuf,
    images: PathBuf,
    references: PathBuf,
    thumbnails: PathBuf,
    staging: PathBuf,
    exports: PathBuf,
}

impl ImagePaths {
    fn new(paths: &WorkstationPaths) -> Self {
        let root = paths.data.join("integrations").join("image-studio");
        Self {
            database: root.join("history.sqlite"),
            images: root.join("images"),
            references: root.join("references"),
            thumbnails: root.join("thumbnails"),
            staging: root.join("staging"),
            exports: paths.workspace.join("ImageExports"),
            root,
        }
    }

    fn initialize(&self) -> Result<()> {
        for directory in [
            &self.root,
            &self.images,
            &self.references,
            &self.thumbnails,
            &self.staging,
            &self.exports,
        ] {
            fs::create_dir_all(directory)
                .with_context(|| format!("无法创建 AI 生图目录 {}", directory.display()))?;
        }
        Ok(())
    }

    fn resolve_relative(&self, relative: &str) -> Result<PathBuf> {
        let relative = Path::new(relative);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.to_string_lossy().contains(':')
            || relative.to_string_lossy().contains('\\')
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!("AI 生图文件路径无效");
        }
        let resolved = self.root.join(relative);
        if !resolved.starts_with(&self.root) {
            bail!("AI 生图文件路径超出便携数据目录");
        }
        Ok(resolved)
    }
}

struct DecodedImage {
    bytes: Vec<u8>,
    mime_type: &'static str,
    extension: &'static str,
    width: u32,
    height: u32,
    image: DynamicImage,
}

struct PendingPayload {
    b64_json: Option<String>,
    url: Option<String>,
    revised_prompt: Option<String>,
}

struct GeneratedPayload {
    bytes: Vec<u8>,
    revised_prompt: Option<String>,
}

#[tauri::command]
pub fn imagegen_initialize(
    app: State<'_, AppState>,
    runtime: State<'_, ImageGenerationRuntime>,
) -> CommandResult<ImageGenerationBootstrap> {
    bootstrap(&app.paths, &runtime).map_err(CommandError::from)
}

#[tauri::command]
pub async fn imagegen_generate(
    app: State<'_, AppState>,
    runtime: State<'_, ImageGenerationRuntime>,
    input: ImageGenerateInput,
    on_event: Channel<ImageGenerationEvent>,
) -> CommandResult<ImageJobDto> {
    generate(&app.paths, &runtime, input, &on_event)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_cancel(runtime: State<'_, ImageGenerationRuntime>, job_id: String) -> bool {
    runtime.cancel(job_id.trim())
}

#[tauri::command]
pub fn imagegen_list_history(
    app: State<'_, AppState>,
    input: ImageHistoryInput,
) -> CommandResult<Vec<ImageJobDto>> {
    list_history(&app.paths, input.limit.unwrap_or(MAX_HISTORY_JOBS as u32))
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_read_asset(
    app: State<'_, AppState>,
    input: ImageIdInput,
) -> CommandResult<ImageAssetContent> {
    let asset_id = input
        .asset_id
        .as_deref()
        .ok_or_else(|| CommandError::from(anyhow!("缺少图片资产 ID")))?;
    read_asset(&app.paths, asset_id).map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_read_reference(
    app: State<'_, AppState>,
    input: ImageIdInput,
) -> CommandResult<Option<ImageReferenceContent>> {
    let job_id = input
        .job_id
        .as_deref()
        .ok_or_else(|| CommandError::from(anyhow!("缺少 AI 生图任务 ID")))?;
    read_reference(&app.paths, job_id).map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_delete_job(
    app: State<'_, AppState>,
    runtime: State<'_, ImageGenerationRuntime>,
    input: ImageIdInput,
) -> CommandResult<bool> {
    let job_id = input
        .job_id
        .as_deref()
        .ok_or_else(|| CommandError::from(anyhow!("缺少 AI 生图任务 ID")))?;
    let _ = runtime.cancel(job_id);
    delete_job(&app.paths, job_id).map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_clear_history(
    app: State<'_, AppState>,
    runtime: State<'_, ImageGenerationRuntime>,
) -> CommandResult<()> {
    clear_history(&app.paths, &runtime).map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_export_asset(
    app: State<'_, AppState>,
    input: ImageIdInput,
) -> CommandResult<String> {
    let asset_id = input
        .asset_id
        .as_deref()
        .ok_or_else(|| CommandError::from(anyhow!("缺少图片资产 ID")))?;
    export_asset(&app.paths, asset_id)
        .map(|path| path.display().to_string())
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn imagegen_open_output_directory(app: State<'_, AppState>) -> CommandResult<()> {
    let paths = ImagePaths::new(&app.paths);
    paths.initialize().map_err(CommandError::from)?;
    open_local_path(&paths.exports).map_err(CommandError::from)
}

fn bootstrap(
    workstation: &WorkstationPaths,
    runtime: &ImageGenerationRuntime,
) -> Result<ImageGenerationBootstrap> {
    let paths = ImagePaths::new(workstation);
    paths.initialize()?;
    let repository = ImageRepository::new(paths.database.clone());
    repository.initialize()?;
    runtime.recover_once(&repository)?;
    Ok(ImageGenerationBootstrap {
        database_path: repository.path().display().to_string(),
        data_dir: paths.root.display().to_string(),
        provider_state: provider_state(workstation)?,
        jobs: repository
            .list_jobs(MAX_HISTORY_JOBS)?
            .into_iter()
            .map(|job| job_dto(&paths, job))
            .collect::<Result<Vec<_>>>()?,
    })
}

pub(crate) fn initialize_storage(workstation: &WorkstationPaths) -> Result<()> {
    let paths = ImagePaths::new(workstation);
    paths.initialize()?;
    ImageRepository::new(paths.database).initialize()
}

async fn generate(
    workstation: &WorkstationPaths,
    runtime: &ImageGenerationRuntime,
    input: ImageGenerateInput,
    on_event: &Channel<ImageGenerationEvent>,
) -> Result<ImageJobDto> {
    let input = normalize_request(input)?;
    let provider = load_provider(workstation, &input.provider_id)?;
    let api_key = provider_secret(workstation, &provider)?;
    let paths = ImagePaths::new(workstation);
    paths.initialize()?;
    let repository = ImageRepository::new(paths.database.clone());
    repository.initialize()?;

    let job_id = Uuid::new_v4().to_string();
    let reference = input
        .reference_data_url
        .as_deref()
        .map(|data_url| decode_data_url(data_url, MAX_REFERENCE_BYTES))
        .transpose()?;
    let reference_relative_path = reference
        .as_ref()
        .map(|image| save_reference(&paths, &job_id, image))
        .transpose()?;
    repository.create_job(&NewImageJob {
        id: job_id.clone(),
        provider_id: provider.id.clone(),
        provider_name: provider.display_name.clone(),
        model: provider.default_model.clone(),
        mode: input.mode.clone(),
        prompt: input.prompt.clone(),
        negative_prompt: input.negative_prompt.clone(),
        size: input.size.clone(),
        quality: input.quality.clone(),
        requested_count: input.count,
        reference_relative_path,
        reference_mime_type: reference.as_ref().map(|image| image.mime_type.to_string()),
    })?;
    let mut cancel = runtime.start(&job_id)?;
    repository.set_status(&job_id, "running", None)?;
    let _ = on_event.send(ImageGenerationEvent::Started {
        job_id: job_id.clone(),
    });

    let outcome = run_request(
        &provider,
        api_key.as_str(),
        &input,
        reference.as_ref(),
        &mut cancel,
        on_event,
        &job_id,
    )
    .await;
    let result = match outcome {
        Ok(payloads) => {
            for payload in payloads {
                let asset = save_result(&paths, &job_id, payload)?;
                repository.add_asset(&asset)?;
            }
            repository.set_status(&job_id, "completed", None)?;
            let job = repository
                .get_job(&job_id)?
                .ok_or_else(|| anyhow!("AI 生图任务完成后无法读取历史"))?;
            let dto = job_dto(&paths, job)?;
            let _ = on_event.send(ImageGenerationEvent::Completed { job: dto.clone() });
            Ok(dto)
        }
        Err(RunError::Cancelled) => {
            repository.set_status(&job_id, "cancelled", Some("用户已取消生成。"))?;
            let _ = on_event.send(ImageGenerationEvent::Cancelled {
                job_id: job_id.clone(),
            });
            Err(anyhow!("用户已取消生成"))
        }
        Err(RunError::Failed(message)) => {
            repository.set_status(&job_id, "failed", Some(&message))?;
            let _ = on_event.send(ImageGenerationEvent::Failed {
                job_id: job_id.clone(),
                message: message.clone(),
            });
            Err(anyhow!(message))
        }
    };
    runtime.finish(&job_id);
    result
}

async fn run_request(
    provider: &ImageProviderConfig,
    api_key: &str,
    input: &ImageGenerateInput,
    reference: Option<&DecodedImage>,
    cancel: &mut watch::Receiver<bool>,
    on_event: &Channel<ImageGenerationEvent>,
    job_id: &str,
) -> Result<Vec<GeneratedPayload>, RunError> {
    let suffix = if input.mode == "edit" {
        "images/edits"
    } else {
        "images/generations"
    };
    let endpoint = endpoint_with_suffix(&provider.base_url, suffix).map_err(RunError::from)?;
    let client = api_client(provider.timeout_seconds, Policy::none()).map_err(RunError::from)?;
    let _ = on_event.send(ImageGenerationEvent::Progress {
        job_id: job_id.to_string(),
        message: "正在连接图片模型…".to_string(),
    });
    let mut response = send_request(
        &client,
        provider,
        api_key,
        input,
        reference,
        &endpoint,
        provider.streaming,
        cancel,
    )
    .await?;
    if response.status().as_u16() == 400 && provider.streaming {
        response = send_request(
            &client, provider, api_key, input, reference, &endpoint, false, cancel,
        )
        .await?;
    }
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let bytes = read_response_limited(&mut response, MAX_ERROR_BYTES, cancel).await?;
        return Err(RunError::Failed(api_error_message(status, &bytes)));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let pending = if content_type.contains("text/event-stream") {
        read_sse_response(&mut response, cancel, on_event, job_id).await?
    } else {
        let bytes = read_response_limited(&mut response, MAX_RESULT_BYTES, cancel).await?;
        parse_json_response(&bytes)?
    };
    if pending.is_empty() {
        return Err(RunError::Failed("图片模型没有返回任何图片".to_string()));
    }
    let mut generated = Vec::with_capacity(pending.len());
    for payload in pending {
        generated.push(resolve_payload(payload, cancel).await?);
    }
    Ok(generated)
}

async fn send_request(
    client: &Client,
    provider: &ImageProviderConfig,
    api_key: &str,
    input: &ImageGenerateInput,
    reference: Option<&DecodedImage>,
    endpoint: &Url,
    stream: bool,
    cancel: &mut watch::Receiver<bool>,
) -> Result<reqwest::Response, RunError> {
    let prompt = combined_prompt(&input.prompt, &input.negative_prompt);
    let builder = if input.mode == "edit" {
        let reference = reference.ok_or_else(|| RunError::Failed("缺少参考图片".to_string()))?;
        let part = Part::bytes(reference.bytes.clone())
            .file_name(format!("reference.{}", reference.extension))
            .mime_str(reference.mime_type)
            .map_err(|_| RunError::Failed("参考图片 MIME 类型无效".to_string()))?;
        let mut form = Form::new()
            .text("model", provider.default_model.clone())
            .text("prompt", prompt)
            .text("n", input.count.to_string())
            .part("image", part);
        if input.size != "auto" {
            form = form.text("size", input.size.clone());
        }
        if input.quality != "auto" {
            form = form.text("quality", input.quality.clone());
        }
        if stream {
            form = form.text("stream", "true").text("partial_images", "2");
        }
        client.post(endpoint.clone()).multipart(form)
    } else {
        let mut body = json!({
            "model": provider.default_model,
            "prompt": prompt,
            "n": input.count,
        });
        if input.size != "auto" {
            body["size"] = Value::String(input.size.clone());
        }
        if input.quality != "auto" {
            body["quality"] = Value::String(input.quality.clone());
        }
        if stream {
            body["stream"] = Value::Bool(true);
            body["partial_images"] = Value::Number(2.into());
        }
        client.post(endpoint.clone()).json(&body)
    }
    .bearer_auth(api_key);
    tokio::select! {
        response = builder.send() => response.map_err(|error| RunError::Failed(request_error(&error))),
        _ = wait_for_cancel(cancel) => Err(RunError::Cancelled),
    }
}

fn combined_prompt(prompt: &str, negative_prompt: &str) -> String {
    if negative_prompt.is_empty() {
        prompt.to_string()
    } else {
        format!("{prompt}\n\nAvoid the following elements: {negative_prompt}")
    }
}

async fn read_sse_response(
    response: &mut reqwest::Response,
    cancel: &mut watch::Receiver<bool>,
    on_event: &Channel<ImageGenerationEvent>,
    job_id: &str,
) -> Result<Vec<PendingPayload>, RunError> {
    let mut buffer = String::new();
    let mut payloads = Vec::new();
    let mut preview_index = 0usize;
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|_| RunError::Failed("读取图片流失败".to_string()))?,
            _ = wait_for_cancel(cancel) => return Err(RunError::Cancelled),
        };
        let Some(chunk) = chunk else { break };
        if buffer.len().saturating_add(chunk.len()) > MAX_RESULT_BYTES {
            return Err(RunError::Failed("图片流响应超过允许大小".to_string()));
        }
        buffer.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));
        while let Some(boundary) = buffer.find("\n\n") {
            let event = buffer[..boundary].to_string();
            buffer.drain(..boundary + 2);
            handle_sse_event(&event, &mut payloads, &mut preview_index, on_event, job_id)?;
        }
    }
    if !buffer.trim().is_empty() {
        handle_sse_event(
            buffer.trim(),
            &mut payloads,
            &mut preview_index,
            on_event,
            job_id,
        )?;
    }
    Ok(payloads)
}

fn handle_sse_event(
    chunk: &str,
    payloads: &mut Vec<PendingPayload>,
    preview_index: &mut usize,
    on_event: &Channel<ImageGenerationEvent>,
    job_id: &str,
) -> Result<(), RunError> {
    let mut event_name = "message";
    let mut data_lines = Vec::new();
    for line in chunk.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = value.trim();
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim());
        }
    }
    let data = data_lines.join("\n");
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let value: Value = serde_json::from_str(&data)
        .map_err(|_| RunError::Failed("图片流返回了无法解析的数据".to_string()))?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(event_name);
    if event_type == "error" || event_name == "error" {
        return Err(RunError::Failed(
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("图片模型返回流式错误")
                .to_string(),
        ));
    }
    let b64 = value
        .get("b64_json")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/data/0/b64_json").and_then(Value::as_str));
    let Some(b64) = b64 else {
        return Ok(());
    };
    if event_type.contains("partial_image") {
        let _ = on_event.send(ImageGenerationEvent::Preview {
            job_id: job_id.to_string(),
            index: *preview_index,
            image_data_url: format!("data:image/png;base64,{b64}"),
        });
        *preview_index += 1;
    } else if event_type.contains("completed") || event_name == "message" {
        payloads.push(PendingPayload {
            b64_json: Some(b64.to_string()),
            url: None,
            revised_prompt: value
                .get("revised_prompt")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    Ok(())
}

fn parse_json_response(bytes: &[u8]) -> Result<Vec<PendingPayload>, RunError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| RunError::Failed("图片模型返回内容不是有效 JSON".to_string()))?;
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| RunError::Failed("图片模型没有返回 data 数组".to_string()))?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let b64_json = item
                .get("b64_json")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            (b64_json.is_some() || url.is_some()).then(|| PendingPayload {
                b64_json,
                url,
                revised_prompt: item
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect())
}

async fn resolve_payload(
    payload: PendingPayload,
    cancel: &mut watch::Receiver<bool>,
) -> Result<GeneratedPayload, RunError> {
    let bytes = if let Some(encoded) = payload.b64_json {
        if encoded.len() > MAX_RESULT_BYTES.saturating_mul(2) {
            return Err(RunError::Failed("图片 Base64 响应超过允许大小".to_string()));
        }
        BASE64
            .decode(encoded.trim())
            .map_err(|_| RunError::Failed("图片 Base64 数据无效".to_string()))?
    } else if let Some(url) = payload.url {
        download_result(&url, cancel).await?
    } else {
        return Err(RunError::Failed(
            "图片结果缺少 Base64 数据或下载地址".to_string(),
        ));
    };
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(RunError::Failed("生成图片超过 64 MB 限制".to_string()));
    }
    Ok(GeneratedPayload {
        bytes,
        revised_prompt: payload.revised_prompt,
    })
}

async fn download_result(
    value: &str,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Vec<u8>, RunError> {
    let url =
        Url::parse(value).map_err(|_| RunError::Failed("生成图片下载地址无效".to_string()))?;
    validate_result_url(&url)?;
    let client = api_client(300, result_redirect_policy()).map_err(RunError::from)?;
    let mut response = tokio::select! {
        response = client.get(url).send() => response.map_err(|error| RunError::Failed(request_error(&error)))?,
        _ = wait_for_cancel(cancel) => return Err(RunError::Cancelled),
    };
    validate_result_url(response.url())?;
    if !response.status().is_success() {
        return Err(RunError::Failed(format!(
            "下载生成图片失败（HTTP {}）",
            response.status().as_u16()
        )));
    }
    read_response_limited(&mut response, MAX_RESULT_BYTES, cancel).await
}

async fn read_response_limited(
    response: &mut reqwest::Response,
    limit: usize,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Vec<u8>, RunError> {
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|_| RunError::Failed("读取图片模型响应失败".to_string()))?,
            _ = wait_for_cancel(cancel) => return Err(RunError::Cancelled),
        };
        let Some(chunk) = chunk else { break };
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(RunError::Failed("图片模型响应超过允许大小".to_string()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    while cancel.changed().await.is_ok() {
        if *cancel.borrow() {
            return;
        }
    }
}

fn normalize_request(mut input: ImageGenerateInput) -> Result<ImageGenerateInput> {
    input.provider_id = input.provider_id.trim().to_string();
    input.mode = input.mode.trim().to_ascii_lowercase();
    input.prompt = input.prompt.trim().to_string();
    input.negative_prompt = input.negative_prompt.trim().to_string();
    input.size = normalize_size(&input.size)?.to_string();
    input.quality = normalize_quality(&input.quality)?.to_string();
    if input.provider_id.is_empty() {
        bail!("请先选择 AI 生图供应商");
    }
    if input.prompt.is_empty() || input.prompt.chars().count() > MAX_PROMPT_CHARS {
        bail!("图片描述必须为 1 至 {MAX_PROMPT_CHARS} 个字符");
    }
    if input.negative_prompt.chars().count() > MAX_PROMPT_CHARS {
        bail!("排除内容不能超过 {MAX_PROMPT_CHARS} 个字符");
    }
    if !matches!(input.mode.as_str(), "generation" | "edit") {
        bail!("图片模式只能是文生图或图生图");
    }
    if input.mode == "edit"
        && input
            .reference_data_url
            .as_deref()
            .is_none_or(str::is_empty)
    {
        bail!("图生图需要先添加参考图片");
    }
    if !(1..=4).contains(&input.count) {
        bail!("每次生成数量必须在 1 到 4 之间");
    }
    Ok(input)
}

fn normalize_size(value: &str) -> Result<&'static str> {
    match value.trim() {
        "auto" => Ok("auto"),
        "1:1" | "1024x1024" => Ok("1024x1024"),
        "4:3" | "16:9" | "1536x1024" => Ok("1536x1024"),
        "9:16" | "1024x1536" => Ok("1024x1536"),
        "1792x1024" => Ok("1792x1024"),
        "1024x1792" => Ok("1024x1792"),
        _ => bail!("不支持的图片尺寸"),
    }
}

fn normalize_quality(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok("auto"),
        "low" => Ok("low"),
        "medium" => Ok("medium"),
        "standard" | "标准" => Ok("standard"),
        "high" | "高清" => Ok("high"),
        "hd" => Ok("hd"),
        _ => bail!("不支持的图片质量"),
    }
}

fn decode_data_url(value: &str, max_bytes: usize) -> Result<DecodedImage> {
    let (header, encoded) = value
        .split_once(',')
        .ok_or_else(|| anyhow!("参考图片不是有效 Data URL"))?;
    if !header.starts_with("data:image/") || !header.ends_with(";base64") {
        bail!("参考图片必须是 Base64 图片 Data URL");
    }
    if encoded.len() > max_bytes.saturating_mul(2) {
        bail!("参考图片超过 20 MB 限制");
    }
    let bytes = BASE64
        .decode(encoded.trim())
        .context("参考图片 Base64 数据无效")?;
    if bytes.len() > max_bytes {
        bail!("参考图片超过 20 MB 限制");
    }
    inspect_image(bytes)
}

fn inspect_image(bytes: Vec<u8>) -> Result<DecodedImage> {
    let format = image::guess_format(&bytes).context("图片格式无法识别")?;
    let (mime_type, extension) = match format {
        ImageFormat::Png => ("image/png", "png"),
        ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        ImageFormat::WebP => ("image/webp", "webp"),
        _ => bail!("仅支持 PNG、JPEG 和 WebP 图片"),
    };
    let image =
        image::load_from_memory_with_format(&bytes, format).context("图片内容损坏或无法解码")?;
    let (width, height) = image.dimensions();
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_IMAGE_PIXELS
    {
        bail!("图片尺寸超过工作站允许范围");
    }
    Ok(DecodedImage {
        bytes,
        mime_type,
        extension,
        width,
        height,
        image,
    })
}

fn save_reference(paths: &ImagePaths, job_id: &str, image: &DecodedImage) -> Result<String> {
    let file_name = format!("{job_id}.{}", image.extension);
    let destination = paths.references.join(&file_name);
    write_atomic(
        paths,
        &format!("reference-{job_id}"),
        &destination,
        &image.bytes,
    )?;
    Ok(format!("references/{file_name}"))
}

fn save_result(
    paths: &ImagePaths,
    job_id: &str,
    payload: GeneratedPayload,
) -> Result<ImageAssetRecord> {
    let decoded = inspect_image(payload.bytes)?;
    let asset_id = Uuid::new_v4().to_string();
    let file_name = format!("{asset_id}.{}", decoded.extension);
    let thumbnail_name = format!("{asset_id}.jpg");
    let destination = paths.images.join(&file_name);
    let thumbnail_destination = paths.thumbnails.join(&thumbnail_name);
    write_atomic(
        paths,
        &format!("image-{asset_id}"),
        &destination,
        &decoded.bytes,
    )?;
    let thumbnail = thumbnail_jpeg(&decoded.image)?;
    if let Err(error) = write_atomic(
        paths,
        &format!("thumbnail-{asset_id}"),
        &thumbnail_destination,
        &thumbnail,
    ) {
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    Ok(ImageAssetRecord {
        id: asset_id,
        job_id: job_id.to_string(),
        relative_path: format!("images/{file_name}"),
        thumbnail_relative_path: format!("thumbnails/{thumbnail_name}"),
        mime_type: decoded.mime_type.to_string(),
        file_size: u64::try_from(decoded.bytes.len()).unwrap_or(u64::MAX),
        width: decoded.width,
        height: decoded.height,
        revised_prompt: payload.revised_prompt,
        created_at_ms: timestamp_ms(),
    })
}

fn thumbnail_jpeg(image: &DynamicImage) -> Result<Vec<u8>> {
    let thumbnail = DynamicImage::ImageRgb8(image.thumbnail(480, 480).to_rgb8());
    let mut cursor = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut cursor, ImageFormat::Jpeg)
        .context("生成图片缩略图失败")?;
    Ok(cursor.into_inner())
}

fn write_atomic(paths: &ImagePaths, name: &str, destination: &Path, bytes: &[u8]) -> Result<()> {
    let staging = paths.staging.join(format!("{name}.partial"));
    let mut file = fs::File::create(&staging)
        .with_context(|| format!("无法创建 AI 生图临时文件 {}", staging.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&staging, destination).with_context(|| {
        let _ = fs::remove_file(&staging);
        format!("无法保存 AI 生图文件 {}", destination.display())
    })
}

fn list_history(workstation: &WorkstationPaths, limit: u32) -> Result<Vec<ImageJobDto>> {
    let paths = ImagePaths::new(workstation);
    paths.initialize()?;
    let repository = ImageRepository::new(paths.database.clone());
    repository
        .list_jobs(limit.clamp(1, MAX_HISTORY_JOBS as u32) as usize)?
        .into_iter()
        .map(|job| job_dto(&paths, job))
        .collect()
}

fn job_dto(paths: &ImagePaths, job: ImageJob) -> Result<ImageJobDto> {
    let mut asset_previews = HashMap::new();
    for asset in &job.assets {
        let thumbnail = paths.resolve_relative(&asset.thumbnail_relative_path)?;
        if thumbnail.is_file() {
            let bytes = read_limited_file(&thumbnail, MAX_RESULT_BYTES)?;
            asset_previews.insert(asset.id.clone(), data_url(&bytes, &asset.mime_type));
        }
    }
    Ok(ImageJobDto {
        job,
        asset_previews,
    })
}

fn read_asset(workstation: &WorkstationPaths, asset_id: &str) -> Result<ImageAssetContent> {
    let paths = ImagePaths::new(workstation);
    let repository = ImageRepository::new(paths.database.clone());
    let asset = repository
        .get_asset(asset_id)?
        .ok_or_else(|| anyhow!("未找到生成图片"))?;
    let file = paths.resolve_relative(&asset.relative_path)?;
    let bytes = read_limited_file(&file, MAX_RESULT_BYTES)?;
    Ok(ImageAssetContent {
        asset_id: asset.id,
        file_name: file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("generated-image")
            .to_string(),
        mime_type: asset.mime_type.clone(),
        data_url: data_url(&bytes, &asset.mime_type),
    })
}

fn read_reference(
    workstation: &WorkstationPaths,
    job_id: &str,
) -> Result<Option<ImageReferenceContent>> {
    let paths = ImagePaths::new(workstation);
    let repository = ImageRepository::new(paths.database.clone());
    let job = repository
        .get_job(job_id)?
        .ok_or_else(|| anyhow!("未找到 AI 生图任务"))?;
    let Some(relative_path) = job.reference_relative_path else {
        return Ok(None);
    };
    let file = paths.resolve_relative(&relative_path)?;
    let bytes = read_limited_file(&file, MAX_REFERENCE_BYTES)?;
    let mime_type = job
        .reference_mime_type
        .unwrap_or_else(|| "image/png".to_string());
    Ok(Some(ImageReferenceContent {
        job_id: job.id,
        file_name: file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("reference-image")
            .to_string(),
        mime_type: mime_type.clone(),
        data_url: data_url(&bytes, &mime_type),
    }))
}

fn delete_job(workstation: &WorkstationPaths, job_id: &str) -> Result<bool> {
    let paths = ImagePaths::new(workstation);
    let repository = ImageRepository::new(paths.database.clone());
    let Some(job) = repository.delete_job(job_id)? else {
        return Ok(false);
    };
    for relative in job
        .assets
        .iter()
        .flat_map(|asset| [&asset.relative_path, &asset.thumbnail_relative_path])
        .chain(job.reference_relative_path.iter())
    {
        if let Ok(file) = paths.resolve_relative(relative)
            && file.is_file()
        {
            fs::remove_file(file)?;
        }
    }
    Ok(true)
}

fn clear_history(workstation: &WorkstationPaths, runtime: &ImageGenerationRuntime) -> Result<()> {
    // 历史列表会按批次限制数量；循环清理，避免只删除最近一页记录。
    loop {
        let jobs = list_history(workstation, MAX_HISTORY_JOBS as u32)?;
        if jobs.is_empty() {
            break;
        }
        for job in jobs {
            let _ = runtime.cancel(&job.job.id);
            delete_job(workstation, &job.job.id)?;
        }
    }
    Ok(())
}

fn export_asset(workstation: &WorkstationPaths, asset_id: &str) -> Result<PathBuf> {
    let paths = ImagePaths::new(workstation);
    paths.initialize()?;
    let repository = ImageRepository::new(paths.database.clone());
    let asset = repository
        .get_asset(asset_id)?
        .ok_or_else(|| anyhow!("未找到需要导出的生成图片"))?;
    let job = repository
        .get_job(&asset.job_id)?
        .ok_or_else(|| anyhow!("生成图片所属任务不存在"))?;
    let source = paths.resolve_relative(&asset.relative_path)?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    let destination = paths.exports.join(format!(
        "{}-{}.{}",
        safe_file_component(&job.prompt),
        &asset.id[..asset.id.len().min(8)],
        extension
    ));
    fs::copy(&source, &destination).with_context(|| {
        format!(
            "无法导出图片 {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

fn read_limited_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("无法读取 AI 生图文件信息 {}", path.display()))?;
    if metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX) {
        bail!("AI 生图文件超过允许读取大小");
    }
    fs::read(path).with_context(|| format!("无法读取 AI 生图文件 {}", path.display()))
}

fn validate_result_url(url: &Url) -> Result<(), RunError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RunError::Failed(
            "生成图片下载地址包含不允许的凭据".to_string(),
        ));
    }
    let allowed = url.scheme() == "https" || (url.scheme() == "http" && is_loopback(url));
    if !allowed {
        return Err(RunError::Failed(
            "生成图片下载地址只允许 HTTPS 或本机 loopback HTTP".to_string(),
        ));
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .ok()
                .is_some_and(|address| address.is_loopback())
    })
}

fn result_redirect_policy() -> Policy {
    Policy::custom(|attempt| {
        if attempt.previous().len() >= 3 || validate_result_url(attempt.url()).is_err() {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

fn api_client(timeout_seconds: u64, redirect: Policy) -> Result<Client> {
    Client::builder()
        .redirect(redirect)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(timeout_seconds.clamp(30, 900)))
        .build()
        .context("无法创建图片模型网络客户端")
}

fn request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "图片模型请求超时，请检查网络、模型服务和生成参数".to_string()
    } else if error.is_connect() {
        "无法连接图片模型，请检查供应商 Base URL 和网络状态".to_string()
    } else {
        format!("图片模型请求失败：{error}")
    }
}

fn api_error_message(status: u16, bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(500).collect::<String>())
        })
        .unwrap_or_else(|| match status {
            400 => "图片接口拒绝了请求，请检查图片模型、尺寸和质量参数".to_string(),
            401 => "图片供应商鉴权失败，请检查 API Key".to_string(),
            403 => "当前 API Key 没有图片生成权限".to_string(),
            404 => "没有找到图片接口或图片模型，请检查 Base URL 和模型名称".to_string(),
            413 => "参考图片或请求内容过大".to_string(),
            429 => "图片供应商触发限流或额度不足，请稍后重试".to_string(),
            500..=599 => format!("图片供应商暂时不可用（HTTP {status}）"),
            _ => format!("图片供应商返回 HTTP {status}"),
        })
}

fn safe_file_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .take(36)
        .collect::<String>();
    if safe.trim().is_empty() {
        "AI生图".to_string()
    } else {
        safe
    }
}

fn data_url(bytes: &[u8], mime_type: &str) -> String {
    format!("data:{mime_type};base64,{}", BASE64.encode(bytes))
}

fn timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn open_local_path(path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| io::Error::other(format!("无法打开目录：{error}")))
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_parameters_are_normalized() {
        let request = normalize_request(ImageGenerateInput {
            provider_id: "provider".to_string(),
            mode: "generation".to_string(),
            prompt: "一座未来城市".to_string(),
            negative_prompt: String::new(),
            size: "1:1".to_string(),
            quality: "标准".to_string(),
            count: 1,
            reference_data_url: None,
        })
        .unwrap();
        assert_eq!(request.size, "1024x1024");
        assert_eq!(request.quality, "standard");
    }

    #[test]
    fn image_paths_reject_traversal() {
        let temporary = tempfile::tempdir().unwrap();
        let workstation = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let paths = ImagePaths::new(&workstation);
        assert!(paths.resolve_relative("images/a.png").is_ok());
        assert!(paths.resolve_relative("../secret.png").is_err());
        assert!(paths.resolve_relative(r"C:\\secret.png").is_err());
    }

    #[test]
    fn result_urls_allow_https_and_loopback_only() {
        assert!(validate_result_url(&Url::parse("https://cdn.example.com/a.png").unwrap()).is_ok());
        assert!(validate_result_url(&Url::parse("http://127.0.0.1:8080/a.png").unwrap()).is_ok());
        assert!(validate_result_url(&Url::parse("http://example.com/a.png").unwrap()).is_err());
    }
}
