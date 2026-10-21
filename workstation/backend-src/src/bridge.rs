use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::model::Engine;
use crate::paths::WorkstationPaths;
use crate::protocol::{
    force_non_stream, request_streaming, safe_error_body, synthetic_sse, translate_request,
    translate_response, upstream_endpoint,
};
use crate::provider::{
    BRIDGE_PORT, ProviderConfig, ProviderProtocol, load, local_token, upstream_secret,
};
use crate::windows_process::{configure_command, process_is_alive, replace_file_atomic};

const BRIDGE_START_TIMEOUT: Duration = Duration::from_secs(8);
const BRIDGE_STOP_TIMEOUT: Duration = Duration::from_secs(8);
const PROVIDER_TEST_TIMEOUT: Duration = Duration::from_secs(20);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REQUEST_LOG_SENDER: OnceLock<mpsc::Sender<RequestLogMessage>> = OnceLock::new();

struct RequestLogLine {
    path: std::path::PathBuf,
    line: String,
}

enum RequestLogMessage {
    Line(RequestLogLine),
    Flush(mpsc::Sender<()>),
}

#[derive(Clone)]
struct AppState {
    paths: WorkstationPaths,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeState {
    pub status: String,
    pub process_id: u32,
    pub port: u16,
    pub bind: String,
    pub started_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub engine: Engine,
    pub success: bool,
    pub status_code: u16,
    pub local_route: String,
    pub upstream_protocol: ProviderProtocol,
    pub message: String,
}

pub async fn start(paths: &WorkstationPaths) -> Result<BridgeState> {
    paths.prepare_layout()?;
    if let Some(state) = read_state(paths)?
        && state.status == "running"
        && process_is_alive(state.process_id)
        && health_check(state.port).await
    {
        return Ok(state);
    }
    remove_if_exists(&paths.bridge_stop_file())?;
    persist_bridge_logs(paths)?;
    prepare_bridge_session_logs(paths)?;
    let executable = bridge_executable(paths)?;
    let mut command = Command::new(executable);
    let bridge_logs = bridge_log_dir(paths);
    let stdout = append_log_file(&bridge_logs.join("bridge.stdout.log"))?;
    let stderr = append_log_file(&bridge_logs.join("bridge.stderr.log"))?;
    command
        .arg("--root")
        .arg(&paths.root)
        .arg("bridge")
        .arg("serve")
        .current_dir(&paths.backend)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    configure_command(&mut command, false);
    let child = command.spawn().context("无法启动本地协议桥")?;
    let deadline = tokio::time::Instant::now() + BRIDGE_START_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if health_check(BRIDGE_PORT).await {
            return read_state(paths)?.ok_or_else(|| anyhow!("协议桥已监听但缺少状态文件"));
        }
        if !process_is_alive(child.id()) {
            bail!("协议桥进程提前退出；端口{BRIDGE_PORT}可能被占用");
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    bail!("协议桥在{}秒内未就绪", BRIDGE_START_TIMEOUT.as_secs())
}

fn bridge_executable(paths: &WorkstationPaths) -> Result<std::path::PathBuf> {
    let windows_sidecar = paths.backend.join("workstation-runtime.exe");
    if windows_sidecar.is_file() {
        return Ok(windows_sidecar);
    }
    let unix_sidecar = paths.backend.join("workstation-runtime");
    if unix_sidecar.is_file() {
        return Ok(unix_sidecar);
    }
    let current = std::env::current_exe().context("无法获取桥接后端路径")?;
    let file_name = current
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.starts_with("workstation-runtime") {
        return Ok(current);
    }
    bail!("缺少受控桥接后端；应存在 {}", windows_sidecar.display())
}

pub async fn ensure_running(paths: &WorkstationPaths) -> Result<BridgeState> {
    start(paths).await
}

pub async fn serve(paths: WorkstationPaths) -> Result<()> {
    paths.prepare_layout()?;
    remove_if_exists(&paths.bridge_stop_file())?;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), BRIDGE_PORT);
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("无法绑定本地协议桥 {address}"))?;
    let now = now_unix_ms();
    write_state(
        &paths,
        &BridgeState {
            status: "running".to_string(),
            process_id: std::process::id(),
            port: BRIDGE_PORT,
            bind: "127.0.0.1".to_string(),
            started_at_unix_ms: now,
            updated_at_unix_ms: now,
            message: "本地协议桥已就绪".to_string(),
        },
    )?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .user_agent("YunshuPortableWorkstation/Phase2")
        .build()
        .context("无法创建协议桥HTTP客户端")?;
    let app = Router::new()
        .route("/health", get(health))
        .route("/codex/v1/models", get(codex_models))
        .route("/codex/v1/responses", post(codex_responses))
        .route("/claude/v1/models", get(claude_models))
        .route("/claude/v1/messages", post(claude_messages))
        .route("/openclaw/v1/models", get(openclaw_models))
        .route("/openclaw/v1/responses", post(openclaw_responses))
        .route(
            "/openclaw/v1/chat/completions",
            post(openclaw_chat_completions),
        )
        .route("/openclaw/v1/messages", post(openclaw_messages))
        .with_state(AppState {
            paths: paths.clone(),
            client,
        });
    let shutdown_paths = paths.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !shutdown_paths.bridge_stop_file().exists() {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await;
    let stopped_at = now_unix_ms();
    let _ = write_state(
        &paths,
        &BridgeState {
            status: "stopped".to_string(),
            process_id: 0,
            port: BRIDGE_PORT,
            bind: "127.0.0.1".to_string(),
            started_at_unix_ms: now,
            updated_at_unix_ms: stopped_at,
            message: "本地协议桥已停止".to_string(),
        },
    );
    let _ = remove_if_exists(&paths.bridge_stop_file());
    flush_request_log_writer();
    let _ = persist_bridge_logs(&paths);
    result.context("本地协议桥服务异常退出")
}

