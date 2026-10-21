use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use getrandom::fill;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use crate::paths::WorkstationPaths;
use crate::vault::PortableVault;
use crate::windows_process::{
    ProcessJob, configure_command, force_terminate_job, process_is_alive, replace_file_atomic,
};

const PROXY_SCHEMA_VERSION: u32 = 1;
const MIHOMO_VERSION: &str = "1.19.29";
// 官方发布页给出的是 ZIP 资产哈希；运行时还需固定从该已验证 ZIP 解出的 EXE 哈希。
const MIHOMO_BINARY_SHA256: &str =
    "82cd796a23492f43a71c1ec27e4e5e0b3d58932014da5a36e79ed9b11fee8162";
const SUBSCRIPTION_URL_REF: &str = "codex-proxy-subscription-url-v1";
const PROVIDER_CACHE_REF: &str = "codex-proxy-provider-cache-v1";
const STATE_FILE: &str = "codex-proxy.json";
const ONCE_MARKER_FILE: &str = "codex-proxy-once.json";
const ACTIVE_STATE_FILE: &str = "active.json";
const OLD_PROXY_CONFIG_FILE: &str = "codex-init-proxy.json";
const OLD_PROXY_MARKER_FILE: &str = "codex-init-proxy-once.json";
const MAX_PROVIDER_CACHE_BYTES: usize = 8 * 1024 * 1024;
const CONTROLLER_WAIT: Duration = Duration::from_secs(12);
const PROVIDER_WAIT: Duration = Duration::from_secs(35);
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const HEALTHCHECK_URL: &str = "https://www.gstatic.com/generate_204";
const CODEX_BOOTSTRAP_PROBE_URL: &str = "https://ab.chatgpt.com/v1/initialize";
const CODEX_BOOTSTRAP_PROBE_TIMEOUT_MS: &str = "7000";
const PROXY_GROUP: &str = "YunshuCodex";
const PROVIDER_NAME: &str = "yunshu";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexProxyRegion {
    All,
    HongKong,
    Japan,
    Singapore,
    UnitedStates,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyNode {
    pub name: String,
    pub region: CodexProxyRegion,
    pub latency_ms: Option<u64>,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyStatus {
    pub configured: bool,
    pub subscription_masked: Option<String>,
    pub subscription_name: Option<String>,
    pub updated_at_unix_ms: Option<u64>,
    pub node_count: usize,
    pub nodes: Vec<CodexProxyNode>,
    pub selected_node: Option<String>,
    pub selected_region: Option<CodexProxyRegion>,
    pub persistent_enabled: bool,
    pub pending_once: bool,
    pub active: bool,
    pub active_mode: Option<CodexProxyLaunchMode>,
    pub core_pid: Option<u32>,
    pub mihomo_version: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxySubscriptionScanInput {
    pub subscription_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyNodeSelectInput {
    pub node_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexProxyLaunchMode {
    Once,
    Persistent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredCodexProxyState {
    schema_version: u32,
    subscription_name: Option<String>,
    updated_at_unix_ms: Option<u64>,
    nodes: Vec<CodexProxyNode>,
    selected_node: Option<String>,
    selected_region: Option<CodexProxyRegion>,
    persistent_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnceMarker {
    schema_version: u32,
    created_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveProxyState {
    schema_version: u32,
    pid: u32,
    #[serde(default)]
    executable: PathBuf,
    mode: CodexProxyLaunchMode,
    mixed_port: u16,
    started_at_unix_ms: u64,
}

pub struct CodexProxySession {
    child: Child,
    job: Option<ProcessJob>,
    executable: PathBuf,
    runtime_dir: PathBuf,
    active_state: PathBuf,
    mixed_port: u16,
    stopped: bool,
}

impl CodexProxySession {
    pub fn launch_environment(&self) -> Vec<(OsString, OsString)> {
        let value = OsString::from(format!("http://127.0.0.1:{}", self.mixed_port));
        let no_proxy = OsString::from("127.0.0.1,localhost,::1");
        vec![
            (OsString::from("HTTP_PROXY"), value.clone()),
            (OsString::from("HTTPS_PROXY"), value.clone()),
            (OsString::from("ALL_PROXY"), value.clone()),
            (OsString::from("http_proxy"), value.clone()),
            (OsString::from("https_proxy"), value.clone()),
            (OsString::from("all_proxy"), value),
            (OsString::from("NO_PROXY"), no_proxy.clone()),
            (OsString::from("no_proxy"), no_proxy),
        ]
    }

    pub fn chromium_proxy_argument(&self) -> OsString {
        OsString::from(format!(
            "--proxy-server=http://127.0.0.1:{}",
            self.mixed_port
        ))
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(job) = self.job.take() {
            let _ = force_terminate_job(&job, 0);
            drop(job);
        }
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.active_state);
        let _ = fs::remove_dir_all(&self.runtime_dir);
        self.stopped = true;
    }
}

impl Drop for CodexProxySession {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn status(paths: &WorkstationPaths) -> Result<CodexProxyStatus> {
    let state = read_state(paths)?;
    let vault = paths
        .vault_file()
        .is_file()
        .then(|| PortableVault::open(paths))
        .transpose()?;
    let subscription = vault
        .as_ref()
        .and_then(|vault| vault.get(SUBSCRIPTION_URL_REF).ok());
    let cache_present = vault
        .as_ref()
        .is_some_and(|vault| vault.contains(PROVIDER_CACHE_REF));
    let active = read_active_state(paths)?;
    Ok(CodexProxyStatus {
        configured: subscription.is_some() && cache_present && state.selected_node.is_some(),
        subscription_masked: subscription
            .as_deref()
            .map(|value| mask_subscription_url(value.as_str())),
        subscription_name: state.subscription_name,
        updated_at_unix_ms: state.updated_at_unix_ms,
        node_count: state.nodes.len(),
        nodes: state.nodes,
        selected_node: state.selected_node,
        selected_region: state.selected_region,
        persistent_enabled: state.persistent_enabled,
        pending_once: once_marker_path(paths).is_file(),
        active: active.is_some(),
        active_mode: active.as_ref().map(|item| item.mode),
        core_pid: active.map(|item| item.pid),
        mihomo_version: MIHOMO_VERSION,
    })
}

pub fn scan_subscription(
    paths: &WorkstationPaths,
    input: CodexProxySubscriptionScanInput,
) -> Result<CodexProxyStatus> {
    paths.prepare_engine_layout(crate::model::Engine::Codex)?;
    let subscription = resolve_subscription_url(paths, input.subscription_url)?;
    let subscription_name = subscription
        .host_str()
        .unwrap_or("Clash/Mihomo 订阅")
        .to_string();
    let mut transient = start_core(paths, CoreSource::Subscription(&subscription))?;
    let provider = wait_for_provider(&transient.controller, PROVIDER_WAIT)?;
    if provider_nodes(&provider).is_empty() {
        bail!("订阅没有返回可用节点");
    }
    let provider = wait_for_health_results(&transient.controller, PROVIDER_WAIT)?;
    let nodes = parse_provider_nodes(&provider);
    if nodes.is_empty() {
        bail!("订阅格式不兼容，未读取到 Clash/Mihomo 节点");
    }
    let provider_cache = wait_for_provider_cache(&transient.provider_path, Duration::from_secs(8))?;
    if provider_cache.len() > MAX_PROVIDER_CACHE_BYTES {
        bail!("订阅节点缓存超过 8MB，拒绝写入便携凭据库");
    }
    transient.session.stop();

    let mut state = read_state(paths)?;
    state.selected_node = preferred_selected_node(state.selected_node.as_deref(), &nodes);
    state.selected_region = state
        .selected_node
        .as_ref()
        .and_then(|selected| nodes.iter().find(|node| &node.name == selected))
        .map(|node| node.region.clone());
    state.schema_version = PROXY_SCHEMA_VERSION;
    state.subscription_name = Some(subscription_name);
    state.updated_at_unix_ms = Some(now_unix_ms());
    state.nodes = nodes;

    let mut vault = PortableVault::open(paths)?;
    vault.set(
        SUBSCRIPTION_URL_REF,
        Zeroizing::new(subscription.as_str().to_string()),
    )?;
    vault.set(
        PROVIDER_CACHE_REF,
        Zeroizing::new(BASE64_STANDARD.encode(provider_cache)),
    )?;
    vault.save(paths)?;
    write_state(paths, &state)?;
    retire_old_manual_proxy(paths)?;
    status(paths)
}

pub fn select_node(
    paths: &WorkstationPaths,
    input: CodexProxyNodeSelectInput,
) -> Result<CodexProxyStatus> {
    let node_name = input.node_name.trim();
    if node_name.is_empty() {
        bail!("节点名称不能为空");
    }
    let mut state = read_state(paths)?;
    let node = state
        .nodes
        .iter()
        .find(|node| node.name == node_name)
        .context("所选节点不在当前订阅中，请重新扫描")?;
    state.selected_node = Some(node.name.clone());
    state.selected_region = Some(node.region.clone());
    write_state(paths, &state)?;
    status(paths)
}

pub fn mark_start_once(paths: &WorkstationPaths) -> Result<CodexProxyStatus> {
    let current = status(paths)?;
    if current.persistent_enabled {
        bail!("持续代理已启用，请先停用后再使用“仅本次”");
    }
    ensure_ready(&current)?;
    write_json_atomic(
        &once_marker_path(paths),
        &OnceMarker {
            schema_version: PROXY_SCHEMA_VERSION,
            created_at_unix_ms: now_unix_ms(),
        },
    )?;
    status(paths)
}

pub fn enable_persistent(paths: &WorkstationPaths) -> Result<CodexProxyStatus> {
    let current = status(paths)?;
    ensure_ready(&current)?;
    let mut state = read_state(paths)?;
    state.persistent_enabled = true;
    write_state(paths, &state)?;
    remove_if_exists(&once_marker_path(paths))?;
    status(paths)
}

pub fn disable(paths: &WorkstationPaths) -> Result<CodexProxyStatus> {
    let mut state = read_state(paths)?;
    state.persistent_enabled = false;
    write_state(paths, &state)?;
    remove_if_exists(&once_marker_path(paths))?;
    status(paths)
}

pub fn clear(paths: &WorkstationPaths) -> Result<CodexProxyStatus> {
    if read_active_state(paths)?.is_some() {
        bail!("请先停止 Codex，再清除订阅代理数据");
    }
    remove_if_exists(&state_path(paths))?;
    remove_if_exists(&once_marker_path(paths))?;
    if paths.vault_file().is_file() {
        let mut vault = PortableVault::open(paths)?;
        let removed = vault.remove(SUBSCRIPTION_URL_REF) | vault.remove(PROVIDER_CACHE_REF);
        if removed {
            vault.save(paths)?;
        }
    }
    status(paths)
}

pub fn prepare_codex_launch(paths: &WorkstationPaths) -> Result<Option<CodexProxySession>> {
    let mode = requested_launch_mode(paths)?;
    let Some(mode) = mode else {
        return Ok(None);
    };
    let current = status(paths)?;
    ensure_ready(&current)?;
    let state = read_state(paths)?;
    let selected = state.selected_node.context("请先选择 Codex 代理节点")?;
    let cache = load_provider_cache(paths)?;
    let started = start_core(paths, CoreSource::Cached(&cache))?;
    wait_for_provider(&started.controller, CONTROLLER_WAIT)?;
    select_running_node(&started.controller, &selected)?;
    verify_codex_bootstrap_route(&started.controller)?;
    let active = ActiveProxyState {
        schema_version: PROXY_SCHEMA_VERSION,
        pid: started.session.child.id(),
        executable: started.session.executable.clone(),
        mode,
        mixed_port: started.session.mixed_port,
        started_at_unix_ms: now_unix_ms(),
    };
    write_json_atomic(&active_state_path(paths), &active)?;
    Ok(Some(started.session))
}

fn requested_launch_mode(paths: &WorkstationPaths) -> Result<Option<CodexProxyLaunchMode>> {
    let state = read_state(paths)?;
    if state.persistent_enabled {
        remove_if_exists(&once_marker_path(paths))?;
        return Ok(Some(CodexProxyLaunchMode::Persistent));
    }
    if once_marker_path(paths).is_file() {
        remove_if_exists(&once_marker_path(paths))?;
        return Ok(Some(CodexProxyLaunchMode::Once));
    }
    Ok(None)
}

fn ensure_ready(status: &CodexProxyStatus) -> Result<()> {
    if !status.configured {
        bail!("请先扫描订阅并选择节点");
    }
    Ok(())
}

enum CoreSource<'a> {
    Subscription(&'a Url),
    Cached(&'a [u8]),
}

struct StartedCore {
    session: CodexProxySession,
    controller: Controller,
    provider_path: PathBuf,
}

#[derive(Clone)]
struct Controller {
    port: u16,
    secret: String,
}

fn start_core(paths: &WorkstationPaths, source: CoreSource<'_>) -> Result<StartedCore> {
    let core = stage_and_verify_core(paths)?;
    let mixed_port = available_loopback_port()?;
    let mut controller_port = available_loopback_port()?;
    while controller_port == mixed_port {
        controller_port = available_loopback_port()?;
    }
    let secret = random_hex(32)?;
    let runtime_dir =
        proxy_runtime_root(paths).join(format!("{}-{}", now_unix_ms(), random_hex(6)?));
    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("无法创建 Mihomo 临时目录 {}", runtime_dir.display()))?;
    let provider_path = runtime_dir.join("provider.yaml");
    if let CoreSource::Cached(bytes) = source {
        fs::write(&provider_path, bytes)
            .with_context(|| format!("无法写入 Mihomo 临时节点缓存 {}", provider_path.display()))?;
    }
    let config_path = runtime_dir.join("config.yaml");
    let config = build_mihomo_config(
        mixed_port,
        controller_port,
        &secret,
        &provider_path,
        match source {
            CoreSource::Subscription(url) => Some(url),
            CoreSource::Cached(_) => None,
        },
    )?;
    fs::write(&config_path, config)
        .with_context(|| format!("无法写入 Mihomo 临时配置 {}", config_path.display()))?;

    let mut command = Command::new(&core);
    command
        .arg("-d")
        .arg(&runtime_dir)
        .arg("-f")
        .arg(&config_path)
        .current_dir(&runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_command(&mut command, false);
    let mut child = command.spawn().context("无法启动 Codex 专用 Mihomo 核心")?;
    let job = ProcessJob::attach(&child).map_err(|error| {
        let _ = child.kill();
        let _ = child.wait();
        error
    })?;
    let mut session = CodexProxySession {
        child,
        job: Some(job),
        executable: core,
        runtime_dir,
        active_state: active_state_path(paths),
        mixed_port,
        stopped: false,
    };
    let controller = Controller {
        port: controller_port,
        secret,
    };
    if let Err(error) = wait_for_controller(&controller, &mut session, CONTROLLER_WAIT) {
        session.stop();
        return Err(error);
    }
    Ok(StartedCore {
        session,
        controller,
        provider_path,
    })
}

fn build_mihomo_config(
    mixed_port: u16,
    controller_port: u16,
    secret: &str,
    provider_path: &Path,
    subscription: Option<&Url>,
) -> Result<String> {
    let provider_path = serde_json::to_string(&provider_path.display().to_string())?;
    let secret = serde_json::to_string(secret)?;
    // 扫描订阅时节点尚未加载，DNS 不能反向依赖节点组，否则 Provider 会永久等待。
    // 使用已缓存节点启动 Codex 时，才让目标域名解析跟随 Codex 专用节点。
    let dns_config = if subscription.is_some() {
        String::new()
    } else {
        format!(
            r#"dns:
  enable: true
  ipv6: false
  use-hosts: false
  use-system-hosts: false
  enhanced-mode: redir-host
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - "https://1.1.1.1/dns-query#{PROXY_GROUP}"
    - "https://8.8.8.8/dns-query#{PROXY_GROUP}"
  proxy-server-nameserver:
    - "https://223.5.5.5/dns-query"
    - "https://120.53.53.53/dns-query"
"#
        )
    };
    let provider_source = if let Some(subscription) = subscription {
        format!(
            "    type: http\n    url: {}\n    path: {provider_path}\n    interval: 86400\n",
            serde_json::to_string(subscription.as_str())?
        )
    } else {
        format!("    type: file\n    path: {provider_path}\n")
    };
    Ok(format!(
        r#"mixed-port: {mixed_port}
bind-address: "127.0.0.1"
allow-lan: false
mode: rule
log-level: silent
ipv6: false
external-controller: "127.0.0.1:{controller_port}"
secret: {secret}
tun:
  enable: false
{dns_config}
proxy-providers:
  {PROVIDER_NAME}:
{provider_source}    health-check:
      enable: true
      lazy: true
      url: "{HEALTHCHECK_URL}"
      interval: 86400
      timeout: 5000
      expected-status: 200-499
proxy-groups:
  - name: "{PROXY_GROUP}"
    type: select
    use:
      - {PROVIDER_NAME}
rules:
  - MATCH,{PROXY_GROUP}
"#
    ))
}

fn wait_for_controller(
    controller: &Controller,
    session: &mut CodexProxySession,
    timeout: Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(status) = session.child.try_wait()? {
            bail!("Mihomo 核心在就绪前退出：{status}");
        }
        if controller_request(controller, "GET", "/version", None).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
    bail!("Mihomo 核心在{}s内没有就绪", timeout.as_secs())
}

fn wait_for_provider(controller: &Controller, timeout: Duration) -> Result<Value> {
    let deadline = std::time::Instant::now() + timeout;
    let target = format!("/providers/proxies/{PROVIDER_NAME}");
    while std::time::Instant::now() < deadline {
        if let Ok(body) = controller_request(controller, "GET", &target, None)
            && let Ok(value) = serde_json::from_slice::<Value>(&body)
            && !provider_nodes(&value).is_empty()
        {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!(
        "订阅在{}s内未返回节点，请检查订阅是否过期或格式是否兼容",
        timeout.as_secs()
    )
}

fn wait_for_health_results(controller: &Controller, timeout: Duration) -> Result<Value> {
    let deadline = std::time::Instant::now() + timeout;
    let target = format!("/providers/proxies/{PROVIDER_NAME}");
    let mut last = None;
    while std::time::Instant::now() < deadline {
        if let Ok(body) = controller_request(controller, "GET", &target, None)
            && let Ok(value) = serde_json::from_slice::<Value>(&body)
        {
            let ready = provider_background_health_results_complete(&value);
            last = Some(value);
            if ready {
                break;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    last.context("节点延迟检测没有返回结果")
}

fn provider_background_health_results_complete(provider: &Value) -> bool {
    let nodes = provider_nodes(provider);
    !nodes.is_empty()
        && nodes.iter().all(|node| {
            node.get("history")
                .and_then(Value::as_array)
                .is_some_and(|history| !history.is_empty())
        })
}

fn provider_nodes(value: &Value) -> &[Value] {
    value
        .get("proxies")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn parse_provider_nodes(provider: &Value) -> Vec<CodexProxyNode> {
    provider_nodes(provider)
        .iter()
        .filter_map(|value| {
            let name = value.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let latency_ms = value
                .get("history")
                .and_then(Value::as_array)
                .and_then(|history| history.last())
                .and_then(|entry| entry.get("delay"))
                .and_then(Value::as_u64)
                .filter(|delay| *delay > 0);
            let alive = value.get("alive").and_then(Value::as_bool).unwrap_or(false);
            Some(CodexProxyNode {
                name: name.to_string(),
                region: classify_region(name),
                latency_ms,
                available: alive && latency_ms.is_some(),
            })
        })
        .collect()
}

fn preferred_selected_node(previous: Option<&str>, nodes: &[CodexProxyNode]) -> Option<String> {
    previous
        .and_then(|selected| {
            nodes
                .iter()
                .find(|node| node.name == selected && node.available)
        })
        .or_else(|| nodes.iter().find(|node| node.available))
        .or_else(|| nodes.first())
        .map(|node| node.name.clone())
}

fn select_running_node(controller: &Controller, name: &str) -> Result<()> {
    let target = controller_path(&["proxies", PROXY_GROUP], None)?;
    let body = serde_json::to_vec(&serde_json::json!({ "name": name }))?;
    controller_request(controller, "PUT", &target, Some(&body))?;
    Ok(())
}

fn codex_bootstrap_probe_path() -> Result<String> {
    controller_path(
        &["proxies", PROXY_GROUP, "delay"],
        Some(&[
            ("url", CODEX_BOOTSTRAP_PROBE_URL),
            ("timeout", CODEX_BOOTSTRAP_PROBE_TIMEOUT_MS),
            ("expected", "200-499"),
        ]),
    )
}

fn verify_codex_bootstrap_route(controller: &Controller) -> Result<()> {
    let target = codex_bootstrap_probe_path()?;
    let body = controller_request(controller, "GET", &target, None).map_err(|error| {
        anyhow!(
            "所选节点无法连接 Codex 中文初始化服务（ab.chatgpt.com），请更换节点后重试：{error}"
        )
    })?;
    let _delay = serde_json::from_slice::<Value>(&body)
        .context("Codex 中文初始化服务连通性结果无法解析")?
        .get("delay")
        .and_then(Value::as_u64)
        .filter(|delay| *delay > 0)
        .context("所选节点没有返回有效的 Codex 中文初始化服务延迟")?;
    Ok(())
}

fn controller_path(segments: &[&str], query: Option<&[(&str, &str)]>) -> Result<String> {
    let mut url = Url::parse("http://127.0.0.1/")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("无法构造 Mihomo API 路径"))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    if let Some(query) = query {
        url.query_pairs_mut().extend_pairs(query.iter().copied());
    }
    Ok(match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    })
}

fn controller_request(
    controller: &Controller,
    method: &str,
    target: &str,
    body: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], controller.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .context("Mihomo Controller 不可达")?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_TIMEOUT))?;
    let body = body.unwrap_or_default();
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        controller.port,
        controller.secret,
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.write_all(body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    parse_http_response(&response)
}

fn parse_http_response(response: &[u8]) -> Result<Vec<u8>> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .context("Mihomo Controller 返回了无效 HTTP 响应")?;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .context("Mihomo Controller 缺少 HTTP 状态码")?;
    if !(200..300).contains(&status) {
        bail!("Mihomo Controller 请求失败（HTTP {status}）");
    }
    let body = &response[header_end..];
    if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        decode_chunked(body)
    } else {
        Ok(body.to_vec())
    }
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .context("无效的 chunked HTTP 响应")?;
        let size_text = std::str::from_utf8(&input[..line_end])?
            .split(';')
            .next()
            .unwrap_or_default();
        let size = usize::from_str_radix(size_text.trim(), 16)?;
        input = &input[line_end + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size + 2 {
            bail!("chunked HTTP 响应被截断");
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
    Ok(output)
}

fn resolve_subscription_url(paths: &WorkstationPaths, input: Option<String>) -> Result<Url> {
    let candidate = input
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            PortableVault::open(paths)
                .ok()?
                .get(SUBSCRIPTION_URL_REF)
                .ok()
                .map(|value| value.to_string())
        })
        .context("请粘贴 Clash/Mihomo 兼容订阅链接")?;
    let url = Url::parse(&candidate).context("订阅链接格式不正确")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("只支持 HTTP/HTTPS 的 Clash/Mihomo 订阅链接");
    }
    Ok(url)
}

fn mask_subscription_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            let host = parsed.host_str()?;
            Some(format!("{}://{host}/***", parsed.scheme()))
        })
        .unwrap_or_else(|| "已加密保存".to_string())
}

fn classify_region(name: &str) -> CodexProxyRegion {
    let lower = name.to_lowercase();
    let has = |values: &[&str]| values.iter().any(|value| lower.contains(value));
    if has(&["🇭🇰", "香港", "hong kong", "hongkong", " hk", "hk "]) {
        CodexProxyRegion::HongKong
    } else if has(&["🇯🇵", "日本", "japan", "tokyo", "osaka", " jp", "jp "]) {
        CodexProxyRegion::Japan
    } else if has(&["🇸🇬", "新加坡", "singapore", " sg", "sg "]) {
        CodexProxyRegion::Singapore
    } else if has(&[
        "🇺🇸",
        "美国",
        "united states",
        "los angeles",
        "san jose",
        "seattle",
        " us",
        "us ",
    ]) {
        CodexProxyRegion::UnitedStates
    } else {
        CodexProxyRegion::Other
    }
}

fn load_provider_cache(paths: &WorkstationPaths) -> Result<Zeroizing<Vec<u8>>> {
    let vault = PortableVault::open(paths)?;
    let encoded = vault
        .get(PROVIDER_CACHE_REF)
        .context("订阅节点缓存不存在，请重新扫描")?;
    let decoded = BASE64_STANDARD
        .decode(encoded.as_bytes())
        .context("订阅节点缓存无法解密")?;
    if decoded.is_empty() || decoded.len() > MAX_PROVIDER_CACHE_BYTES {
        bail!("订阅节点缓存大小异常，请重新扫描");
    }
    Ok(Zeroizing::new(decoded))
}

fn wait_for_provider_cache(path: &Path, timeout: Duration) -> Result<Vec<u8>> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(bytes) = fs::read(path)
            && !bytes.is_empty()
        {
            return Ok(bytes);
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("Mihomo 没有生成可便携的订阅缓存")
}

fn stage_and_verify_core(paths: &WorkstationPaths) -> Result<PathBuf> {
    let source = paths
        .root
        .join("runtimes")
        .join("mihomo")
        .join("mihomo.exe");
    if !source.is_file() {
        bail!("Codex 专用 Mihomo 核心缺失：{}", source.display());
    }
    let actual = sha256_file(&source)?;
    if !actual.eq_ignore_ascii_case(MIHOMO_BINARY_SHA256) {
        bail!("Mihomo 核心校验失败，拒绝启动");
    }
    let destination = paths
        .machine_cache_root()
        .join("codex-proxy")
        .join("core")
        .join(MIHOMO_BINARY_SHA256)
        .join("mihomo.exe");
    if destination.is_file()
        && sha256_file(&destination)?.eq_ignore_ascii_case(MIHOMO_BINARY_SHA256)
    {
        return Ok(destination);
    }
    let parent = destination.parent().context("Mihomo 本机缓存目录不正确")?;
    fs::create_dir_all(parent)?;
    let temporary = destination.with_extension(format!("exe.{}.tmp", std::process::id()));
    fs::copy(&source, &temporary).with_context(|| {
        format!(
            "无法暂存 Mihomo 核心 {} -> {}",
            source.display(),
            temporary.display()
        )
    })?;
    if !sha256_file(&temporary)?.eq_ignore_ascii_case(MIHOMO_BINARY_SHA256) {
        let _ = fs::remove_file(&temporary);
        bail!("Mihomo 核心暂存后校验失败");
    }
    replace_file_atomic(&temporary, &destination)?;
    Ok(destination)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn available_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("无法分配 Mihomo 本地端口")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut value = vec![0_u8; bytes];
    fill(&mut value).map_err(|error| anyhow!("无法生成 Mihomo 随机密钥：{error}"))?;
    Ok(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn proxy_runtime_root(paths: &WorkstationPaths) -> PathBuf {
    paths
        .machine_cache_root()
        .join("codex-proxy")
        .join("runtime")
}

fn state_path(paths: &WorkstationPaths) -> PathBuf {
    paths.config.join(STATE_FILE)
}

fn once_marker_path(paths: &WorkstationPaths) -> PathBuf {
    paths.runtime_state.join(ONCE_MARKER_FILE)
}

fn active_state_path(paths: &WorkstationPaths) -> PathBuf {
    paths
        .machine_cache_root()
        .join("codex-proxy")
        .join(ACTIVE_STATE_FILE)
}

fn read_state(paths: &WorkstationPaths) -> Result<StoredCodexProxyState> {
    let path = state_path(paths);
    if !path.is_file() {
        return Ok(StoredCodexProxyState {
            schema_version: PROXY_SCHEMA_VERSION,
            ..StoredCodexProxyState::default()
        });
    }
    let state: StoredCodexProxyState = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("无法解析 Codex 代理状态 {}", path.display()))?;
    if state.schema_version != PROXY_SCHEMA_VERSION {
        bail!("Codex 代理状态版本不支持");
    }
    Ok(state)
}

fn write_state(paths: &WorkstationPaths, state: &StoredCodexProxyState) -> Result<()> {
    write_json_atomic(&state_path(paths), state)
}

fn read_active_state(paths: &WorkstationPaths) -> Result<Option<ActiveProxyState>> {
    let path = active_state_path(paths);
    if !path.is_file() {
        return Ok(None);
    }
    let state: ActiveProxyState = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("无法解析 Codex 代理运行状态 {}", path.display()))?;
    if process_is_alive(state.pid) {
        return Ok(Some(state));
    }
    remove_if_exists(&path)?;
    Ok(None)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, path)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("无法删除 {}", path.display()))?;
    }
    Ok(())
}

fn retire_old_manual_proxy(paths: &WorkstationPaths) -> Result<()> {
    remove_if_exists(&paths.config.join(OLD_PROXY_CONFIG_FILE))?;
    remove_if_exists(&paths.config.join(OLD_PROXY_MARKER_FILE))?;
    if paths.vault_file().is_file() {
        let mut vault = PortableVault::open(paths)?;
        if vault.remove("codex-init-proxy-password-v1") {
            vault.save(paths)?;
        }
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_mask_never_contains_path_or_token() {
        let masked = mask_subscription_url("https://example.com/sub?token=secret-value");
        assert_eq!(masked, "https://example.com/***");
        assert!(!masked.contains("secret-value"));
    }

    #[test]
    fn region_classifier_handles_flags_and_names() {
        assert_eq!(classify_region("🇭🇰 香港 01"), CodexProxyRegion::HongKong);
        assert_eq!(classify_region("Tokyo JP 02"), CodexProxyRegion::Japan);
        assert_eq!(classify_region("Singapore SG"), CodexProxyRegion::Singapore);
        assert_eq!(
            classify_region("Los Angeles US"),
            CodexProxyRegion::UnitedStates
        );
        assert_eq!(classify_region("自动选择"), CodexProxyRegion::Other);
    }

    #[test]
    fn mihomo_config_is_loopback_only_without_tun_or_system_proxy() {
        let config = build_mihomo_config(
            41001,
            41002,
            "controller-secret",
            Path::new(r"C:\Temp\provider.yaml"),
            Some(&Url::parse("https://example.com/sub?token=secret").unwrap()),
        )
        .unwrap();
        assert!(config.contains("bind-address: \"127.0.0.1\""));
        assert!(config.contains("allow-lan: false"));
        assert!(config.contains("enable: false"));
        assert!(!config.contains("\ndns:\n"));
        assert!(config.contains("external-controller: \"127.0.0.1:41002\""));
        assert!(config.contains("\n  yunshu:\n    type: http\n"));
        assert!(config.contains("\n    health-check:\n      enable: true\n"));
        assert!(config.contains("url: \"https://www.gstatic.com/generate_204\""));
        assert!(config.contains("expected-status: 200-499"));
        assert!(config.contains(
            "\nproxy-groups:\n  - name: \"YunshuCodex\"\n    type: select\n    use:\n      - yunshu\n"
        ));
        assert!(!config.contains("system-proxy"));
    }

    #[test]
    fn cached_provider_dns_uses_the_codex_group_after_nodes_exist() {
        let config = build_mihomo_config(
            41001,
            41002,
            "controller-secret",
            Path::new(r"C:\Temp\provider.yaml"),
            None,
        )
        .unwrap();
        assert!(config.contains("\ndns:\n  enable: true\n"));
        assert!(config.contains("https://1.1.1.1/dns-query#YunshuCodex"));
        assert!(config.contains("\n  proxy-server-nameserver:\n"));
        assert!(config.contains("\n  yunshu:\n    type: file\n"));
    }

    #[test]
    fn codex_bootstrap_probe_targets_the_official_feature_service() {
        let target = codex_bootstrap_probe_path().unwrap();
        assert!(target.starts_with("/proxies/YunshuCodex/delay?"));
        assert!(target.contains("url=https%3A%2F%2Fab.chatgpt.com%2Fv1%2Finitialize"));
        assert!(target.contains("timeout=7000"));
        assert!(target.contains("expected=200-499"));
    }

    #[test]
    fn node_scan_and_codex_bootstrap_use_separate_targets() {
        let config = build_mihomo_config(
            41001,
            41002,
            "controller-secret",
            Path::new(r"C:\Temp\provider.yaml"),
            Some(&Url::parse("https://example.com/sub").unwrap()),
        )
        .unwrap();

        assert!(config.contains("url: \"https://www.gstatic.com/generate_204\""));
        assert!(!config.contains("url: \"https://ab.chatgpt.com/v1/initialize\""));
        assert!(
            codex_bootstrap_probe_path()
                .unwrap()
                .contains("ab.chatgpt.com")
        );
    }

    #[test]
    fn selected_node_and_mode_are_portable_state() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        paths
            .prepare_engine_layout(crate::model::Engine::Codex)
            .unwrap();
        let state = StoredCodexProxyState {
            schema_version: PROXY_SCHEMA_VERSION,
            subscription_name: Some("example.com".to_string()),
            updated_at_unix_ms: Some(1),
            nodes: vec![CodexProxyNode {
                name: "🇭🇰 香港".to_string(),
                region: CodexProxyRegion::HongKong,
                latency_ms: Some(20),
                available: true,
            }],
            selected_node: Some("🇭🇰 香港".to_string()),
            selected_region: Some(CodexProxyRegion::HongKong),
            persistent_enabled: true,
        };
        write_state(&paths, &state).unwrap();
        let restored = read_state(&paths).unwrap();
        assert_eq!(restored.selected_node.as_deref(), Some("🇭🇰 香港"));
        assert!(restored.persistent_enabled);
    }

    #[test]
    fn scan_replaces_an_incompatible_previous_selection() {
        let nodes = vec![
            CodexProxyNode {
                name: "新加坡-3".to_string(),
                region: CodexProxyRegion::Singapore,
                latency_ms: None,
                available: false,
            },
            CodexProxyNode {
                name: "日本-1".to_string(),
                region: CodexProxyRegion::Japan,
                latency_ms: Some(180),
                available: true,
            },
        ];

        assert_eq!(
            preferred_selected_node(Some("新加坡-3"), &nodes).as_deref(),
            Some("日本-1")
        );
    }

    #[test]
    fn provider_scan_waits_for_all_background_health_results() {
        let provider = serde_json::json!({
            "proxies": [
                { "name": "香港-1", "history": [{ "delay": 81 }] },
                { "name": "日本-1", "history": [] }
            ]
        });

        assert!(!provider_background_health_results_complete(&provider));

        let provider = serde_json::json!({
            "proxies": [
                { "name": "香港-1", "history": [{ "delay": 81 }] },
                { "name": "日本-1", "history": [{ "delay": 0 }] }
            ]
        });

        assert!(provider_background_health_results_complete(&provider));
    }
}