pub async fn stop(paths: &WorkstationPaths) -> Result<BridgeState> {
    let Some(state) = read_state(paths)? else {
        persist_bridge_logs(paths)?;
        return Ok(stopped_state());
    };
    if state.status != "running" || !process_is_alive(state.process_id) {
        let stopped = stopped_state();
        write_state(paths, &stopped)?;
        persist_bridge_logs(paths)?;
        return Ok(stopped);
    }
    fs::write(paths.bridge_stop_file(), b"stop")
        .with_context(|| format!("无法写入 {}", paths.bridge_stop_file().display()))?;
    let deadline = tokio::time::Instant::now() + BRIDGE_STOP_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if !process_is_alive(state.process_id) {
            persist_bridge_logs(paths)?;
            return Ok(read_state(paths)?.unwrap_or_else(stopped_state));
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    bail!(
        "协议桥PID={}未在{}秒内正常退出；未强杀，请先检查占用",
        state.process_id,
        BRIDGE_STOP_TIMEOUT.as_secs()
    )
}

pub async fn status(paths: &WorkstationPaths) -> Result<BridgeState> {
    let Some(mut state) = read_state(paths)? else {
        return Ok(stopped_state());
    };
    if state.status == "running"
        && (!process_is_alive(state.process_id) || !health_check(state.port).await)
    {
        state.status = "stopped".to_string();
        state.process_id = 0;
        state.updated_at_unix_ms = now_unix_ms();
        state.message = "状态文件存在，但协议桥进程或健康检查已失效".to_string();
        write_state(paths, &state)?;
        persist_bridge_logs(paths)?;
    }
    Ok(state)
}

pub async fn test_provider(paths: &WorkstationPaths, engine: Engine) -> Result<ProviderTestResult> {
    let config = load(paths, engine)?;
    let resolved_protocol = config.resolved_protocol;
    let token = local_token(paths, &config)?;
    ensure_running(paths).await?;
    let (route, body) = match engine {
        Engine::Codex => (
            format!("http://127.0.0.1:{BRIDGE_PORT}/codex/v1/responses"),
            json!({
                "model": config.model,
                "input": "请只回复 OK",
                "max_output_tokens": 8,
                "stream": false
            }),
        ),
        Engine::Claude => (
            format!("http://127.0.0.1:{BRIDGE_PORT}/claude/v1/messages"),
            json!({
                "model": config.model,
                "messages": [{"role":"user","content":"请只回复 OK"}],
                "max_tokens": 8,
                "stream": false
            }),
        ),
        Engine::OpenClaw => match resolved_protocol {
            ProviderProtocol::Auto => unreachable!("供应商协议保存时已经解析"),
            ProviderProtocol::Responses => (
                format!("http://127.0.0.1:{BRIDGE_PORT}/openclaw/v1/responses"),
                json!({"model":config.model,"input":"请只回复 OK","max_output_tokens":8,"stream":false}),
            ),
            ProviderProtocol::ChatCompletions => (
                format!("http://127.0.0.1:{BRIDGE_PORT}/openclaw/v1/chat/completions"),
                json!({"model":config.model,"messages":[{"role":"user","content":"请只回复 OK"}],"max_tokens":8,"stream":false}),
            ),
            ProviderProtocol::AnthropicMessages => (
                format!("http://127.0.0.1:{BRIDGE_PORT}/openclaw/v1/messages"),
                json!({"model":config.model,"messages":[{"role":"user","content":"请只回复 OK"}],"max_tokens":8,"stream":false}),
            ),
        },
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(PROVIDER_TEST_TIMEOUT)
        .build()?;
    let mut request = client
        .post(&route)
        .header(header::CONTENT_TYPE, "application/json")
        .json(&body);
    request = if engine == Engine::Claude {
        request.header("x-api-key", token.as_str())
    } else {
        request.bearer_auth(token.as_str())
    };
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            anyhow!(
                "供应商测试超过{}秒；上游响应过慢，但真实对话仍使用保存的{}秒超时",
                PROVIDER_TEST_TIMEOUT.as_secs(),
                config.timeout_seconds
            )
        } else {
            anyhow!("无法连接本地协议桥：{}", redact_error(&error.to_string()))
        }
    })?;
    let status_code = response.status().as_u16();
    let success = response.status().is_success();
    Ok(ProviderTestResult {
        engine,
        success,
        status_code,
        local_route: route,
        upstream_protocol: resolved_protocol,
        message: if success {
            "供应商最小请求通过".to_string()
        } else {
            format!("供应商返回HTTP {status_code}；请检查Base URL、协议、模型和Key")
        },
    })
}

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({
            "ok": true,
            "service": "yunshu-provider-bridge",
            "bind": "127.0.0.1",
            "port": BRIDGE_PORT
        })),
    )
}

async fn codex_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    models_response(&state, Engine::Codex, &headers)
}

async fn claude_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    models_response(&state, Engine::Claude, &headers)
}

async fn openclaw_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    models_response(&state, Engine::OpenClaw, &headers)
}

async fn codex_responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        &state,
        Engine::Codex,
        ProviderProtocol::Responses,
        headers,
        body,
    )
    .await
}

async fn claude_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        &state,
        Engine::Claude,
        ProviderProtocol::AnthropicMessages,
        headers,
        body,
    )
    .await
}

async fn openclaw_responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        &state,
        Engine::OpenClaw,
        ProviderProtocol::Responses,
        headers,
        body,
    )
    .await
}

async fn openclaw_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        &state,
        Engine::OpenClaw,
        ProviderProtocol::ChatCompletions,
        headers,
        body,
    )
    .await
}

async fn openclaw_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        &state,
        Engine::OpenClaw,
        ProviderProtocol::AnthropicMessages,
        headers,
        body,
    )
    .await
}

fn models_response(state: &AppState, engine: Engine, headers: &HeaderMap) -> Response {
    match authenticated_config(&state.paths, engine, headers) {
        Ok(config) => {
            let mut model_ids = vec![
                config.models.default.clone(),
                config.models.opus.clone(),
                config.models.sonnet.clone(),
                config.models.haiku.clone(),
            ];
            if config.models.subagent != "inherit" {
                model_ids.push(config.models.subagent.clone());
            }
            if engine == Engine::Codex {
                model_ids = crate::provider::codex_model_ids(&config);
            }
            model_ids.sort();
            model_ids.dedup();
            json_response(
                StatusCode::OK,
                json!({
                    "object":"list",
                    "data":model_ids.into_iter().map(|id| json!({
                        "id":id,
                        "object":"model",
                        "owned_by":config.display_name,
                        "protocol":config.resolved_protocol
                    })).collect::<Vec<_>>()
                }),
            )
        }
        Err(response) => *response,
    }
}

async fn proxy(
    state: &AppState,
    engine: Engine,
    incoming_protocol: ProviderProtocol,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let request_id = format!(
        "{}-{}-{}",
        engine.slug(),
        now_unix_ms(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let config = match authenticated_config(&state.paths, engine, &headers) {
        Ok(config) => config,
        Err(response) => return *response,
    };
    let resolved_protocol = config.resolved_protocol;
    append_request_event(
        &state.paths,
        &request_id,
        engine,
        "accepted",
        started.elapsed().as_millis(),
        &format!(
            "incoming={} upstream={}",
            incoming_protocol.slug(),
            resolved_protocol.slug()
        ),
    );
    let incoming_body: Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(error) => {
            append_request_event(
                &state.paths,
                &request_id,
                engine,
                "upstream_error",
                started.elapsed().as_millis(),
                &redact_error(&error.to_string()),
            );
            return json_response(
                StatusCode::BAD_REQUEST,
                safe_error_body("invalid_json", &format!("请求JSON无法解析：{error}")),
            );
        }
    };
    let requested_stream = request_streaming(&incoming_body);
    let cross_protocol = incoming_protocol != resolved_protocol;
    let mut selected_model = config.model.clone();
    if engine == Engine::Codex {
        if let Some(requested) = incoming_body.get("model") {
            let Some(requested) = requested.as_str().filter(|id| !id.trim().is_empty()) else {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    safe_error_body("invalid_model", "模型 ID 必须是非空字符串"),
                );
            };
            if !crate::provider::codex_model_ids(&config)
                .iter()
                .any(|id| id == requested)
            {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    safe_error_body(
                        "invalid_model",
                        "所选模型不在当前供应商的已保存列表中，请重新获取模型并保存配置",
                    ),
                );
            }
            selected_model = requested.to_string();
        }
    }
    let codex_effort = (engine == Engine::Codex
        && incoming_protocol == ProviderProtocol::Responses
        && cross_protocol)
        .then(|| incoming_body.pointer("/reasoning/effort").cloned())
        .flatten()
        .filter(|effort| !effort.is_null());
    let mut upstream_body = match translate_request(
        incoming_protocol,
        resolved_protocol,
        incoming_body,
        &selected_model,
    ) {
        Ok(body) => body,
        Err(error) => {
            return json_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                safe_error_body("unsupported_protocol_feature", &error.to_string()),
            );
        }
    };
    if let Some(effort) = codex_effort {
        // 仅传递 Codex 的选择，不按型号猜测能力或静默降档。
        if resolved_protocol == ProviderProtocol::ChatCompletions {
            upstream_body["reasoning_effort"] = effort;
        } else {
            return json_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                safe_error_body(
                    "unsupported_reasoning_protocol",
                    "当前 Messages 接口尚无已验证的推理强度映射，请使用支持该参数的 Responses 或 Chat Completions 接口",
                ),
            );
        }
    }
    if cross_protocol && requested_stream {
        // 跨协议时先完整转换，再合成为目标客户端可识别的SSE，避免不同事件语义互相污染。
        force_non_stream(&mut upstream_body);
    }

    let secret = match upstream_secret(&state.paths, &config) {
        Ok(secret) => secret,
        Err(error) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                safe_error_body("credential_unavailable", &error.to_string()),
            );
        }
    };
    let endpoint = upstream_endpoint(&config.base_url, resolved_protocol);
    let mut request = state
        .client
        .post(endpoint)
        .timeout(Duration::from_secs(config.timeout_seconds))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::ACCEPT,
            if requested_stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .json(&upstream_body);
    request = match resolved_protocol {
        ProviderProtocol::Auto => unreachable!("供应商协议保存时已经解析"),
        ProviderProtocol::AnthropicMessages => request
            .header("x-api-key", secret.as_str())
            .header("anthropic-version", "2023-06-01"),
        ProviderProtocol::Responses | ProviderProtocol::ChatCompletions => {
            request.bearer_auth(secret.as_str())
        }
    };
    if let Some(beta) = headers.get("anthropic-beta") {
        request = request.header("anthropic-beta", beta);
    }
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return json_response(
                StatusCode::BAD_GATEWAY,
                safe_error_body(
                    "upstream_unreachable",
                    &format!("供应商连接失败：{}", redact_error(&error.to_string())),
                ),
            );
        }
    };
    let status = upstream.status();
    append_request_event(
        &state.paths,
        &request_id,
        engine,
        "upstream_headers",
        started.elapsed().as_millis(),
        &format!("status={}", status.as_u16()),
    );
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    if !status.is_success() {
        let bytes = upstream.bytes().await.unwrap_or_default();
        append_request_event(
            &state.paths,
            &request_id,
            engine,
            "upstream_http_error",
            started.elapsed().as_millis(),
            &format!("status={}", status.as_u16()),
        );
        return response_with_bytes_with_request_id(status, content_type, bytes, &request_id);
    }
    if !cross_protocol {
        let upstream_stream = Box::pin(upstream.bytes_stream());
        let guard =
            RequestStreamGuard::new(state.paths.clone(), request_id.clone(), engine, started);
        let stream = futures_util::stream::unfold(
            (upstream_stream, guard, false),
            |(mut upstream_stream, mut guard, first_byte_seen)| async move {
                if guard.finished {
                    return None;
                }
                match upstream_stream.next().await {
                    Some(Ok(bytes)) => {
                        let next_first_byte_seen = first_byte_seen || !bytes.is_empty();
                        if !first_byte_seen && !bytes.is_empty() {
                            guard.log("first_byte", "stream");
                        }
                        Some((
                            Ok::<Bytes, std::io::Error>(bytes),
                            (upstream_stream, guard, next_first_byte_seen),
                        ))
                    }
                    Some(Err(error)) => {
                        guard.finish("stream_error", &redact_error(&error.to_string()));
                        Some((
                            Err(std::io::Error::other(error)),
                            (upstream_stream, guard, first_byte_seen),
                        ))
                    }
                    None => {
                        guard.finish("completed", "stream");
                        None
                    }
                }
            },
        );
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, content_type)
            .header("x-yunshu-request-id", request_id)
            .body(Body::from_stream(stream))
            .unwrap_or_else(|error| {
                json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    safe_error_body("response_build_failed", &error.to_string()),
                )
            });
    }

    let upstream_json: Value = match upstream.json().await {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode::BAD_GATEWAY,
                safe_error_body(
                    "upstream_invalid_json",
                    &format!("供应商响应不是有效JSON：{error}"),
                ),
            );
        }
    };
    let translated = match translate_response(resolved_protocol, incoming_protocol, upstream_json) {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode::BAD_GATEWAY,
                safe_error_body("response_translation_failed", &error.to_string()),
            );
        }
    };
    append_request_event(
        &state.paths,
        &request_id,
        engine,
        "completed",
        started.elapsed().as_millis(),
        "translated",
    );
    if requested_stream {
        match synthetic_sse(incoming_protocol, &translated) {
            Ok(bytes) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-cache")
                .header("x-yunshu-request-id", request_id)
                .body(Body::from(bytes))
                .unwrap(),
            Err(error) => json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                safe_error_body("sse_translation_failed", &error.to_string()),
            ),
        }
    } else {
        json_response_with_request_id(StatusCode::OK, translated, &request_id)
    }
}

fn authenticated_config(
    paths: &WorkstationPaths,
    engine: Engine,
    headers: &HeaderMap,
) -> std::result::Result<ProviderConfig, Box<Response>> {
    let config = load(paths, engine).map_err(|error| {
        Box::new(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            safe_error_body("provider_not_configured", &error.to_string()),
        ))
    })?;
    if !config.enabled {
        return Err(Box::new(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            safe_error_body("provider_disabled", "该智能体的供应商已停用"),
        )));
    }
    let expected = local_token(paths, &config).map_err(|error| {
        Box::new(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            safe_error_body("local_token_unavailable", &error.to_string()),
        ))
    })?;
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let api_key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());
    if bearer != Some(expected.as_str()) && api_key != Some(expected.as_str()) {
        return Err(Box::new(json_response(
            StatusCode::UNAUTHORIZED,
            safe_error_body("invalid_local_token", "本地智能体访问令牌无效"),
        )));
    }
    Ok(config)
}

fn json_response(status: StatusCode, value: Value) -> Response {
    let bytes =
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"serialize\"}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(bytes))
        .unwrap()
}

fn json_response_with_request_id(status: StatusCode, value: Value, request_id: &str) -> Response {
    let bytes =
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"serialize\"}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header("x-yunshu-request-id", request_id)
        .body(Body::from(bytes))
        .unwrap()
}

fn response_with_bytes_with_request_id(
    status: StatusCode,
    content_type: HeaderValue,
    bytes: Bytes,
    request_id: &str,
) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header("x-yunshu-request-id", request_id)
        .body(Body::from(bytes))
        .unwrap()
}

struct RequestStreamGuard {
    paths: WorkstationPaths,
    request_id: String,
    engine: Engine,
    started: Instant,
    finished: bool,
}

impl RequestStreamGuard {
    fn new(paths: WorkstationPaths, request_id: String, engine: Engine, started: Instant) -> Self {
        Self {
            paths,
            request_id,
            engine,
            started,
            finished: false,
        }
    }

    fn log(&self, stage: &str, detail: &str) {
        append_request_event(
            &self.paths,
            &self.request_id,
            self.engine,
            stage,
            self.started.elapsed().as_millis(),
            detail,
        );
    }

    fn finish(&mut self, stage: &str, detail: &str) {
        self.log(stage, detail);
        self.finished = true;
    }
}

impl Drop for RequestStreamGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.log(
                "downstream_disconnected",
                "response body dropped before completion",
            );
        }
    }
}

fn append_request_event(
    paths: &WorkstationPaths,
    request_id: &str,
    engine: Engine,
    stage: &str,
    elapsed_ms: u128,
    detail: &str,
) {
    let event = json!({
        "timestampUnixMs": now_unix_ms(),
        "requestId": request_id,
        "engine": engine.slug(),
        "stage": stage,
        "elapsedMs": elapsed_ms,
        "detail": detail,
    });
    // U盘尤其是 exFAT 上的同步打开/写入会直接拖慢首字返回。请求线程只投递一条
    // 内存消息，后台线程负责落盘；日志失败不能阻塞或改变真实模型响应。
    let sender = REQUEST_LOG_SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<RequestLogMessage>();
        let _ = thread::Builder::new()
            .name("yunshu-bridge-log-writer".to_string())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    match message {
                        RequestLogMessage::Line(message) => {
                            if let Ok(mut file) = append_log_file(&message.path) {
                                let _ = writeln!(file, "{}", message.line);
                            }
                        }
                        RequestLogMessage::Flush(completed) => {
                            let _ = completed.send(());
                        }
                    }
                }
            });
        sender
    });
    let _ = sender.send(RequestLogMessage::Line(RequestLogLine {
        path: bridge_log_dir(paths).join("bridge-requests.jsonl"),
        line: event.to_string(),
    }));
}

fn flush_request_log_writer() {
    let Some(sender) = REQUEST_LOG_SENDER.get() else {
        return;
    };
    let (completed, receiver) = mpsc::channel();
    if sender.send(RequestLogMessage::Flush(completed)).is_ok() {
        let _ = receiver.recv_timeout(Duration::from_secs(2));
    }
}

fn bridge_log_dir(paths: &WorkstationPaths) -> std::path::PathBuf {
    paths.machine_engine_cache("bridge").join("logs")
}

fn prepare_bridge_session_logs(paths: &WorkstationPaths) -> Result<()> {
    let logs = bridge_log_dir(paths);
    fs::create_dir_all(&logs)
        .with_context(|| format!("无法创建本机协议桥日志目录 {}", logs.display()))?;
    for name in [
        "bridge.stdout.log",
        "bridge.stderr.log",
        "bridge-requests.jsonl",
    ] {
        fs::write(logs.join(name), b"")
            .with_context(|| format!("无法初始化本机协议桥日志 {name}"))?;
    }
    Ok(())
}

fn persist_bridge_logs(paths: &WorkstationPaths) -> Result<()> {
    flush_request_log_writer();
    let source_dir = bridge_log_dir(paths);
    for name in [
        "bridge.stdout.log",
        "bridge.stderr.log",
        "bridge-requests.jsonl",
    ] {
        let source = source_dir.join(name);
        if !source.is_file() {
            continue;
        }
        let bytes = fs::read(&source)
            .with_context(|| format!("无法读取本机协议桥日志 {}", source.display()))?;
        let target = paths.logs.join(name);
        if fs::read(&target).is_ok_and(|existing| existing == bytes) {
            continue;
        }
        fs::create_dir_all(&paths.logs)
            .with_context(|| format!("无法创建便携日志目录 {}", paths.logs.display()))?;
        let temporary = target.with_extension("sync.tmp");
        fs::write(&temporary, &bytes)
            .with_context(|| format!("无法写入日志快照 {}", temporary.display()))?;
        replace_file_atomic(&temporary, &target)?;
    }
    Ok(())
}

fn append_log_file(path: &std::path::Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("无法打开日志 {}", path.display()))
}

fn read_state(paths: &WorkstationPaths) -> Result<Option<BridgeState>> {
    if !paths.bridge_state.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&paths.bridge_state)
        .with_context(|| format!("无法读取 {}", paths.bridge_state.display()))?;
    Ok(Some(serde_json::from_slice(&bytes).with_context(|| {
        format!("无法解析 {}", paths.bridge_state.display())
    })?))
}

fn write_state(paths: &WorkstationPaths, state: &BridgeState) -> Result<()> {
    fs::create_dir_all(&paths.runtime_state)?;
    let temporary = paths.bridge_state.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, &paths.bridge_state)
}

fn stopped_state() -> BridgeState {
    BridgeState {
        status: "stopped".to_string(),
        process_id: 0,
        port: BRIDGE_PORT,
        bind: "127.0.0.1".to_string(),
        started_at_unix_ms: 0,
        updated_at_unix_ms: now_unix_ms(),
        message: "尚未启动".to_string(),
    }
}

async fn health_check(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(400))
        .timeout(Duration::from_millis(800))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

fn redact_error(error: &str) -> String {
    error
        .split_whitespace()
        .take(32)
        .collect::<Vec<_>>()
        .join(" ")
}

fn remove_if_exists(path: &std::path::Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!(error)).with_context(|| format!("无法删除 {}", path.display())),
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::OriginalUri;
    use axum::routing::post;
    use zeroize::Zeroizing;

    use super::*;
    use crate::provider::{ConfigureProvider, configure};

    #[derive(Clone)]
    struct MockState {
        observations: Arc<Mutex<Vec<String>>>,
    }

    async fn mock_upstream(
        State(state): State<MockState>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        let path = uri.path().to_string();
        if let Some(effort) = body.get("reasoning_effort") {
            state
                .observations
                .lock()
                .unwrap()
                .push(format!("effort:{effort}"));
        }
        if body["reasoning_effort"] == "unsupported" {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error":{"message":"unsupported reasoning effort"}}),
            );
        }
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let api_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        state.observations.lock().unwrap().push(format!(
            "{path}|{authorization}|{api_key}|{}",
            body.get("model").and_then(Value::as_str).unwrap_or("")
        ));
        match path.as_str() {
            "/codex/v1/chat/completions" => json_response(
                StatusCode::OK,
                json!({"id":"chat_1","model":"codex-model","choices":[{"message":{"role":"assistant","content":"codex-ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1}}),
            ),
            "/claude/v1/responses" => json_response(
                StatusCode::OK,
                json!({"id":"resp_1","model":"claude-model","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"claude-ok"}]}],"usage":{"input_tokens":2,"output_tokens":1}}),
            ),
            "/openclaw/v1/messages" => json_response(
                StatusCode::OK,
                json!({"id":"msg_1","type":"message","model":"openclaw-model","content":[{"type":"text","text":"openclaw-ok"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":1}}),
            ),
            "/direct/v1/chat/completions" => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from(
                    "data: {\"id\":\"chunk_1\",\"choices\":[{\"delta\":{\"content\":\"direct-ok\"}}]}\n\ndata: [DONE]\n\n",
                ))
                .unwrap(),
            _ => json_response(
                StatusCode::NOT_FOUND,
                json!({"error":{"message":"unexpected mock path"}}),
            ),
        }
    }

    fn configure_mock(
        paths: &WorkstationPaths,
        engine: Engine,
        base_url: &str,
        model: &str,
        protocol: ProviderProtocol,
        key: &str,
    ) {
        configure(
            paths,
            ConfigureProvider {
                engine,
                display_name: format!("{engine} mock"),
                base_url: base_url.to_string(),
                model: model.to_string(),
                models: None,
                protocol,
                workspace: None,
                timeout_seconds: 30,
                api_key: Some(Zeroizing::new(key.to_string())),
            },
        )
        .unwrap();
    }

    async fn invoke_proxy(
        state: &AppState,
        engine: Engine,
        incoming_protocol: ProviderProtocol,
        body: Value,
    ) -> (StatusCode, Value) {
        let (status, _, bytes) = invoke_proxy_raw(state, engine, incoming_protocol, body).await;
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn invoke_proxy_raw(
        state: &AppState,
        engine: Engine,
        incoming_protocol: ProviderProtocol,
        body: Value,
    ) -> (StatusCode, Option<String>, Bytes) {
        let config = load(&state.paths, engine).unwrap();
        let token = local_token(&state.paths, &config).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token.as_str())).unwrap(),
        );
        let response = proxy(
            state,
            engine,
            incoming_protocol,
            headers,
            Bytes::from(serde_json::to_vec(&body).unwrap()),
        )
        .await;
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, content_type, bytes)
    }

    #[tokio::test]
    async fn codex_selected_catalog_model_reaches_upstream() {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let mock = Router::new()
            .route("/{*path}", post(mock_upstream))
            .with_state(MockState {
                observations: observations.clone(),
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, mock).await.unwrap();
        });
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("模型测试"));
        configure_mock(
            &paths,
            Engine::Codex,
            &format!("http://{address}/codex/v1"),
            "codex-model",
            ProviderProtocol::ChatCompletions,
            "test-only-key",
        );
        let path = paths.provider_file("codex");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["models"]["available"] = json!(["codex-model", "DeepSeek/test"]);
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        let state = AppState {
            paths,
            client: reqwest::Client::new(),
        };
        let (status, _) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"DeepSeek/test", "input":"test", "stream":false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            observations
                .lock()
                .unwrap()
                .iter()
                .any(|entry| entry.ends_with("|DeepSeek/test")),
            "Codex selection must not be replaced by the provider default"
        );
        let before = observations.lock().unwrap().len();
        let (status, _) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"stale-model", "input":"test"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(observations.lock().unwrap().len(), before);
        for effort in ["low", "medium", "high", "xhigh", "max", "ultra"] {
            let (status, _) = invoke_proxy(
                &state,
                Engine::Codex,
                ProviderProtocol::Responses,
                json!({"model":"DeepSeek/test", "input":"test", "reasoning":{"effort":effort}}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                observations
                    .lock()
                    .unwrap()
                    .contains(&format!("effort:\"{effort}\"")),
                "selected reasoning effort must reach Chat Completions unchanged"
            );
        }
        let (status, _) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"DeepSeek/test", "input":"test", "reasoning":{"effort":"unsupported"}}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, stream) = invoke_proxy_raw(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"DeepSeek/test", "input":"test", "stream":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = String::from_utf8(stream.to_vec()).unwrap();
        assert!(events.contains("response.output_text.delta"));
        assert!(events.contains("response.completed"));
        assert!(
            observations
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .ends_with("|DeepSeek/test")
        );
        let (status, _) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"input":"test", "stream":false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            observations
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .ends_with("|codex-model")
        );
        configure_mock(
            &state.paths,
            Engine::Codex,
            &format!("http://{address}/codex/v1"),
            "codex-model",
            ProviderProtocol::AnthropicMessages,
            "test-only-key",
        );
        let before = observations.lock().unwrap().len();
        let (status, body) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"codex-model", "input":"test", "reasoning":{"effort":"high"}}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.to_string().contains("unsupported_reasoning_protocol"));
        assert_eq!(observations.lock().unwrap().len(), before);
        server.abort();
    }

    #[tokio::test]
    async fn three_routes_use_independent_keys_models_and_protocols() {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let mock = Router::new()
            .route("/{*path}", post(mock_upstream))
            .with_state(MockState {
                observations: observations.clone(),
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, mock).await.unwrap();
        });

        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 dev2"));
        paths.prepare_layout().unwrap();
        configure_mock(
            &paths,
            Engine::Codex,
            &format!("http://{address}/codex/v1"),
            "codex-model",
            ProviderProtocol::ChatCompletions,
            "sk-codex-only",
        );
        configure_mock(
            &paths,
            Engine::Claude,
            &format!("http://{address}/claude/v1"),
            "claude-model",
            ProviderProtocol::Responses,
            "sk-claude-only",
        );
        configure_mock(
            &paths,
            Engine::OpenClaw,
            &format!("http://{address}/openclaw/v1"),
            "openclaw-model",
            ProviderProtocol::AnthropicMessages,
            "sk-openclaw-only",
        );
        let state = AppState {
            paths: paths.clone(),
            client: reqwest::Client::new(),
        };

        let (codex_status, codex) = invoke_proxy(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"codex-model","input":"test","stream":false}),
        )
        .await;
        assert_eq!(codex_status, StatusCode::OK);
        assert_eq!(codex["output"][0]["content"][0]["text"], "codex-ok");
        let (codex_stream_status, codex_stream_content_type, codex_stream) = invoke_proxy_raw(
            &state,
            Engine::Codex,
            ProviderProtocol::Responses,
            json!({"model":"codex-model","input":"test","stream":true}),
        )
        .await;
        assert_eq!(codex_stream_status, StatusCode::OK);
        assert_eq!(
            codex_stream_content_type.as_deref(),
            Some("text/event-stream; charset=utf-8")
        );
        let codex_stream = String::from_utf8(codex_stream.to_vec()).unwrap();
        assert!(codex_stream.contains("response.output_text.delta"));
        assert!(codex_stream.contains("response.completed"));

        let (claude_status, claude) = invoke_proxy(
            &state,
            Engine::Claude,
            ProviderProtocol::AnthropicMessages,
            json!({"model":"wrong","messages":[{"role":"user","content":"test"}],"max_tokens":8,"stream":false}),
        )
        .await;
        assert_eq!(claude_status, StatusCode::OK);
        assert_eq!(claude["content"][0]["text"], "claude-ok");

        let (openclaw_status, openclaw) = invoke_proxy(
            &state,
            Engine::OpenClaw,
            ProviderProtocol::ChatCompletions,
            json!({"model":"wrong","messages":[{"role":"user","content":"test"}],"stream":false}),
        )
        .await;
        assert_eq!(openclaw_status, StatusCode::OK);
        assert_eq!(openclaw["choices"][0]["message"]["content"], "openclaw-ok");

        let observed = observations.lock().unwrap().clone();
        assert!(observed.iter().any(|entry| {
            entry == "/codex/v1/chat/completions|Bearer sk-codex-only||codex-model"
        }));
        assert!(
            observed.iter().any(|entry| {
                entry == "/claude/v1/responses|Bearer sk-claude-only||claude-model"
            })
        );
        assert!(
            observed
                .iter()
                .any(|entry| { entry == "/openclaw/v1/messages||sk-openclaw-only|openclaw-model" })
        );

        configure_mock(
            &paths,
            Engine::OpenClaw,
            &format!("http://{address}/direct/v1"),
            "direct-model",
            ProviderProtocol::ChatCompletions,
            "sk-direct-only",
        );
        let (direct_status, direct_content_type, direct_stream) = invoke_proxy_raw(
            &state,
            Engine::OpenClaw,
            ProviderProtocol::ChatCompletions,
            json!({"model":"wrong","messages":[{"role":"user","content":"test"}],"stream":true}),
        )
        .await;
        assert_eq!(direct_status, StatusCode::OK);
        assert_eq!(direct_content_type.as_deref(), Some("text/event-stream"));
        assert!(
            String::from_utf8(direct_stream.to_vec())
                .unwrap()
                .contains("direct-ok")
        );
        assert!(observations.lock().unwrap().iter().any(|entry| {
            entry == "/direct/v1/chat/completions|Bearer sk-direct-only||direct-model"
        }));
        server.abort();
    }
}
