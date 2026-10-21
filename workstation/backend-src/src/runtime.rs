use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::ZipArchive;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::config::{prepare_clean_configs, prepare_engine_config};
use crate::engine::LaunchSpec;
use crate::engine::{
    build_launch_spec, build_launch_spec_without_provider, build_probe_spec, detect_engine,
};
use crate::model::{Engine, EngineDetection, EngineState, RuntimeStatus};
use crate::paths::{PreparedLayout, WorkstationPaths};
use crate::windows_process::{
    ProcessJob, configure_command, force_terminate_job, open_url_native, probe_process_tree_window,
    process_is_alive, replace_file_atomic, request_graceful_tree_stop, restore_process_tree_window,
    terminate_child_as_fallback,
};

const STATE_SCHEMA_VERSION: u32 = 1;
// Codex 首次启动仍可能建立本机 NTFS 运行镜像；父进程需要等待监督进程完成准备。
const START_WAIT: Duration = Duration::from_secs(300);
// 新电脑首次需要从随盘单文件载荷包建立本机 NTFS 运行体。虚拟 USB
// 与 Defender 可能让 6 万个文件的首次落盘超过 5 分钟；这个等待只覆盖
// 运行体准备。网关进程创建后使用冷/热两档健康检查时限。
const OPENCLAW_START_WAIT: Duration = Duration::from_secs(20 * 60);
// 父进程会先写入 starting，再创建 supervisor。这个无 PID 窗口正常只持续很短时间；
// 超出宽限期仍没有任何受控 PID，就应恢复为 stopped，而不是永久显示“启动中”。
const PARENT_SPAWN_GRACE: Duration = Duration::from_secs(10);
const CODEX_WINDOW_WAIT: Duration = Duration::from_secs(90);
const CODEX_READY_SAMPLE_COUNT: u8 = 2;
const OPENCLAW_HEALTH_WAIT: Duration = Duration::from_secs(120);
const OPENCLAW_COLD_HEALTH_WAIT: Duration = Duration::from_secs(300);
const OPENCLAW_WARM_MARKER: &str = "gateway-warm-ready-v1";
const OPENCLAW_CONTROL_UI_REOPEN_AFTER: Duration = Duration::from_secs(15);
const OPENCLAW_CONTROL_UI_OBSERVE_WAIT: Duration = Duration::from_secs(90);
const STOP_WAIT: Duration = Duration::from_secs(15);
const STOP_WITHOUT_WINDOW_WAIT: Duration = Duration::from_secs(3);
const CODEX_HANDLE_RELEASE_GRACE: Duration = Duration::from_millis(750);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const CODEX_ARG0_CLEANUP_RETRIES: usize = 15;
const CODEX_ARG0_CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(200);
// 启动器状态轮询与 supervisor 最终状态写入可能恰好同时发生。Windows 上原子替换
// 会在极短时间内返回 sharing violation，因此状态文件写入需要做有界重试。
const STATE_WRITE_RETRIES: usize = 12;
const STATE_WRITE_RETRY_DELAY: Duration = Duration::from_millis(25);
const EXTENSIONS_READY_MARKER: &str = "extensions-ready-v1";
const OPENCLAW_PAYLOAD_MANIFEST_NAME: &str = "payload-manifest.json";
const OPENCLAW_PAYLOAD_ARCHIVE_SCHEMA_VERSION: u32 = 1;
const OPENCLAW_PAYLOAD_MAX_FILES: usize = 120_000;
const OPENCLAW_PAYLOAD_MAX_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const OPENCLAW_PAYLOAD_PROGRESS_INTERVAL: usize = 2_000;
static STATE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct OpenClawMachinePayload {
    node: PathBuf,
    entry: PathBuf,
}

fn openclaw_warm_marker(machine_cache: &Path) -> PathBuf {
    machine_cache.join(OPENCLAW_WARM_MARKER)
}

fn openclaw_health_wait(machine_cache: &Path) -> Duration {
    if openclaw_warm_marker(machine_cache).is_file() {
        OPENCLAW_HEALTH_WAIT
    } else {
        OPENCLAW_COLD_HEALTH_WAIT
    }
}

fn latest_openclaw_log(machine_cache: &Path) -> Option<PathBuf> {
    fs::read_dir(machine_cache.join("temp/openclaw"))
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "log"))
        .max_by_key(|path| {
            fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH)
        })
}

fn openclaw_control_ui_checkpoint(machine_cache: &Path) -> (Option<PathBuf>, u64) {
    let path = latest_openclaw_log(machine_cache);
    let len = path
        .as_ref()
        .and_then(|path| fs::metadata(path).ok())
        .map_or(0, |meta| meta.len());
    (path, len)
}

fn openclaw_control_ui_connected_since(
    machine_cache: &Path,
    checkpoint: &(Option<PathBuf>, u64),
) -> bool {
    let Some(path) = latest_openclaw_log(machine_cache) else {
        return false;
    };
    let offset = if checkpoint.0.as_ref() == Some(&path) {
        checkpoint
            .1
            .min(fs::metadata(&path).map_or(0, |meta| meta.len()))
    } else {
        0
    };
    let Ok(mut log) = File::open(path) else {
        return false;
    };
    let mut appended = String::new();
    log.seek(SeekFrom::Start(offset)).is_ok()
        && log.read_to_string(&mut appended).is_ok()
        && appended.contains("client=openclaw-control-ui")
}

fn set_openclaw_running_message(paths: &WorkstationPaths, port: u16, message: &str) {
    if let Ok(mut state) = status(paths, Engine::OpenClaw) {
        if matches!(state.status, RuntimeStatus::Running) && state.loopback_port == Some(port) {
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some(message.to_string());
            let _ = write_state(paths, &state);
        }
    }
}

fn observe_openclaw_control_ui(
    paths: WorkstationPaths,
    port: u16,
    url: String,
    checkpoint: (Option<PathBuf>, u64),
) {
    thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut reopened = false;
        loop {
            if openclaw_control_ui_connected_since(
                &paths.machine_engine_cache("openclaw"),
                &checkpoint,
            ) {
                set_openclaw_running_message(&paths, port, "OpenClaw控制台已连接");
                let _ = append_runtime_event(
                    &paths,
                    Engine::OpenClaw,
                    "control ui connection observed",
                );
                return;
            }
            let still_running = status(&paths, Engine::OpenClaw).is_ok_and(|state| {
                matches!(state.status, RuntimeStatus::Running)
                    && state.loopback_port == Some(port)
                    && state.process_pid.is_some_and(process_is_alive)
            });
            if !still_running {
                return;
            }
            let elapsed = started.elapsed();
            if !reopened && elapsed >= OPENCLAW_CONTROL_UI_REOPEN_AFTER {
                let _ = open_url_native(&url);
                let _ = append_runtime_event(
                    &paths,
                    Engine::OpenClaw,
                    "control ui browser request retried",
                );
                reopened = true;
            }
            if elapsed >= OPENCLAW_CONTROL_UI_OBSERVE_WAIT {
                set_openclaw_running_message(
                    &paths,
                    port,
                    "OpenClaw网关已启动，控制台尚未连接，可点击打开OpenClaw重试",
                );
                return;
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
}

fn apply_openclaw_machine_payload(
    spec: &mut LaunchSpec,
    payload: &OpenClawMachinePayload,
) -> Result<()> {
    if spec.engine != Engine::OpenClaw {
        bail!("OpenClaw本机运行体不能应用到{}", spec.engine);
    }
    spec.program = payload.node.clone();
    let entry = spec
        .args
        .first_mut()
        .context("OpenClaw启动参数缺少程序入口")?;
    *entry = payload.entry.as_os_str().to_os_string();
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenClawPayloadManifest {
    schema_version: u32,
    fingerprint: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawPayloadPackResult {
    pub archive: String,
    pub fingerprint: String,
    pub file_count: usize,
    pub uncompressed_bytes: u64,
    pub archive_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareResult {
    pub layout: PreparedLayout,
    pub detections: Vec<EngineDetection>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub engine: Engine,
    pub program: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorCheckResult {
    pub engine: Engine,
    pub executable: String,
    pub build_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseResult {
    pub root: String,
    pub portable_data: String,
    pub workspace: String,
    pub detections: Vec<EngineDetection>,
    pub states: Vec<EngineState>,
    pub codex_config_exists: bool,
    pub claude_config_exists: bool,
    pub openclaw_config_exists: bool,
    pub codex_locale: crate::locale::CodexLocaleStatus,
}

pub fn prepare(paths: &WorkstationPaths) -> Result<PrepareResult> {
    let layout = paths.prepare_layout()?;
    paths.ensure_payload_exists()?;
    prepare_clean_configs(paths)?;
    crate::skills::prepare(paths)?;
    crate::plugins::prepare(paths)?;
    write_extensions_ready_marker(paths)?;
    Ok(PrepareResult {
        layout,
        detections: Engine::ALL
            .into_iter()
            .map(|engine| detect_engine(paths, engine))
            .collect(),
    })
}

/// 引擎每次启动只执行当前引擎的轻量目录、主体和配置校验。
/// Skills、插件和其他引擎均由各自页面或维护命令按需准备，不能进入启动关键路径。
fn prepare_for_launch(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    paths.prepare_engine_layout(engine)?;
    let detection = detect_engine(paths, engine);
    if !detection.ready {
        let missing = detection
            .missing_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("；");
        bail!("{engine}主体不完整：{missing}");
    }
    prepare_engine_config(paths, engine)?;
    Ok(())
}

/// 启动器进程只做只读主体检查。真正的目录和配置整理只在 supervisor 中执行一次，
/// 避免启动器与 sidecar 对同一份移动盘 config.toml 重复读写。
fn preflight_for_launch(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    let detection = detect_engine(paths, engine);
    if !detection.ready {
        let missing = detection
            .missing_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("；");
        bail!("{engine}主体不完整：{missing}");
    }
    Ok(())
}

fn extensions_ready_marker(paths: &WorkstationPaths) -> PathBuf {
    paths.runtime_state.join(EXTENSIONS_READY_MARKER)
}

fn write_extensions_ready_marker(paths: &WorkstationPaths) -> Result<()> {
    fs::create_dir_all(&paths.runtime_state)?;
    fs::write(extensions_ready_marker(paths), b"1\n").context("无法写入Skills/插件准备标记")
}

pub fn detect(paths: &WorkstationPaths, engine: Option<Engine>) -> Vec<EngineDetection> {
    match engine {
        Some(engine) => vec![detect_engine(paths, engine)],
        None => Engine::ALL
            .into_iter()
            .map(|engine| detect_engine(paths, engine))
            .collect(),
    }
}

pub fn start(paths: &WorkstationPaths, engine: Engine) -> Result<EngineState> {
    preflight_for_launch(paths, engine)?;
    // 先建立稳定 installation-id，确保启动器和 supervisor 使用同一个本机状态目录。
    paths.prepare_engine_layout(engine)?;
    if engine == Engine::Claude {
        return start_claude_console(paths);
    }
    let mut current = status(paths, engine)?;
    let process_alive = current.process_pid.is_some_and(process_is_alive);
    let supervisor_alive = current.supervisor_pid != 0 && process_is_alive(current.supervisor_pid);
    if process_alive && supervisor_alive {
        if engine == Engine::OpenClaw {
            let port = current
                .loopback_port
                .context("OpenClaw运行状态缺少本地网关端口")?;
            if !probe_openclaw_health(port) {
                bail!("OpenClaw进程存在，但网关健康检查未通过；请先停止网关后重试");
            }
            current.status = RuntimeStatus::Running;
            current.window_ready = true;
            current.updated_at_unix_ms = now_unix_ms();
            current.message = Some(format!("OpenClaw网关已就绪，Control UI端口为{port}"));
            crate::config::set_openclaw_gateway_port(paths, port)?;
            write_state(paths, &current)?;
            return Ok(current);
        }
        if current.window_ready {
            current.status = RuntimeStatus::Running;
            current.updated_at_unix_ms = now_unix_ms();
            current.message = Some("Codex窗口已出现并且可以响应".to_string());
            write_state(paths, &current)?;
            return Ok(current);
        }
        if let Some(process_id) = current.process_pid {
            let _ = restore_process_tree_window(process_id);
        }
        current.status = RuntimeStatus::Starting;
        current.updated_at_unix_ms = now_unix_ms();
        current.message = Some("Codex后台进程存在，正在等待窗口出现".to_string());
        write_state(paths, &current)?;
        return wait_for_running_state(paths, engine, current.supervisor_pid);
    }
    if process_alive && !supervisor_alive {
        bail!("检测到{engine}进程仍在运行，但监督进程已经退出；请先安全停止后重试");
    }
    if supervisor_alive && !process_alive {
        if matches!(current.status, RuntimeStatus::Starting) {
            // OpenClaw 首次准备本机载荷时，supervisor 会先运行而网关进程尚未创建。
            // 重复点击只等待现有启动任务，禁止再创建第二份载荷复制任务。
            return wait_for_running_state(paths, engine, current.supervisor_pid);
        }
        bail!("{engine}监督进程正在清理，请稍后重试");
    }

    remove_if_exists(&paths.stop_request_file(engine.slug()))?;
    let started_at = now_unix_ms();
    let executable = supervisor_executable(paths, engine).map_err(|error| {
        record_parent_start_failure(paths, engine, started_at, "无法准备本机监督后端", error)
    })?;
    verify_supervisor_build(paths, &executable).map_err(|error| {
        record_parent_start_failure(paths, engine, started_at, "本机监督后端版本校验失败", error)
    })?;
    let mut starting = EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine,
        status: RuntimeStatus::Starting,
        supervisor_pid: 0,
        process_pid: None,
        executable: detect_engine(paths, engine).executable,
        started_at_unix_ms: started_at,
        updated_at_unix_ms: started_at,
        loopback_port: None,
        window_ready: false,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some("正在创建监督进程".to_string()),
    };
    let _ = append_runtime_event(paths, engine, "parent state write begin");
    write_state(paths, &starting)?;
    let _ = append_runtime_event(paths, engine, "parent state write complete");

    let mut command = Command::new(executable);
    command
        .arg("--root")
        .arg(&paths.root)
        .arg("supervise")
        .arg(engine.slug())
        .current_dir(&paths.backend)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_command(&mut command, false);
    let _ = append_runtime_event(paths, engine, "parent supervisor spawn begin");
    let supervisor = match command.spawn() {
        Ok(supervisor) => {
            let _ = append_runtime_event(
                paths,
                engine,
                &format!("parent supervisor spawn complete pid={}", supervisor.id()),
            );
            supervisor
        }
        Err(error) => {
            let message = format!("无法启动{engine}监督进程：{error}");
            let _ = append_runtime_event(
                paths,
                engine,
                &format!("parent supervisor spawn failed error={error}"),
            );
            starting.status = RuntimeStatus::Failed;
            starting.updated_at_unix_ms = now_unix_ms();
            starting.message = Some(message.clone());
            write_state(paths, &starting)?;
            return Err(error).context(message);
        }
    };
    if read_state(paths, engine)?
        .is_none_or(|state| matches!(state.status, RuntimeStatus::Starting))
    {
        starting.supervisor_pid = supervisor.id();
        starting.updated_at_unix_ms = now_unix_ms();
        starting.message = Some("监督进程已启动，正在创建引擎进程".to_string());
        write_state(paths, &starting)?;
    }

    wait_for_running_state(paths, engine, supervisor.id())
}

fn start_claude_console(paths: &WorkstationPaths) -> Result<EngineState> {
    let current = status(paths, Engine::Claude)?;
    let process_alive = current.process_pid.is_some_and(process_is_alive);
    let supervisor_alive = current.supervisor_pid != 0 && process_is_alive(current.supervisor_pid);
    if process_alive && supervisor_alive {
        return Ok(current);
    }
    if process_alive && !supervisor_alive {
        bail!("检测到Claude Code仍在运行，但交互控制台监督进程已经退出；请先安全停止后重试");
    }
    if supervisor_alive {
        bail!("Claude Code交互控制台正在清理，请稍后重试");
    }

    remove_if_exists(&paths.stop_request_file(Engine::Claude.slug()))?;
    let started_at = now_unix_ms();
    let executable = supervisor_executable(paths, Engine::Claude).map_err(|error| {
        record_parent_start_failure(
            paths,
            Engine::Claude,
            started_at,
            "无法准备本机监督后端",
            error,
        )
    })?;
    verify_supervisor_build(paths, &executable).map_err(|error| {
        record_parent_start_failure(
            paths,
            Engine::Claude,
            started_at,
            "本机监督后端版本校验失败",
            error,
        )
    })?;
    let mut starting = EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine: Engine::Claude,
        status: RuntimeStatus::Starting,
        supervisor_pid: 0,
        process_pid: None,
        executable: detect_engine(paths, Engine::Claude).executable,
        started_at_unix_ms: started_at,
        updated_at_unix_ms: started_at,
        loopback_port: None,
        window_ready: false,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some("正在创建Claude Code交互控制台".to_string()),
    };
    write_state(paths, &starting)?;

    let mut command = Command::new(executable);
    command
        .arg("--root")
        .arg(&paths.root)
        .arg("console")
        .arg(Engine::Claude.slug())
        .current_dir(&paths.backend);
    // 不给新控制台设置空的标准句柄。Windows 会为这个 sidecar 分配真实控制台，
    // sidecar 再把这些句柄传给 Claude Code，避免出现空白黑框后立即退出。
    configure_command(&mut command, true);
    let supervisor = command.spawn().context("无法创建Claude Code交互控制台")?;
    starting.supervisor_pid = supervisor.id();
    starting.updated_at_unix_ms = now_unix_ms();
    starting.message = Some("Claude Code交互控制台已创建，正在等待CLI就绪".to_string());
    write_state(paths, &starting)?;

    wait_for_running_state(paths, Engine::Claude, supervisor.id())
}

fn wait_for_running_state(
    paths: &WorkstationPaths,
    engine: Engine,
    supervisor_pid: u32,
) -> Result<EngineState> {
    let wait = if engine == Engine::OpenClaw {
        OPENCLAW_START_WAIT
    } else {
        START_WAIT
    };
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        let state = status(paths, engine)?;
        if matches!(state.status, RuntimeStatus::Running) {
            return Ok(state);
        }
        if matches!(state.status, RuntimeStatus::Failed) {
            bail!(
                "{}",
                state.message.unwrap_or_else(|| format!("{engine}启动失败"))
            );
        }
        if matches!(state.status, RuntimeStatus::Stopped) {
            bail!(
                "{}",
                state
                    .message
                    .unwrap_or_else(|| format!("{engine}启动进程已经退出"))
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
    bail!(
        "{engine}启动超时（{}秒），监督进程PID={supervisor_pid}",
        wait.as_secs()
    )
}

fn supervisor_executable(paths: &WorkstationPaths, engine: Engine) -> Result<std::path::PathBuf> {
    let windows_sidecar = paths.backend.join("workstation-runtime.exe");
    if windows_sidecar.is_file() {
        return stage_supervisor_executable(paths, engine, &windows_sidecar);
    }
    let unix_sidecar = paths.backend.join("workstation-runtime");
    if unix_sidecar.is_file() {
        return Ok(unix_sidecar);
    }
    let current = std::env::current_exe().context("无法获取启动后端路径")?;
    let file_name = current
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.starts_with("workstation-runtime") {
        return Ok(current);
    }
    bail!("缺少受控启动后端：{}", windows_sidecar.display())
}

/// 把监督后端复制到本机 NTFS 后再启动。
///
/// Windows 在移动盘上同时运行启动器并再次创建同一个 Runtime 进程时，可能长时间
/// 阻塞在 CreateProcess，界面只会停在“正在创建监督进程”。运行时主体仍以移动盘
/// 版本为准，但真正执行的监督副本按源码 Build ID 和引擎放在本机缓存中。同一个
/// Build ID 的副本是不可变文件：存在时直接复用，避免第二个引擎启动时覆盖第一个
/// 引擎仍在执行的 EXE。随后仍由 `verify_supervisor_build` 校验版本指纹。
fn stage_supervisor_executable(
    paths: &WorkstationPaths,
    engine: Engine,
    source: &Path,
) -> Result<PathBuf> {
    let runtime_root = paths.machine_cache_root().join("runtime");
    let _ = cleanup_legacy_supervisor_temporaries(&runtime_root);
    stage_supervisor_binary(source, &runtime_root, engine)
}

fn stage_supervisor_binary(source: &Path, runtime_root: &Path, engine: Engine) -> Result<PathBuf> {
    let destination_root = runtime_root
        .join(crate::RUNTIME_BUILD_ID)
        .join(engine.slug());
    fs::create_dir_all(&destination_root)
        .with_context(|| format!("无法创建本机监督后端目录 {}", destination_root.display()))?;
    let destination = destination_root.join("workstation-runtime.exe");
    if supervisor_copy_is_reusable(source, &destination)? {
        return Ok(destination);
    }
    let temporary = destination.with_extension(format!(
        "exe.{}.{}.tmp",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<PathBuf> {
        fs::copy(source, &temporary).with_context(|| {
            format!(
                "无法暂存监督后端 {} -> {}",
                source.display(),
                temporary.display()
            )
        })?;
        match replace_file_atomic(&temporary, &destination) {
            Ok(()) => Ok(destination.clone()),
            Err(_error) if supervisor_copy_is_reusable(source, &destination)? => {
                // 另一个并发启动请求已经完成同一个不可变副本；直接复用即可。
                Ok(destination.clone())
            }
            Err(error) => Err(error)
                .with_context(|| format!("无法启用本机监督后端 {}", destination.display())),
        }
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn supervisor_copy_is_reusable(source: &Path, destination: &Path) -> Result<bool> {
    if !destination.is_file() {
        return Ok(false);
    }
    let source_length = fs::metadata(source)
        .with_context(|| format!("无法读取监督后端源文件属性 {}", source.display()))?
        .len();
    let destination_length = fs::metadata(destination)
        .with_context(|| format!("无法读取本机监督后端属性 {}", destination.display()))?
        .len();
    if source_length != destination_length {
        return Ok(false);
    }
    Ok(file_sha256(source)? == file_sha256(destination)?)
}

fn file_sha256(path: &Path) -> Result<[u8; 32]> {
    let mut file =
        File::open(path).with_context(|| format!("无法打开 {} 计算哈希", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("无法读取 {} 计算哈希", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn cleanup_legacy_supervisor_temporaries(runtime_root: &Path) -> Result<usize> {
    if !runtime_root.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(runtime_root)
        .with_context(|| format!("无法读取本机监督后端目录 {}", runtime_root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("workstation-runtime-")
            && name.contains(".exe.")
            && name.ends_with(".tmp")
        {
            fs::remove_file(&path)
                .with_context(|| format!("无法清理旧监督后端临时文件 {}", path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn verify_supervisor_build(paths: &WorkstationPaths, executable: &Path) -> Result<()> {
    let mut command = Command::new(executable);
    command
        .arg("--root")
        .arg(&paths.root)
        .arg("build-id")
        .current_dir(&paths.backend)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    configure_command(&mut command, false);
    let output = command
        .output()
        .with_context(|| format!("无法读取外部 Runtime 版本：{}", executable.display()))?;
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || actual != crate::RUNTIME_BUILD_ID {
        bail!(
            "启动器与外部 Runtime 版本不一致：launcher={} runtime={}；请重新完整部署",
            crate::RUNTIME_BUILD_ID,
            if actual.is_empty() {
                "unknown"
            } else {
                &actual
            }
        );
    }
    Ok(())
}

/// 只暂存并校验指定引擎的本机监督 Runtime，不启动引擎进程。
/// 部署验收使用该入口逐个暖机，避免只检查移动盘文件却遗漏真正执行的本机副本。
pub fn check_supervisor(paths: &WorkstationPaths, engine: Engine) -> Result<SupervisorCheckResult> {
    let executable = supervisor_executable(paths, engine)?;
    verify_supervisor_build(paths, &executable)?;
    Ok(SupervisorCheckResult {
        engine,
        executable: executable.display().to_string(),
        build_id: crate::RUNTIME_BUILD_ID.to_string(),
    })
}

pub fn supervise(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    supervise_internal(paths, engine, true)
}

/// 仅供离线验收使用：沿用正式的进程监督和便携环境，但不读取供应商凭据。
///
/// 该入口不会被普通启动器调用，避免为了验证 Codex 的语言参数而写入测试 API Key。
pub fn supervise_payload(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    supervise_internal(paths, engine, false)
}

fn supervise_internal(
    paths: &WorkstationPaths,
    engine: Engine,
    include_provider: bool,
) -> Result<()> {
    let started_at = now_unix_ms();
    if let Err(error) = prepare_for_launch(paths, engine) {
        let failed = failed_state(
            paths,
            engine,
            std::process::id(),
            None,
            started_at,
            error.to_string(),
        );
        let _ = write_state(paths, &failed);
        return Err(error);
    }

    let machine_cache = prepare_machine_runtime(paths, engine).map_err(|error| {
        record_supervisor_preparation_failure(
            paths,
            engine,
            started_at,
            "本机运行缓存准备失败",
            error,
        )
    })?;

    if engine == Engine::Codex {
        crate::codex_machine::prepare(paths).map_err(|error| {
            record_supervisor_preparation_failure(
                paths,
                engine,
                started_at,
                "Codex本机运行镜像准备失败",
                error,
            )
        })?;
        append_runtime_event(paths, engine, "local NTFS runtime mirror ready")?;
    }

    if engine == Engine::OpenClaw {
        update_starting_message(paths, engine, "正在检查小龙虾随盘状态")?;
        let recovered = prepare_openclaw_session_locks(paths)?;
        if recovered > 0 {
            append_runtime_event(
                paths,
                engine,
                &format!("recovered stale session locks count={recovered}"),
            )?;
        }
        let normalized = normalize_openclaw_session_index_paths(paths)?;
        if normalized > 0 {
            append_runtime_event(
                paths,
                engine,
                &format!("normalized portable session paths count={normalized}"),
            )?;
        }
        let removed_transients = remove_openclaw_machine_transients(&machine_cache.join("temp"))?;
        if removed_transients > 0 {
            append_runtime_event(
                paths,
                engine,
                &format!("removed stale machine temp files count={removed_transients}"),
            )?;
        }
        append_runtime_event(paths, engine, "portable payload direct launch ready")?;
        update_starting_message(paths, engine, "小龙虾随盘状态已就绪，正在启动网关")?;
    }

    let loopback_port = match engine {
        Engine::OpenClaw => Some(find_available_loopback_port("OpenClaw服务")?),
        _ => None,
    };
    if let Some(port) = loopback_port {
        crate::config::set_openclaw_gateway_port(paths, port)?;
    }
    let mut spec = if include_provider {
        build_launch_spec(paths, engine, loopback_port)?
    } else {
        build_launch_spec_without_provider(paths, engine, loopback_port)?
    };
    if engine == Engine::OpenClaw {
        let shared_payload_root = paths.shared_openclaw_payload_root();
        let legacy_payload_root = paths.legacy_openclaw_payload_root();
        let payload = prepare_openclaw_machine_payload_with_roots(
            paths,
            &machine_cache,
            &shared_payload_root,
            legacy_payload_root.as_deref(),
        )
        .map_err(|error| {
            record_supervisor_preparation_failure(
                paths,
                engine,
                started_at,
                "小龙虾本机程序组件准备失败",
                error,
            )
        })?;
        apply_openclaw_machine_payload(&mut spec, &payload)?;
        append_runtime_event(paths, engine, "local immutable payload ready")?;
        update_starting_message(paths, engine, "小龙虾本机组件已就绪，正在启动网关")?;
    }
    // Mihomo 由 Codex 监督进程持有，不进入工作站全局环境。
    // 持续模式和仅本次模式都在 Codex 真正启动前就绪；
    // 任何代理错误都会中止本次启动，不允许悄悄回退到直连。
    let codex_proxy = if include_provider && engine == Engine::Codex {
        crate::codex_proxy::prepare_codex_launch(paths).map_err(|error| {
            record_supervisor_preparation_failure(
                paths,
                engine,
                started_at,
                "Codex专用代理启动失败",
                error,
            )
        })?
    } else {
        None
    };
    if let Some(proxy) = codex_proxy.as_ref() {
        spec.environment.extend(proxy.launch_environment());
        spec.args.push(proxy.chromium_proxy_argument());
    }
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.current_dir)
        .envs(spec.environment.iter().cloned());
    configure_command(&mut command, spec.open_new_console);

    let mut redirected_logs = None;
    if spec.open_new_console {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else {
        let logs = machine_cache.join("logs");
        fs::create_dir_all(&logs)?;
        let stdout_path = logs.join(format!("{engine}.stdout.log"));
        let stderr_path = logs.join(format!("{engine}.stderr.log"));
        let stdout_log = append_log_file(&stdout_path)?;
        let stderr_log = append_log_file(&stderr_path)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_log))
            .stderr(Stdio::from(stderr_log));
        redirected_logs = Some((stdout_path, stderr_path));
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("无法启动{}：{error}", spec.program.display());
            write_state(
                paths,
                &failed_state(
                    paths,
                    engine,
                    std::process::id(),
                    None,
                    started_at,
                    message.clone(),
                ),
            )?;
            bail!("{message}");
        }
    };
    let job = match ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let message = format!("引擎已创建但无法加入Job Object：{error}");
            write_state(
                paths,
                &failed_state(
                    paths,
                    engine,
                    std::process::id(),
                    Some(child.id()),
                    started_at,
                    message.clone(),
                ),
            )?;
            bail!("{message}");
        }
    };

    let codex_waiting_for_window = engine == Engine::Codex;
    let openclaw_waiting_for_health = engine == Engine::OpenClaw;
    let waiting_for_readiness = codex_waiting_for_window || openclaw_waiting_for_health;
    let mut state = EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine,
        status: if waiting_for_readiness {
            RuntimeStatus::Starting
        } else {
            RuntimeStatus::Running
        },
        supervisor_pid: std::process::id(),
        process_pid: Some(child.id()),
        executable: spec.program.clone(),
        started_at_unix_ms: started_at,
        updated_at_unix_ms: now_unix_ms(),
        loopback_port: spec.loopback_port,
        window_ready: !waiting_for_readiness,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some(match engine {
            Engine::Codex => "Codex进程已进入受控Job Object，正在等待可见窗口".to_string(),
            Engine::OpenClaw => "OpenClaw网关已进入受控Job Object，正在等待健康检查".to_string(),
            Engine::Claude => "引擎已进入受控Job Object".to_string(),
        }),
    };
    write_state(paths, &state)?;
    append_runtime_event(
        paths,
        engine,
        &format!(
            "spawned supervisor_pid={} process_pid={} waiting_for_window={}",
            state.supervisor_pid,
            child.id(),
            codex_waiting_for_window
        ),
    )?;

    let codex_window_deadline = std::time::Instant::now() + CODEX_WINDOW_WAIT;
    let mut next_codex_window_probe = std::time::Instant::now();
    let mut codex_ready_samples = 0_u8;
    let mut codex_window_timeout_recorded = false;
    let openclaw_health_wait = openclaw_health_wait(&machine_cache);
    let openclaw_health_deadline = std::time::Instant::now() + openclaw_health_wait;
    let mut openclaw_ready = false;

    loop {
        if let Some(exit_status) = child.try_wait().context("无法读取引擎退出状态")? {
            if let Some(logs) = redirected_logs.as_ref() {
                let _ = persist_engine_logs(paths, engine, logs);
            }
            // 主进程退出时 Chromium 子进程可能仍短暂持有 arg0 文件句柄。
            // 先释放 Job Object，确保整个受控进程树退出，再清理短生命周期目录。
            release_process_job(job);
            if engine == Engine::Codex {
                if let Err(error) = cleanup_codex_arg0_temp_with_retry(paths) {
                    let _ = append_runtime_event(
                        paths,
                        engine,
                        &format!("codex arg0 cleanup after exit failed: {error}"),
                    );
                }
                finish_codex_shutdown(
                    paths,
                    &mut state,
                    if exit_status.success() {
                        RuntimeStatus::Stopped
                    } else {
                        RuntimeStatus::Failed
                    },
                    &format!("引擎进程已退出：{exit_status}"),
                )?;
                break;
            }
            state.status = if exit_status.success() {
                RuntimeStatus::Stopped
            } else {
                RuntimeStatus::Failed
            };
            state.process_pid = None;
            state.window_ready = false;
            state.window_process_id = None;
            state.window_handle = None;
            state.window_title = None;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some(format!("引擎进程已退出：{exit_status}"));
            write_state(paths, &state)?;
            remove_if_exists(&paths.stop_request_file(engine.slug()))?;
            break;
        }

        if engine == Engine::Codex
            && !state.window_ready
            && std::time::Instant::now() >= next_codex_window_probe
        {
            next_codex_window_probe = std::time::Instant::now() + Duration::from_millis(750);
            let window = probe_process_tree_window(child.id());
            if window.ready {
                codex_ready_samples = codex_ready_samples.saturating_add(1);
                if codex_ready_samples >= CODEX_READY_SAMPLE_COUNT
                    && (!state.window_ready || state.status != RuntimeStatus::Running)
                {
                    state.status = RuntimeStatus::Running;
                    state.window_ready = true;
                    state.window_process_id = window.process_id;
                    state.window_handle = window.window_handle;
                    state.window_title = window.title;
                    state.updated_at_unix_ms = now_unix_ms();
                    state.message = Some("Codex窗口已出现并且可以响应".to_string());
                    write_state(paths, &state)?;
                    append_runtime_event(paths, engine, "window ready")?;
                }
            } else {
                codex_ready_samples = 0;
                if !codex_window_timeout_recorded
                    && std::time::Instant::now() >= codex_window_deadline
                {
                    codex_window_timeout_recorded = true;
                    state.status = RuntimeStatus::Failed;
                    state.updated_at_unix_ms = now_unix_ms();
                    state.message =
                        Some("Codex进程已经运行，但90秒内没有检测到可见窗口".to_string());
                    write_state(paths, &state)?;
                    append_runtime_event(paths, engine, "window readiness timeout")?;
                }
            }
        }

        if engine == Engine::OpenClaw && !openclaw_ready {
            let port = state
                .loopback_port
                .context("OpenClaw运行状态缺少本地网关端口")?;
            if probe_openclaw_health(port) {
                openclaw_ready = true;
                state.status = RuntimeStatus::Running;
                state.window_ready = true;
                state.updated_at_unix_ms = now_unix_ms();
                state.message = Some(format!("OpenClaw网关已就绪，Control UI端口为{port}"));
                write_state(paths, &state)?;
                append_runtime_event(paths, engine, &format!("gateway ready port={port}"))?;
                if let Err(error) = fs::write(openclaw_warm_marker(&machine_cache), b"ready\n") {
                    let _ = append_runtime_event(
                        paths,
                        engine,
                        &format!("gateway warm marker write failed: {error}"),
                    );
                }
            } else if std::time::Instant::now() >= openclaw_health_deadline {
                append_runtime_event(paths, engine, "gateway health timeout; terminating job")?;
                if force_terminate_job(&job, 1).is_err() {
                    terminate_child_as_fallback(&mut child)?;
                }
                let _ = child.wait();
                state.status = RuntimeStatus::Failed;
                state.process_pid = None;
                state.window_ready = false;
                state.updated_at_unix_ms = now_unix_ms();
                state.message = Some(format!(
                    "OpenClaw进程已经启动，但{}秒内网关健康检查未通过",
                    openclaw_health_wait.as_secs()
                ));
                if let Some(logs) = redirected_logs.as_ref() {
                    let _ = persist_engine_logs(paths, engine, logs);
                }
                write_state(paths, &state)?;
                remove_if_exists(&paths.stop_request_file(engine.slug()))?;
                break;
            }
        }

        if paths.stop_request_file(engine.slug()).exists() {
            state.status = RuntimeStatus::Stopping;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some("已请求正常退出".to_string());
            write_state(paths, &state)?;

            // Electron 即使主窗口已经被用户关闭，也可能保留隐藏窗口。旧逻辑只要
            // 给任意隐藏窗口发送过 WM_CLOSE 就等待完整宽限期，造成“停止”长时间
            // 转圈。这里只把真实可见窗口视为需要完整正常退出宽限期。
            let visible_window_present = probe_process_tree_window(child.id()).visible;
            let graceful_requested = match request_graceful_tree_stop(child.id()) {
                Ok(requested) => requested,
                Err(error) => {
                    let _ = append_runtime_event(
                        paths,
                        engine,
                        &format!("graceful stop request failed: {error}"),
                    );
                    false
                }
            };
            let graceful_wait = if graceful_requested && visible_window_present {
                STOP_WAIT
            } else {
                STOP_WITHOUT_WINDOW_WAIT
            };
            let _ = append_runtime_event(
                paths,
                engine,
                &format!(
                    "stop requested graceful={} visible_window={} wait_seconds={}",
                    graceful_requested,
                    visible_window_present,
                    graceful_wait.as_secs()
                ),
            );
            let deadline = std::time::Instant::now() + graceful_wait;
            let mut exited = false;
            while std::time::Instant::now() < deadline {
                if child.try_wait()?.is_some() {
                    exited = true;
                    break;
                }
                thread::sleep(POLL_INTERVAL);
            }
            if !exited {
                append_runtime_event(paths, engine, "graceful stop timeout; terminating job")?;
                if force_terminate_job(&job, 1).is_err() {
                    terminate_child_as_fallback(&mut child)?;
                }
                let _ = child.wait();
            }
            if let Some(logs) = redirected_logs.as_ref() {
                let _ = persist_engine_logs(paths, engine, logs);
            }
            release_process_job(job);
            if engine == Engine::Codex {
                if let Err(error) = cleanup_codex_arg0_temp_with_retry(paths) {
                    let _ = append_runtime_event(
                        paths,
                        engine,
                        &format!("codex arg0 cleanup after stop failed: {error}"),
                    );
                }
                finish_codex_shutdown(
                    paths,
                    &mut state,
                    RuntimeStatus::Stopped,
                    "引擎已停止，Job Object已释放",
                )?;
                break;
            }
            state.status = RuntimeStatus::Stopped;
            state.process_pid = None;
            state.window_ready = false;
            state.window_process_id = None;
            state.window_handle = None;
            state.window_title = None;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some("引擎已停止，Job Object已释放".to_string());
            write_state(paths, &state)?;
            remove_if_exists(&paths.stop_request_file(engine.slug()))?;
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    drop(codex_proxy);
    Ok(())
}

pub fn run_console(paths: &WorkstationPaths, engine: Engine) -> Result<()> {
    if engine != Engine::Claude {
        bail!("console命令当前只支持Claude Code");
    }

    prepare_for_launch(paths, Engine::Claude)?;
    remove_if_exists(&paths.stop_request_file(engine.slug()))?;
    let spec = build_launch_spec(paths, engine, None)?;
    let started_at = now_unix_ms();
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.current_dir)
        .envs(spec.environment.iter().cloned())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // 交互式 CLI 必须继承调用者的真实控制台，不能再创建隐藏监督进程或重定向标准句柄。
    let mut child = command
        .spawn()
        .with_context(|| format!("无法在当前控制台启动{engine}"))?;
    let job = match ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            bail!("Claude Code已创建但无法加入Job Object：{error}");
        }
    };

    let mut state = EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine,
        status: RuntimeStatus::Running,
        supervisor_pid: std::process::id(),
        process_pid: Some(child.id()),
        executable: spec.program,
        started_at_unix_ms: started_at,
        updated_at_unix_ms: now_unix_ms(),
        loopback_port: None,
        window_ready: true,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some("Claude Code正在当前交互控制台运行".to_string()),
    };
    write_state(paths, &state)?;
    append_runtime_event(
        paths,
        engine,
        &format!(
            "interactive running supervisor_pid={} process_pid={}",
            state.supervisor_pid,
            child.id()
        ),
    )?;

    loop {
        if let Some(exit_status) = child.try_wait().context("无法读取Claude Code退出状态")?
        {
            state.status = if exit_status.success() {
                RuntimeStatus::Stopped
            } else {
                RuntimeStatus::Failed
            };
            state.process_pid = None;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some(format!("Claude Code交互进程已退出：{exit_status}"));
            write_state(paths, &state)?;
            remove_if_exists(&paths.stop_request_file(engine.slug()))?;
            return if exit_status.success() {
                Ok(())
            } else {
                bail!("Claude Code退出码：{exit_status}")
            };
        }

        if paths.stop_request_file(engine.slug()).exists() {
            state.status = RuntimeStatus::Stopping;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some("已请求Claude Code退出".to_string());
            write_state(paths, &state)?;

            let _ = request_graceful_tree_stop(child.id());
            let deadline = std::time::Instant::now() + STOP_WAIT;
            let mut exited = false;
            while std::time::Instant::now() < deadline {
                if child.try_wait()?.is_some() {
                    exited = true;
                    break;
                }
                thread::sleep(POLL_INTERVAL);
            }
            if !exited {
                append_runtime_event(
                    paths,
                    engine,
                    "interactive graceful stop timeout; terminating job",
                )?;
                if force_terminate_job(&job, 1).is_err() {
                    terminate_child_as_fallback(&mut child)?;
                }
                let _ = child.wait();
            }
            state.status = RuntimeStatus::Stopped;
            state.process_pid = None;
            state.window_ready = false;
            state.window_process_id = None;
            state.window_handle = None;
            state.window_title = None;
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some("Claude Code交互进程已停止".to_string());
            write_state(paths, &state)?;
            remove_if_exists(&paths.stop_request_file(engine.slug()))?;
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

pub fn open_openclaw_dashboard(paths: &WorkstationPaths) -> Result<EngineState> {
    let mut state = status(paths, Engine::OpenClaw)?;
    let port = state
        .loopback_port
        .context("OpenClaw尚未分配Control UI端口")?;
    if !matches!(state.status, RuntimeStatus::Running)
        || state.process_pid.is_none_or(|pid| !process_is_alive(pid))
        || !probe_openclaw_health(port)
    {
        bail!("OpenClaw网关尚未就绪，不能打开Control UI");
    }

    // 端口与 token 都由 Rust 后端读取，URL 只交给 Win32 ShellExecuteW。
    // 不再二次冷启动 `openclaw dashboard` CLI，避免移动盘上数万个
    // JavaScript 文件被重新扫描，也避免 token 进入子进程命令行。
    crate::config::set_openclaw_gateway_port(paths, port)?;
    let control_url = crate::config::openclaw_control_url(paths, port)?;
    let machine_cache = paths.machine_engine_cache("openclaw");
    let checkpoint = openclaw_control_ui_checkpoint(&machine_cache);
    let observe_connection = state
        .message
        .as_deref()
        .is_some_and(|message| message.starts_with("OpenClaw网关已就绪"));
    open_url_native(control_url.as_str())?;
    // 严禁在事件中记录 control_url，其 fragment 包含本地网关 token。
    append_runtime_event(
        paths,
        Engine::OpenClaw,
        "control ui browser request submitted",
    )?;

    state.updated_at_unix_ms = now_unix_ms();
    state.message = Some("OpenClaw网关已启动，正在打开浏览器控制台".to_string());
    write_state(paths, &state)?;
    if observe_connection {
        observe_openclaw_control_ui(paths.clone(), port, control_url.to_string(), checkpoint);
    }
    Ok(state)
}

const OPENCLAW_HEALTH_PROBE_TIMEOUT_MS: u64 = 5_000;

fn probe_openclaw_health(port: u16) -> bool {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let Ok(mut stream) = TcpStream::connect_timeout(
        &address.into(),
        Duration::from_millis(OPENCLAW_HEALTH_PROBE_TIMEOUT_MS),
    ) else {
        return false;
    };
    let timeout = Some(Duration::from_millis(OPENCLAW_HEALTH_PROBE_TIMEOUT_MS));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = [0_u8; 256];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    http_response_is_success(&response[..read])
}

fn http_response_is_success(response: &[u8]) -> bool {
    response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.0 200")
}

pub fn stop(paths: &WorkstationPaths, engine: Engine, timeout: Duration) -> Result<EngineState> {
    let mut state = status(paths, engine)?;
    if matches!(
        state.status,
        RuntimeStatus::Saving | RuntimeStatus::Stopped | RuntimeStatus::Failed
    ) && state.process_pid.is_none_or(|pid| !process_is_alive(pid))
    {
        return Ok(state);
    }

    paths.prepare_engine_layout(engine)?;
    let stop_request = paths.stop_request_file(engine.slug());
    if let Some(parent) = stop_request.parent() {
        fs::create_dir_all(parent)?;
    }
    File::create(&stop_request).with_context(|| format!("无法写入{engine}停止请求"))?;
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        state = status(paths, engine)?;
        if matches!(
            state.status,
            RuntimeStatus::Saving | RuntimeStatus::Stopped | RuntimeStatus::Failed
        ) && state.process_pid.is_none_or(|pid| !process_is_alive(pid))
        {
            return Ok(state);
        }
        thread::sleep(POLL_INTERVAL);
    }
    bail!("{engine}在{}秒内没有完成停止", timeout.as_secs())
}

pub fn stop_all(paths: &WorkstationPaths, timeout: Duration) -> Result<Vec<EngineState>> {
    let mut states = Vec::new();
    for engine in Engine::ALL {
        let state = stop(paths, engine, timeout)?;
        states.push(if state.status == RuntimeStatus::Saving {
            wait_for_persisted_stop(paths, engine, timeout)?
        } else {
            state
        });
    }
    Ok(states)
}

pub fn status(paths: &WorkstationPaths, engine: Engine) -> Result<EngineState> {
    let Some(mut state) = read_state(paths, engine)? else {
        return Ok(stopped_state(paths, engine));
    };
    let observed_updated_at = state.updated_at_unix_ms;
    let had_supervisor_pid = state.supervisor_pid != 0;
    let had_process_pid = state.process_pid.is_some();
    let process_alive = state.process_pid.is_some_and(process_is_alive);
    let supervisor_alive = state.supervisor_pid != 0 && process_is_alive(state.supervisor_pid);
    let mut metadata_changed = false;
    if state.process_pid.is_some() && !process_alive {
        state.process_pid = None;
        metadata_changed = true;
    }
    if state.supervisor_pid != 0 && !supervisor_alive {
        state.supervisor_pid = 0;
        metadata_changed = true;
    }
    if !process_alive && !supervisor_alive {
        let had_window_metadata = state.window_ready
            || state.window_process_id.is_some()
            || state.window_handle.is_some()
            || state.window_title.is_some();
        let had_runtime_metadata = state.loopback_port.is_some() || had_window_metadata;
        state.loopback_port = None;
        state.window_ready = false;
        state.window_process_id = None;
        state.window_handle = None;
        state.window_title = None;
        if had_runtime_metadata {
            metadata_changed = true;
        }
    }
    // 普通状态查询只核对 PID，不再同步探测 Electron 窗口响应。
    // 可见窗口只在启动监督阶段做有界探测，避免 UI 每次轮询都被慢窗口拖住。
    // 只有父进程尚未来得及写入任何 PID 的短暂窗口可以保留 starting。若状态文件
    // 原本记录过 PID 而该进程已经消失（包括电脑重启），必须立即归一化为 stopped。
    let parent_spawn_in_progress = matches!(state.status, RuntimeStatus::Starting)
        && !had_supervisor_pid
        && !had_process_pid
        && now_unix_ms().saturating_sub(observed_updated_at) < PARENT_SPAWN_GRACE.as_millis();
    if !parent_spawn_in_progress
        && matches!(
            state.status,
            RuntimeStatus::Starting
                | RuntimeStatus::Running
                | RuntimeStatus::Stopping
                | RuntimeStatus::Saving
        )
        && !process_alive
        && !supervisor_alive
    {
        state.status = RuntimeStatus::Stopped;
        state.supervisor_pid = 0;
        state.process_pid = None;
        state.loopback_port = None;
        state.window_ready = false;
        state.window_process_id = None;
        state.window_handle = None;
        state.window_title = None;
        state.updated_at_unix_ms = now_unix_ms();
        state.message = Some("检测到旧状态文件，实际进程已经退出".to_string());
        write_state(paths, &state)?;
    } else if metadata_changed && !process_alive && !supervisor_alive && !parent_spawn_in_progress {
        // supervisor 存活时由 supervisor 独占持久化状态。UI/stop 的轮询只能返回
        // 内存中的规范化结果，不能与 supervisor 抢写同一个 JSON 文件。
        state.updated_at_unix_ms = now_unix_ms();
        write_state(paths, &state)?;
    }
    Ok(state)
}

pub fn status_all(paths: &WorkstationPaths) -> Result<Vec<EngineState>> {
    Engine::ALL
        .into_iter()
        .map(|engine| status(paths, engine))
        .collect()
}

pub fn probe(paths: &WorkstationPaths, engine: Engine) -> Result<ProbeResult> {
    prepare_for_launch(paths, engine)?;
    let spec = build_probe_spec(paths, engine)?;
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.current_dir)
        .envs(spec.environment.iter().cloned())
        .stdin(Stdio::null());
    configure_command(&mut command, false);
    let output = command
        .output()
        .with_context(|| format!("无法执行{engine}离线版本探针"))?;
    if engine == Engine::Codex {
        let _ = cleanup_codex_arg0_temp_at(&paths.codex_home);
    }
    Ok(ProbeResult {
        engine,
        program: spec.program.display().to_string(),
        success: output.status.success(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

pub fn diagnose(paths: &WorkstationPaths) -> Result<DiagnoseResult> {
    paths.prepare_layout()?;
    Ok(DiagnoseResult {
        root: paths.root.display().to_string(),
        portable_data: paths.data.display().to_string(),
        workspace: paths.workspace.display().to_string(),
        detections: detect(paths, None),
        states: status_all(paths)?,
        codex_config_exists: paths.codex_home.join("config.toml").is_file(),
        claude_config_exists: paths.claude_home.join("settings.json").is_file(),
        openclaw_config_exists: paths.openclaw_home.join("openclaw.json").is_file(),
        codex_locale: crate::locale::codex_locale_status(paths)?,
    })
}

fn read_state(paths: &WorkstationPaths, engine: Engine) -> Result<Option<EngineState>> {
    let state_path = paths.state_file(engine.slug());
    if !state_path.is_file() {
        return Ok(None);
    }
    let bytes =
        fs::read(&state_path).with_context(|| format!("无法读取 {}", state_path.display()))?;
    let state = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析 {}", state_path.display()))?;
    Ok(Some(state))
}

fn write_state(paths: &WorkstationPaths, state: &EngineState) -> Result<()> {
    let state_path = paths.state_file(state.engine.slug());
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
    }
    // 启动器和监督 sidecar 会在启动瞬间并发写同一个状态文件。
    // 每个进程使用独立临时文件，避免共同写入固定 *.json.tmp 导致状态被截断。
    let temporary = state_temporary_path(&state_path);
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).with_context(|| format!("无法写入 {}", temporary.display()))?;
    let mut last_error = None;
    for attempt in 0..STATE_WRITE_RETRIES {
        match replace_file_atomic(&temporary, &state_path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < STATE_WRITE_RETRIES {
                    thread::sleep(STATE_WRITE_RETRY_DELAY);
                }
            }
        }
    }
    let _ = fs::remove_file(&temporary);
    Err(last_error.unwrap_or_else(|| anyhow!("状态文件原子替换失败")))
}

fn update_starting_message(
    paths: &WorkstationPaths,
    engine: Engine,
    message: impl Into<String>,
) -> Result<()> {
    let Some(mut state) = read_state(paths, engine)? else {
        return Ok(());
    };
    if !matches!(state.status, RuntimeStatus::Starting) {
        return Ok(());
    }
    state.updated_at_unix_ms = now_unix_ms();
    state.message = Some(message.into());
    write_state(paths, &state)
}

fn state_temporary_path(state_path: &Path) -> std::path::PathBuf {
    state_path.with_extension(format!(
        "json.{}.{}.{}.tmp",
        std::process::id(),
        now_unix_ms(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn stopped_state(paths: &WorkstationPaths, engine: Engine) -> EngineState {
    EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine,
        status: RuntimeStatus::Stopped,
        supervisor_pid: 0,
        process_pid: None,
        executable: detect_engine(paths, engine).executable,
        started_at_unix_ms: 0,
        updated_at_unix_ms: now_unix_ms(),
        loopback_port: None,
        window_ready: false,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some("尚未启动".to_string()),
    }
}

fn failed_state(
    paths: &WorkstationPaths,
    engine: Engine,
    supervisor_pid: u32,
    process_pid: Option<u32>,
    started_at: u128,
    message: String,
) -> EngineState {
    EngineState {
        schema_version: STATE_SCHEMA_VERSION,
        engine,
        status: RuntimeStatus::Failed,
        supervisor_pid,
        process_pid,
        executable: detect_engine(paths, engine).executable,
        started_at_unix_ms: started_at,
        updated_at_unix_ms: now_unix_ms(),
        loopback_port: None,
        window_ready: false,
        window_process_id: None,
        window_handle: None,
        window_title: None,
        message: Some(message),
    }
}

fn find_available_loopback_port(purpose: &str) -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .with_context(|| format!("无法为{purpose}分配本地端口"))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn release_process_job(job: ProcessJob) {
    #[cfg(windows)]
    drop(job);
    #[cfg(not(windows))]
    let _ = job;
}

fn record_supervisor_preparation_failure(
    paths: &WorkstationPaths,
    engine: Engine,
    started_at: u128,
    stage: &str,
    error: anyhow::Error,
) -> anyhow::Error {
    let message = format!("{stage}：{error:#}");
    let failed = failed_state(
        paths,
        engine,
        std::process::id(),
        None,
        started_at,
        message.clone(),
    );
    let _ = write_state(paths, &failed);
    let _ = append_runtime_event(
        paths,
        engine,
        &format!("supervisor preparation failed stage={stage} error={error:#}"),
    );
    anyhow!(message)
}

fn record_parent_start_failure(
    paths: &WorkstationPaths,
    engine: Engine,
    started_at: u128,
    stage: &str,
    error: anyhow::Error,
) -> anyhow::Error {
    let message = format!("{stage}：{error:#}");
    let failed = failed_state(paths, engine, 0, None, started_at, message.clone());
    let _ = write_state(paths, &failed);
    let _ = append_runtime_event(
        paths,
        engine,
        &format!("parent start failed stage={stage} error={error:#}"),
    );
    anyhow!(message)
}

fn append_log_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("无法打开日志 {}", path.display()))
}

fn append_runtime_event(paths: &WorkstationPaths, engine: Engine, event: &str) -> Result<()> {
    let mut log = append_log_file(&paths.logs.join("runtime-events.log"))?;
    writeln!(log, "{} {} {}", now_unix_ms(), engine.slug(), event)?;
    Ok(())
}

fn prepare_machine_runtime(paths: &WorkstationPaths, engine: Engine) -> Result<PathBuf> {
    let root = paths.machine_engine_cache(engine.slug());
    let mut directories = vec![root.clone(), root.join("logs")];
    if engine == Engine::OpenClaw {
        directories.extend([
            root.join("temp"),
            root.join("npm-cache"),
            root.join("node-compile-cache"),
            root.join("state"),
        ]);
    }
    for directory in directories {
        fs::create_dir_all(&directory)
            .with_context(|| format!("无法创建本机易失缓存 {}", directory.display()))?;
    }
    for log in [
        root.join("logs").join(format!("{engine}.stdout.log")),
        root.join("logs").join(format!("{engine}.stderr.log")),
    ] {
        File::create(&log).with_context(|| format!("无法初始化本机日志 {}", log.display()))?;
    }
    Ok(root)
}

/// Codex CLI 会在 `CODEX_HOME/tmp/arg0` 解压短生命周期入口文件。异常退出或移动盘
/// 文件系统的延迟写入可能留下非空目录，之后每次启动都会先扫描并尝试清理全部旧目录，
/// 造成明显启动延迟。这里只处理固定父目录下、名称严格匹配 `codex-arg0*` 的临时目录；
/// 项目、会话、配置、Skills 和插件目录均不在处理范围内。
fn cleanup_codex_arg0_temp(paths: &WorkstationPaths) -> Result<usize> {
    cleanup_codex_arg0_temp_at(&paths.codex_machine_home())
}

fn cleanup_codex_arg0_temp_at(home: &Path) -> Result<usize> {
    let arg0_root = home.join("tmp").join("arg0");
    if !arg0_root.is_dir() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in fs::read_dir(&arg0_root)
        .with_context(|| format!("无法读取 Codex 临时目录 {}", arg0_root.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !file_type.is_dir() || !name.starts_with("codex-arg0") {
            continue;
        }
        fs::remove_dir_all(entry.path())
            .with_context(|| format!("无法清理 Codex 临时入口目录 {}", entry.path().display()))?;
        removed += 1;
    }
    Ok(removed)
}

fn cleanup_codex_arg0_temp_with_retry(paths: &WorkstationPaths) -> Result<usize> {
    let mut last_error = None;
    for attempt in 0..CODEX_ARG0_CLEANUP_RETRIES {
        match cleanup_codex_arg0_temp(paths) {
            Ok(removed) => return Ok(removed),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < CODEX_ARG0_CLEANUP_RETRIES {
                    thread::sleep(CODEX_ARG0_CLEANUP_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Codex arg0 临时目录清理失败")))
}

fn sync_codex_machine_to_portable(paths: &WorkstationPaths) -> Result<()> {
    let summary = crate::codex_machine::sync_to_portable(paths)?;
    append_runtime_event(
        paths,
        Engine::Codex,
        &format!(
            "portable sync complete files={} bytes={} skipped={}",
            summary.copied_files, summary.copied_bytes, summary.skipped_files
        ),
    )?;
    Ok(())
}

fn finish_codex_shutdown(
    paths: &WorkstationPaths,
    state: &mut EngineState,
    final_status: RuntimeStatus,
    final_message: &str,
) -> Result<()> {
    state.status = RuntimeStatus::Saving;
    state.process_pid = None;
    state.window_ready = false;
    state.window_process_id = None;
    state.window_handle = None;
    state.window_title = None;
    state.updated_at_unix_ms = now_unix_ms();
    state.message = Some("Codex已关闭，正在保存随盘数据".to_string());
    write_state(paths, state)?;
    remove_if_exists(&paths.stop_request_file(Engine::Codex.slug()))?;
    append_runtime_event(paths, Engine::Codex, "process stopped; portable sync begin")?;

    // Job Object 关闭后给 Chromium/SQLite 少量时间释放句柄。数据回写不再阻塞
    // “停止 Codex”的确认，但安全弹出仍会通过 stop_all 等待最终状态。
    thread::sleep(CODEX_HANDLE_RELEASE_GRACE);
    let sync_error = sync_codex_machine_to_portable(paths).err();
    if let Some(error) = sync_error.as_ref() {
        let _ = append_runtime_event(
            paths,
            Engine::Codex,
            &format!("portable sync failed: {error}"),
        );
    }

    state.status = final_status;
    state.supervisor_pid = 0;
    state.updated_at_unix_ms = now_unix_ms();
    state.message = Some(match sync_error {
        Some(error) => {
            format!("{final_message}；部分随盘数据保存未完成，下次启动将自动恢复：{error}")
        }
        None => format!("{final_message}；随盘数据已保存"),
    });
    write_state(paths, state)
}

fn wait_for_persisted_stop(
    paths: &WorkstationPaths,
    engine: Engine,
    timeout: Duration,
) -> Result<EngineState> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let state = status(paths, engine)?;
        if matches!(state.status, RuntimeStatus::Stopped | RuntimeStatus::Failed)
            && state.process_pid.is_none_or(|pid| !process_is_alive(pid))
            && (state.supervisor_pid == 0 || !process_is_alive(state.supervisor_pid))
        {
            return Ok(state);
        }
        thread::sleep(POLL_INTERVAL);
    }
    bail!(
        "{engine}已经关闭，但随盘数据在{}秒内没有保存完成",
        timeout.as_secs()
    )
}

fn persist_engine_logs(
    paths: &WorkstationPaths,
    engine: Engine,
    logs: &(PathBuf, PathBuf),
) -> Result<()> {
    fs::create_dir_all(&paths.logs)?;
    for (source, suffix) in [(&logs.0, "stdout"), (&logs.1, "stderr")] {
        if !source.is_file() {
            continue;
        }
        let destination = paths.logs.join(format!("{engine}.{suffix}.log"));
        copy_file_atomic(source, &destination)?;
    }
    Ok(())
}

/// 在开发/发布阶段把 OpenClaw 的 Node 运行时和 core 打成一个顺序读取的 ZIP。
/// 该命令不包含用户配置、会话、API Key 或日志，只包含不可变程序载荷。
pub fn pack_openclaw_payload(
    paths: &WorkstationPaths,
    output: Option<&Path>,
) -> Result<OpenClawPayloadPackResult> {
    let source_package = paths
        .openclaw_entry
        .parent()
        .context("OpenClaw入口缺少包目录")?;
    let source_node_modules = source_package
        .parent()
        .context("OpenClaw包缺少node_modules目录")?;
    let source_core = source_node_modules
        .parent()
        .context("OpenClaw包缺少core目录")?;
    let source_node_root = paths
        .openclaw_node
        .parent()
        .context("OpenClaw Node缺少运行目录")?;
    if !source_core.join("package-lock.json").is_file() {
        bail!("OpenClaw core缺少package-lock.json，无法打包本机运行载荷");
    }

    let fingerprint =
        openclaw_payload_fingerprint(source_core, &paths.openclaw_entry, &paths.openclaw_node)?;
    let destination = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| paths.openclaw_payload_archive());
    let parent = destination.parent().context("OpenClaw载荷包缺少父目录")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("无法创建OpenClaw载荷包目录 {}", parent.display()))?;
    let temporary = destination.with_extension(format!(
        "zip.{}.{}.tmp",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| -> Result<(usize, u64)> {
        let file = File::create(&temporary)
            .with_context(|| format!("无法创建OpenClaw载荷包 {}", temporary.display()))?;
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        let manifest = OpenClawPayloadManifest {
            schema_version: OPENCLAW_PAYLOAD_ARCHIVE_SCHEMA_VERSION,
            fingerprint: fingerprint.clone(),
        };
        writer.start_file(OPENCLAW_PAYLOAD_MANIFEST_NAME, options)?;
        writer.write_all(&serde_json::to_vec_pretty(&manifest)?)?;

        let mut files = Vec::new();
        collect_openclaw_payload_files(source_node_root, Path::new("node-win-x64"), &mut files)?;
        collect_openclaw_payload_files(source_core, Path::new("core"), &mut files)?;
        files.sort_by(|left, right| left.0.cmp(&right.0));
        if files.len() > OPENCLAW_PAYLOAD_MAX_FILES {
            bail!(
                "OpenClaw载荷文件数量异常：{}，上限为{}",
                files.len(),
                OPENCLAW_PAYLOAD_MAX_FILES
            );
        }

        let mut total_bytes = 0_u64;
        let mut buffer = [0_u8; 128 * 1024];
        for (relative, source) in &files {
            let size = fs::metadata(source)?.len();
            total_bytes = total_bytes
                .checked_add(size)
                .context("OpenClaw载荷大小溢出")?;
            if total_bytes > OPENCLAW_PAYLOAD_MAX_UNCOMPRESSED_BYTES {
                bail!(
                    "OpenClaw载荷未压缩大小超过{}字节安全上限",
                    OPENCLAW_PAYLOAD_MAX_UNCOMPRESSED_BYTES
                );
            }
            let archive_name = relative.to_string_lossy().replace('\\', "/");
            writer.start_file(archive_name, options)?;
            let mut input = File::open(source)
                .with_context(|| format!("无法读取OpenClaw载荷文件 {}", source.display()))?;
            loop {
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                writer.write_all(&buffer[..read])?;
            }
        }
        writer.finish()?.sync_all()?;
        Ok((files.len(), total_bytes))
    })();

    let (file_count, uncompressed_bytes) = match result {
        Ok(summary) => summary,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    replace_file_atomic(&temporary, &destination)
        .with_context(|| format!("无法启用OpenClaw载荷包 {}", destination.display()))?;
    let archive_bytes = fs::metadata(&destination)?.len();
    Ok(OpenClawPayloadPackResult {
        archive: destination.display().to_string(),
        fingerprint,
        file_count,
        uncompressed_bytes,
        archive_bytes,
    })
}

fn collect_openclaw_payload_files(
    source_root: &Path,
    archive_root: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    let mut stack = vec![(source_root.to_path_buf(), archive_root.to_path_buf())];
    while let Some((directory, relative_root)) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("无法读取OpenClaw载荷目录 {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!(
                    "OpenClaw载荷包含不允许的符号链接：{}",
                    entry.path().display()
                );
            }
            let relative = relative_root.join(entry.file_name());
            if file_type.is_dir() {
                stack.push((entry.path(), relative));
            } else if file_type.is_file() {
                files.push((relative, entry.path()));
            } else {
                bail!(
                    "OpenClaw载荷包含不支持的文件类型：{}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(())
}

/// OpenClaw 的 core 包含数万个小型 JavaScript 文件。直接从 exFAT
/// 移动盘启动时，Node 加载模块的随机读会占用大部分启动时间。
/// 这里按上游 lockfile 和运行体指纹在当前电脑 NTFS 建立只读镜像：
/// 首次会初始化一次，同版本后续启动直接复用；用户数据仍位于移动盘。
fn prepare_openclaw_machine_payload_with_roots(
    paths: &WorkstationPaths,
    machine_cache: &Path,
    shared_payload_root: &Path,
    legacy_payload_root: Option<&Path>,
) -> Result<OpenClawMachinePayload> {
    let source_package = paths
        .openclaw_entry
        .parent()
        .context("OpenClaw入口缺少包目录")?;
    let source_node_modules = source_package
        .parent()
        .context("OpenClaw包缺少node_modules目录")?;
    let source_core = source_node_modules
        .parent()
        .context("OpenClaw包缺少core目录")?;
    let source_node_root = paths
        .openclaw_node
        .parent()
        .context("OpenClaw Node缺少运行目录")?;
    if !source_core.join("package-lock.json").is_file() {
        bail!("OpenClaw core缺少package-lock.json，无法建立可验证镜像");
    }

    let fingerprint =
        openclaw_payload_fingerprint(source_core, &paths.openclaw_entry, &paths.openclaw_node)?;
    let relative_entry = paths
        .openclaw_entry
        .strip_prefix(source_core)
        .context("OpenClaw入口不在core目录内")?;
    let node_file_name = paths
        .openclaw_node
        .file_name()
        .context("OpenClaw Node缺少文件名")?;
    let isolated_payload_root = machine_cache.join("payload");
    let _ = cleanup_abandoned_openclaw_payload_staging(&isolated_payload_root, &fingerprint);
    let _ = cleanup_abandoned_openclaw_payload_staging(shared_payload_root, &fingerprint);

    // 新版优先使用按内容指纹共享的不可变缓存；随后兼容本次隔离版本以及隔离前
    // 已经预热完成的旧缓存。三个位置都只在 marker 和入口文件完整时才会复用。
    for payload_root in std::iter::once(shared_payload_root)
        .chain(std::iter::once(isolated_payload_root.as_path()))
        .chain(legacy_payload_root.into_iter())
    {
        if let Some(payload) =
            reusable_openclaw_payload(payload_root, &fingerprint, node_file_name, relative_entry)
        {
            return Ok(payload);
        }
    }

    fs::create_dir_all(shared_payload_root)?;
    let destination = shared_payload_root.join(&fingerprint);
    let destination_node = destination.join("node-win-x64").join(node_file_name);
    let destination_entry = destination.join("core").join(relative_entry);
    let ready_marker = destination.join("ready-v1");
    let staging = shared_payload_root.join(format!(
        ".staging-{fingerprint}-{}-{}",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .with_context(|| format!("无法清理OpenClaw载荷临时目录 {}", staging.display()))?;
    }

    let result = (|| -> Result<()> {
        let archive = paths.openclaw_payload_archive();
        if archive.is_file() {
            update_starting_message(
                paths,
                Engine::OpenClaw,
                "首次在此电脑运行，正在解压小龙虾本机组件",
            )?;
            extract_openclaw_payload_archive(paths, &archive, &staging, &fingerprint)?;
        } else {
            update_starting_message(
                paths,
                Engine::OpenClaw,
                "未找到预打包组件，正在从移动盘初始化小龙虾（可能较慢）",
            )?;
            crate::skills::package::copy_tree(source_node_root, &staging.join("node-win-x64"))
                .context("无法把OpenClaw Node运行时镜像到本机NTFS")?;
            crate::skills::package::copy_tree(source_core, &staging.join("core"))
                .context("无法把OpenClaw core镜像到本机NTFS")?;
        }
        let staged_node = staging.join("node-win-x64").join(node_file_name);
        let staged_entry = staging.join("core").join(relative_entry);
        if !staged_node.is_file() || !staged_entry.is_file() {
            bail!("OpenClaw本机运行镜像验证失败");
        }
        fs::write(staging.join("ready-v1"), format!("{fingerprint}\n"))?;
        if destination.exists() {
            // 只可能是并发初始化已经完成。完整镜像通过 marker 和
            // 两个入口文件验证后直接复用，不覆盖正在使用的目录。
            if ready_marker.is_file() && destination_node.is_file() && destination_entry.is_file() {
                let _ = fs::remove_dir_all(&staging);
                return Ok(());
            }
            bail!("OpenClaw本机运行镜像目录不完整：{}", destination.display());
        }
        fs::rename(&staging, &destination)
            .with_context(|| format!("无法启用OpenClaw本机运行镜像 {}", destination.display()))?;
        Ok(())
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;

    Ok(OpenClawMachinePayload {
        node: destination_node,
        entry: destination_entry,
    })
}

fn extract_openclaw_payload_archive(
    paths: &WorkstationPaths,
    archive_path: &Path,
    destination: &Path,
    expected_fingerprint: &str,
) -> Result<()> {
    let file = File::open(archive_path)
        .with_context(|| format!("无法打开OpenClaw载荷包 {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("无法解析OpenClaw载荷包 {}", archive_path.display()))?;
    if archive.len() > OPENCLAW_PAYLOAD_MAX_FILES + 1 {
        bail!(
            "OpenClaw载荷包文件数量异常：{}，上限为{}",
            archive.len(),
            OPENCLAW_PAYLOAD_MAX_FILES + 1
        );
    }
    let manifest: OpenClawPayloadManifest = {
        let mut entry = archive
            .by_name(OPENCLAW_PAYLOAD_MANIFEST_NAME)
            .context("OpenClaw载荷包缺少manifest")?;
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .context("无法读取OpenClaw载荷manifest")?;
        serde_json::from_slice(&bytes).context("OpenClaw载荷manifest格式无效")?
    };
    if manifest.schema_version != OPENCLAW_PAYLOAD_ARCHIVE_SCHEMA_VERSION {
        bail!("OpenClaw载荷包版本不兼容：{}", manifest.schema_version);
    }
    if manifest.fingerprint != expected_fingerprint {
        bail!(
            "OpenClaw载荷包与当前程序不一致：archive={} source={}",
            manifest.fingerprint,
            expected_fingerprint
        );
    }

    fs::create_dir_all(destination)
        .with_context(|| format!("无法创建OpenClaw解压目录 {}", destination.display()))?;
    let total_entries = archive.len();
    let mut extracted_files = 0_usize;
    let mut extracted_bytes = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    for index in 0..total_entries {
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            bail!("OpenClaw载荷包包含越界路径：{}", entry.name());
        };
        if relative == Path::new(OPENCLAW_PAYLOAD_MANIFEST_NAME) {
            continue;
        }
        let allowed_root = relative.components().next().is_some_and(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("core" | "node-win-x64")
            )
        });
        if !allowed_root {
            bail!("OpenClaw载荷包包含未知顶层路径：{}", relative.display());
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("OpenClaw载荷包包含不允许的符号链接：{}", entry.name());
        }
        let target = destination.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        extracted_files += 1;
        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .context("OpenClaw载荷解压大小溢出")?;
        if extracted_files > OPENCLAW_PAYLOAD_MAX_FILES
            || extracted_bytes > OPENCLAW_PAYLOAD_MAX_UNCOMPRESSED_BYTES
        {
            bail!("OpenClaw载荷包超过允许的文件数量或大小");
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&target)
            .with_context(|| format!("无法创建OpenClaw本机文件 {}", target.display()))?;
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
        }
        if extracted_files % OPENCLAW_PAYLOAD_PROGRESS_INTERVAL == 0 {
            let _ = update_starting_message(
                paths,
                Engine::OpenClaw,
                format!("首次初始化小龙虾本机组件：已处理 {extracted_files} 个文件"),
            );
        }
    }
    if extracted_files == 0 {
        bail!("OpenClaw载荷包没有可解压文件");
    }
    Ok(())
}

fn reusable_openclaw_payload(
    payload_root: &Path,
    fingerprint: &str,
    node_file_name: &std::ffi::OsStr,
    relative_entry: &Path,
) -> Option<OpenClawMachinePayload> {
    let destination = payload_root.join(fingerprint);
    let marker = fs::read_to_string(destination.join("ready-v1")).ok()?;
    if marker.trim() != fingerprint {
        return None;
    }
    let node = destination.join("node-win-x64").join(node_file_name);
    let entry = destination.join("core").join(relative_entry);
    (node.is_file() && entry.is_file()).then_some(OpenClawMachinePayload { node, entry })
}

fn cleanup_abandoned_openclaw_payload_staging(
    payload_root: &Path,
    fingerprint: &str,
) -> Result<usize> {
    if !payload_root.is_dir() {
        return Ok(0);
    }
    let prefix = format!(".staging-{fingerprint}-");
    let mut removed = 0;
    for entry in fs::read_dir(payload_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
            continue;
        };
        let Some((pid, _sequence)) = suffix.split_once('-') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if process_is_alive(pid) {
            continue;
        }
        fs::remove_dir_all(entry.path()).with_context(|| {
            format!(
                "无法清理已中断的OpenClaw载荷目录 {}",
                entry.path().display()
            )
        })?;
        removed += 1;
    }
    Ok(removed)
}

fn openclaw_payload_fingerprint(
    source_core: &Path,
    source_entry: &Path,
    source_node: &Path,
) -> Result<String> {
    let mut digest = Sha256::new();
    for (relative_label, file) in [
        ("core/package.json", source_core.join("package.json")),
        (
            "core/package-lock.json",
            source_core.join("package-lock.json"),
        ),
        (
            "core/node_modules/openclaw/openclaw.mjs",
            source_entry.to_path_buf(),
        ),
        ("node-win-x64/node.exe", source_node.to_path_buf()),
    ] {
        digest.update(relative_label.as_bytes());
        digest.update([0]);
        let mut input = File::open(&file)
            .with_context(|| format!("无法读取OpenClaw运行指纹文件 {}", file.display()))?;
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        digest.update([0xff]);
    }
    let fingerprint = digest.finalize();
    let mut encoded = String::with_capacity(16);
    for byte in &fingerprint[..8] {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")?;
    }
    Ok(encoded)
}

/// OpenClaw 使用 hard-link 和原子锁维护会话。exFAT 不支持 hard-link，所以实时状态
/// 在当前电脑的 NTFS 缓存中运行；会话、身份和设备状态每秒增量同步回移动盘。
#[cfg(test)]
fn prepare_openclaw_machine_state(paths: &WorkstationPaths, machine_cache: &Path) -> Result<()> {
    let machine_state = machine_cache.join("state");
    if machine_state.is_dir() {
        // 上次异常退出时，本机镜像可能比移动盘新；先做崩溃恢复。
        // 不再每次删除并重建整个目录，否则 28MB 会话状态会在 exFAT
        // 与 NTFS 之间反复全量拷贝，直接拉长网关启动时间。
        sync_openclaw_tree(&machine_state, &paths.openclaw_home, true)?;
    }
    fs::create_dir_all(&machine_state)?;
    remove_openclaw_machine_transients(&machine_state)?;
    sync_openclaw_tree(&paths.openclaw_home, &machine_state, false)
}

#[cfg(test)]
fn sync_openclaw_machine_state_to_portable(
    paths: &WorkstationPaths,
    machine_cache: &Path,
) -> Result<()> {
    let machine_state = machine_cache.join("state");
    if !machine_state.is_dir() {
        return Ok(());
    }
    sync_openclaw_tree(&machine_state, &paths.openclaw_home, true)
}

#[cfg(test)]
fn sync_openclaw_tree(source: &Path, destination: &Path, to_portable: bool) -> Result<()> {
    let mut stack = vec![(source.to_path_buf(), PathBuf::new())];
    while let Some((directory, relative)) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("无法读取OpenClaw状态目录 {}", directory.display()))?
        {
            let entry = entry?;
            let child_relative = relative.join(entry.file_name());
            if openclaw_sync_entry_is_transient(&child_relative, to_portable) {
                continue;
            }
            let file_type = entry.file_type()?;
            let target = destination.join(&child_relative);
            if file_type.is_dir() {
                fs::create_dir_all(&target)?;
                stack.push((entry.path(), child_relative));
            } else if file_type.is_file()
                && source_file_is_newer_or_different(&entry.path(), &target)?
            {
                copy_file_atomic(&entry.path(), &target)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn openclaw_sync_entry_is_transient(relative: &Path, to_portable: bool) -> bool {
    if relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("plugin-skills" | "logs")
        )
    }) {
        return true;
    }
    let name = relative
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    name.contains(".lock")
        || name.ends_with(".tmp")
        || name.starts_with(".usage-cost-cache")
        || (to_portable && name.eq_ignore_ascii_case("openclaw.json"))
}

#[cfg(test)]
fn source_file_is_newer_or_different(source: &Path, destination: &Path) -> Result<bool> {
    if !destination.is_file() {
        return Ok(true);
    }
    let source_metadata = fs::metadata(source)?;
    let destination_metadata = fs::metadata(destination)?;
    if source_metadata.len() != destination_metadata.len() {
        return Ok(true);
    }
    let source_is_newer = match (source_metadata.modified(), destination_metadata.modified()) {
        (Ok(source_time), Ok(destination_time)) => source_time > destination_time,
        _ => false,
    };
    if !source_is_newer {
        return Ok(false);
    }
    // fs::copy 会让目标文件拥有更新的时间戳。只比时间会导致同一
    // 文件在两个方向反复回写。大小相同时做流式内容比对；相同就不写盘。
    Ok(!files_have_same_content(source, destination)?)
}

#[cfg(test)]
fn files_have_same_content(left: &Path, right: &Path) -> Result<bool> {
    let mut left_file = File::open(left)?;
    let mut right_file = File::open(right)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_read = left_file.read(&mut left_buffer)?;
        let right_read = right_file.read(&mut right_buffer)?;
        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

fn remove_openclaw_machine_transients(root: &Path) -> Result<usize> {
    let mut removed = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("无法读取OpenClaw本机状态目录 {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let entry_path = entry.path();
            let name = entry_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let transient = name.contains(".lock")
                || name.ends_with(".tmp")
                || name.starts_with(".usage-cost-cache");
            if file_type.is_file() && transient {
                fs::remove_file(&entry_path).with_context(|| {
                    format!("无法清理OpenClaw本机临时文件 {}", entry_path.display())
                })?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn copy_file_atomic(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension(format!(
        "yunshu-sync-{}-{}.tmp",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::copy(source, &temporary).with_context(|| {
        format!(
            "无法原子复制运行文件 {} -> {}",
            source.display(),
            temporary.display()
        )
    })?;
    replace_file_atomic(&temporary, destination)
}

fn normalize_openclaw_session_index_paths(paths: &WorkstationPaths) -> Result<usize> {
    let sessions_root = paths.openclaw_home.join("agents/main/sessions");
    let index_path = sessions_root.join("sessions.json");
    if !index_path.is_file() {
        return Ok(0);
    }
    let mut index: Value = serde_json::from_slice(
        &fs::read(&index_path)
            .with_context(|| format!("无法读取OpenClaw会话索引 {}", index_path.display()))?,
    )
    .with_context(|| format!("OpenClaw会话索引格式无效 {}", index_path.display()))?;
    let entries = index
        .as_object_mut()
        .context("OpenClaw会话索引必须是JSON对象")?;
    let mut normalized = 0;
    for entry in entries.values_mut() {
        let Some(session_file) = entry.get_mut("sessionFile") else {
            continue;
        };
        let Some(current) = session_file.as_str() else {
            continue;
        };
        let Some(file_name) = current
            .rsplit(|character| character == '\\' || character == '/')
            .next()
        else {
            continue;
        };
        if file_name.is_empty() {
            continue;
        }
        let portable = sessions_root.join(file_name);
        if !portable.is_file() {
            continue;
        }
        let portable = portable.to_string_lossy();
        if current.eq_ignore_ascii_case(portable.as_ref()) {
            continue;
        }
        *session_file = Value::String(portable.into_owned());
        normalized += 1;
    }
    if normalized == 0 {
        return Ok(0);
    }
    let temporary = index_path.with_extension(format!(
        "yunshu-portable-{}-{}.tmp",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut bytes = serde_json::to_vec_pretty(&index)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入OpenClaw便携会话索引 {}", temporary.display()))?;
    replace_file_atomic(&temporary, &index_path)
        .with_context(|| format!("无法启用OpenClaw便携会话索引 {}", index_path.display()))?;
    Ok(normalized)
}

/// OpenClaw 使用 `<session>.jsonl.lock` 保护会话写入。电脑强制重启或进程被终止后，
/// 锁文件可能留在移动盘上；继续启动会在真正回复落盘时才报 EPERM/EEXIST。
///
/// 这里只处理 PID 已不存在或载荷损坏的锁，仍有活动 PID 的锁绝不删除。
/// 旧锁先移动到 Backups，随后用与 OpenClaw 相同的 create_new 语义做一次短探针，
/// 让不支持写锁的文件系统在启动前明确报错，而不是聊天时无限转圈。
fn prepare_openclaw_session_locks(paths: &WorkstationPaths) -> Result<usize> {
    let sessions_root = paths
        .openclaw_home
        .join("agents")
        .join("main")
        .join("sessions");
    fs::create_dir_all(&sessions_root)
        .with_context(|| format!("无法创建OpenClaw会话目录 {}", sessions_root.display()))?;

    let mut stack = vec![sessions_root.clone()];
    let mut stale_locks = Vec::new();
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("无法扫描OpenClaw会话目录 {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let is_lock_artifact = name.contains(".lock");
            let file_type = entry.file_type()?;
            if file_type.is_dir() && !is_lock_artifact {
                stack.push(path);
                continue;
            }
            if !is_lock_artifact {
                continue;
            }
            let owner_pid = (!file_type.is_dir())
                .then(|| fs::read_to_string(&path).ok())
                .flatten()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|value| value.get("pid").and_then(Value::as_u64))
                .and_then(|pid| u32::try_from(pid).ok());
            if owner_pid.is_some_and(process_is_alive) {
                bail!(
                    "OpenClaw会话仍由活动进程PID={}占用，请先停止该网关",
                    owner_pid.unwrap_or_default()
                );
            }
            stale_locks.push(path);
        }
    }

    if !stale_locks.is_empty() {
        let backup = paths
            .backups
            .join("openclaw-stale-locks")
            .join(now_unix_ms().to_string());
        fs::create_dir_all(&backup)?;
        for (index, lock) in stale_locks.iter().enumerate() {
            let name = lock
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("session.jsonl.lock");
            let destination = backup.join(format!("{index}-{name}"));
            fs::rename(lock, &destination).with_context(|| {
                format!(
                    "无法隔离OpenClaw旧会话锁 {}；请确认移动盘未被其他程序占用",
                    lock.display()
                )
            })?;
        }
    }

    let probe = sessions_root.join(format!(
        ".yunshu-lock-probe-{}-{}.jsonl.lock",
        std::process::id(),
        STATE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .with_context(|| {
                format!(
                    "OpenClaw会话目录无法创建排他锁 {}；当前移动盘不适合直接保存实时会话",
                    probe.display()
                )
            })?;
        file.write_all(b"lock-probe")?;
        file.flush()?;
    }
    remove_if_exists(&probe)?;
    Ok(stale_locks.len())
}

fn remove_if_exists(path: &Path) -> Result<()> {
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
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn openclaw_health_accepts_only_http_200() {
        assert!(http_response_is_success(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}"
        ));
        assert!(http_response_is_success(b"HTTP/1.0 200 OK\r\n\r\n"));
        assert!(!http_response_is_success(
            b"HTTP/1.1 401 Unauthorized\r\n\r\n"
        ));
        assert!(!http_response_is_success(b""));
    }

    #[test]
    fn openclaw_health_tolerates_short_event_loop_stall() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 256];
            let _ = stream.read(&mut request);
            thread::sleep(Duration::from_millis(750));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        });

        assert!(probe_openclaw_health(port));
        responder.join().unwrap().unwrap();
    }

    #[test]
    fn openclaw_machine_payload_replaces_only_program_and_entry() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        for file in [
            &paths.openclaw_node,
            &paths.openclaw_entry,
            &paths.bundled_node,
            &paths.bundled_python,
        ] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"test").unwrap();
        }
        let mut spec =
            build_launch_spec_without_provider(&paths, Engine::OpenClaw, Some(18789)).unwrap();
        let original_tail = spec.args[1..].to_vec();
        let local = OpenClawMachinePayload {
            node: temporary.path().join("local/node.exe"),
            entry: temporary.path().join("local/openclaw.mjs"),
        };

        apply_openclaw_machine_payload(&mut spec, &local).unwrap();

        assert_eq!(spec.program, local.node);
        assert_eq!(spec.args.first().unwrap(), local.entry.as_os_str());
        assert_eq!(spec.args[1..], original_tail);
        assert_eq!(spec.loopback_port, Some(18789));
    }

    #[test]
    fn concurrent_state_writers_receive_unique_temporary_paths() {
        let state = Path::new("runtime").join("openclaw.json");
        let first = state_temporary_path(&state);
        let second = state_temporary_path(&state);
        assert_ne!(first, second);
        assert_eq!(first.parent(), second.parent());
    }

    #[test]
    fn supervisor_runtime_is_staged_under_local_destination_by_build_id() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("portable-workstation-runtime.exe");
        let runtime_root = temporary.path().join("local-ntfs/runtime");
        fs::write(&source, b"runtime-v1").unwrap();

        let destination = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();

        assert_eq!(
            destination,
            runtime_root
                .join(crate::RUNTIME_BUILD_ID)
                .join("codex")
                .join("workstation-runtime.exe")
        );
        assert_eq!(fs::read(destination).unwrap(), b"runtime-v1");
    }

    #[test]
    fn supervisor_runtime_is_isolated_per_engine() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("portable-workstation-runtime.exe");
        let runtime_root = temporary.path().join("local-ntfs/runtime");
        fs::write(&source, b"runtime-v1").unwrap();

        let codex = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();
        let claude = stage_supervisor_binary(&source, &runtime_root, Engine::Claude).unwrap();
        let openclaw = stage_supervisor_binary(&source, &runtime_root, Engine::OpenClaw).unwrap();

        assert_ne!(codex, claude);
        assert_ne!(codex, openclaw);
        assert_ne!(claude, openclaw);
        assert_eq!(fs::read(codex).unwrap(), b"runtime-v1");
        assert_eq!(fs::read(claude).unwrap(), b"runtime-v1");
        assert_eq!(fs::read(openclaw).unwrap(), b"runtime-v1");
    }

    #[test]
    fn existing_supervisor_runtime_is_reused_without_replacement() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("portable-workstation-runtime.exe");
        let runtime_root = temporary.path().join("local-ntfs/runtime");
        fs::write(&source, b"runtime-v1").unwrap();

        let first = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();
        let first_modified = fs::metadata(&first).unwrap().modified().unwrap();
        let second = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();

        assert_eq!(first, second);
        assert_eq!(fs::read(second).unwrap(), b"runtime-v1");
        assert_eq!(
            fs::metadata(first).unwrap().modified().unwrap(),
            first_modified
        );
    }

    #[test]
    fn legacy_supervisor_temporary_files_are_cleaned_without_touching_binaries() {
        let temporary = tempdir().unwrap();
        let runtime_root = temporary.path().join("runtime");
        fs::create_dir_all(&runtime_root).unwrap();
        let stale = runtime_root.join("workstation-runtime-old.exe.7.tmp");
        let binary = runtime_root.join("workstation-runtime-old.exe");
        let unrelated = runtime_root.join("keep.tmp");
        fs::write(&stale, b"partial").unwrap();
        fs::write(&binary, b"runtime").unwrap();
        fs::write(&unrelated, b"keep").unwrap();

        assert_eq!(
            cleanup_legacy_supervisor_temporaries(&runtime_root).unwrap(),
            1
        );
        assert!(!stale.exists());
        assert!(binary.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn codex_arg0_cleanup_only_removes_known_temporary_directories() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_engine_layout(Engine::Codex).unwrap();
        let arg0 = paths.codex_machine_home().join("tmp/arg0");
        let stale = arg0.join("codex-arg0Stale");
        let unrelated = arg0.join("keep-this-directory");
        fs::create_dir_all(&stale).unwrap();
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(stale.join("payload.exe"), b"temporary").unwrap();
        fs::write(unrelated.join("user.txt"), b"keep").unwrap();

        assert_eq!(cleanup_codex_arg0_temp(&paths).unwrap(), 1);
        assert!(!stale.exists());
        assert!(unrelated.join("user.txt").is_file());
    }

    #[test]
    fn openclaw_stale_session_lock_is_backed_up_before_launch() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        let sessions = paths.openclaw_home.join("agents/main/sessions");
        fs::create_dir_all(&sessions).unwrap();
        let stale = sessions.join("thread.jsonl.lock");
        fs::write(&stale, r#"{"pid":4294967295}"#).unwrap();

        assert_eq!(prepare_openclaw_session_locks(&paths).unwrap(), 1);
        assert!(!stale.exists());
        assert!(paths.backups.join("openclaw-stale-locks").is_dir());
    }

    #[test]
    fn openclaw_session_index_paths_are_normalized_to_portable_state() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();
        let sessions = paths.openclaw_home.join("agents/main/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(sessions.join("thread.jsonl"), b"conversation\n").unwrap();
        fs::write(
            sessions.join("sessions.json"),
            br#"{
  "agent:main:main": {
    "sessionId": "thread",
    "sessionFile": "C:\\Users\\OldMachine\\AppData\\Local\\YunshuAI\\state\\thread.jsonl"
  }
}"#,
        )
        .unwrap();

        assert_eq!(normalize_openclaw_session_index_paths(&paths).unwrap(), 1);
        let index: Value =
            serde_json::from_slice(&fs::read(sessions.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(
            index["agent:main:main"]["sessionFile"].as_str(),
            Some(sessions.join("thread.jsonl").to_string_lossy().as_ref())
        );
    }

    #[test]
    fn openclaw_machine_state_filters_locks_and_syncs_sessions_back() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        let sessions = paths.openclaw_home.join("agents/main/sessions");
        fs::create_dir_all(&sessions).unwrap();
        let session = sessions.join("thread.jsonl");
        fs::write(&session, b"portable\n").unwrap();
        fs::write(sessions.join("thread.jsonl.lock"), b"stale").unwrap();

        let machine_cache = temporary.path().join("machine-cache");
        prepare_openclaw_machine_state(&paths, &machine_cache).unwrap();
        let machine_session = machine_cache.join("state/agents/main/sessions/thread.jsonl");
        assert_eq!(fs::read(&machine_session).unwrap(), b"portable\n");
        assert!(
            !machine_cache
                .join("state/agents/main/sessions/thread.jsonl.lock")
                .exists()
        );

        let first_modified = fs::metadata(&machine_session).unwrap().modified().unwrap();
        let local_lock = machine_cache.join("state/agents/main/sessions/local.jsonl.lock");
        fs::write(&local_lock, b"stale-local-lock").unwrap();
        thread::sleep(Duration::from_millis(10));
        prepare_openclaw_machine_state(&paths, &machine_cache).unwrap();
        assert!(!local_lock.exists());
        assert_eq!(
            fs::metadata(&machine_session).unwrap().modified().unwrap(),
            first_modified,
            "未变更的本机状态不应在每次启动时被重写"
        );

        fs::write(&machine_session, b"portable\nassistant\n").unwrap();
        sync_openclaw_machine_state_to_portable(&paths, &machine_cache).unwrap();
        assert_eq!(fs::read(&session).unwrap(), b"portable\nassistant\n");
    }

    #[test]
    fn openclaw_machine_payload_is_versioned_and_reused() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        let core = paths
            .openclaw_entry
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();
        let package_json = core.join("package.json");
        let package_lock = core.join("package-lock.json");
        for (path, content) in [
            (&paths.openclaw_node, b"node".as_slice()),
            (&paths.openclaw_entry, b"entry".as_slice()),
            (&package_json, br#"{"name":"openclaw","version":"1.0.0"}"#),
            (&package_lock, br#"{"lockfileVersion":3}"#),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        let machine_cache = temporary.path().join("machine-cache");
        let shared_payload = temporary.path().join("shared-payload");
        let first = prepare_openclaw_machine_payload_with_roots(
            &paths,
            &machine_cache,
            &shared_payload,
            None,
        )
        .unwrap();
        let first_modified = fs::metadata(&first.entry).unwrap().modified().unwrap();
        let second = prepare_openclaw_machine_payload_with_roots(
            &paths,
            &machine_cache,
            &shared_payload,
            None,
        )
        .unwrap();

        assert_eq!(first.node, second.node);
        assert_eq!(first.entry, second.entry);
        assert_eq!(
            fs::metadata(second.entry).unwrap().modified().unwrap(),
            first_modified
        );
    }

    #[test]
    fn openclaw_payload_archive_is_used_for_first_machine_initialization() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        let core = paths
            .openclaw_entry
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();
        for (path, content) in [
            (&paths.openclaw_node, b"node".as_slice()),
            (&paths.openclaw_entry, b"entry".as_slice()),
            (&core.join("package.json"), br#"{"name":"openclaw"}"#),
            (&core.join("package-lock.json"), br#"{"lockfileVersion":3}"#),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        let packed = pack_openclaw_payload(&paths, None).unwrap();
        assert_eq!(packed.file_count, 4);
        assert!(paths.openclaw_payload_archive().is_file());

        // 打包后再加入的文件不能进入本机镜像，以此证明初始化使用了单文件ZIP，
        // 而不是重新从移动盘逐文件复制整个core。
        fs::write(core.join("added-after-pack.txt"), b"must-not-be-copied").unwrap();
        let shared_payload = temporary.path().join("shared-payload");
        let prepared = prepare_openclaw_machine_payload_with_roots(
            &paths,
            &temporary.path().join("machine-cache"),
            &shared_payload,
            None,
        )
        .unwrap();

        assert!(prepared.node.is_file());
        assert!(prepared.entry.is_file());
        assert!(
            !prepared
                .entry
                .ancestors()
                .nth(3)
                .unwrap()
                .join("added-after-pack.txt")
                .exists()
        );
    }

    #[test]
    fn openclaw_payload_fingerprint_depends_on_content_not_modified_time() {
        let temporary = tempdir().unwrap();
        let core = temporary.path().join("core");
        let entry = core.join("node_modules/openclaw/openclaw.mjs");
        let node = temporary.path().join("node-win-x64/node.exe");
        for (path, content) in [
            (&node, b"node".as_slice()),
            (&entry, b"entry".as_slice()),
            (&core.join("package.json"), br#"{"name":"openclaw"}"#),
            (&core.join("package-lock.json"), br#"{"lockfileVersion":3}"#),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        let first = openclaw_payload_fingerprint(&core, &entry, &node).unwrap();
        let relocated_core = temporary.path().join("relocated/core");
        let relocated_entry = relocated_core.join("node_modules/openclaw/openclaw.mjs");
        let relocated_node = temporary.path().join("relocated/node-win-x64/node.exe");
        for (path, content) in [
            (&relocated_node, b"node".as_slice()),
            (&relocated_entry, b"entry".as_slice()),
            (
                &relocated_core.join("package.json"),
                br#"{"name":"openclaw"}"#,
            ),
            (
                &relocated_core.join("package-lock.json"),
                br#"{"lockfileVersion":3}"#,
            ),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let relocated =
            openclaw_payload_fingerprint(&relocated_core, &relocated_entry, &relocated_node)
                .unwrap();
        assert_eq!(first, relocated);
        thread::sleep(Duration::from_millis(10));
        fs::write(&node, b"node").unwrap();
        let same_content = openclaw_payload_fingerprint(&core, &entry, &node).unwrap();
        fs::write(&node, b"node-v2").unwrap();
        let changed_content = openclaw_payload_fingerprint(&core, &entry, &node).unwrap();

        assert_eq!(first, same_content);
        assert_ne!(first, changed_content);
    }

    #[test]
    fn openclaw_machine_payload_reuses_complete_legacy_cache() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_layout().unwrap();
        let core = paths
            .openclaw_entry
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();
        for (path, content) in [
            (&paths.openclaw_node, b"node".as_slice()),
            (&paths.openclaw_entry, b"entry".as_slice()),
            (&core.join("package.json"), br#"{"name":"openclaw"}"#),
            (&core.join("package-lock.json"), br#"{"lockfileVersion":3}"#),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let legacy_payload = temporary.path().join("legacy-payload");
        let legacy = prepare_openclaw_machine_payload_with_roots(
            &paths,
            &temporary.path().join("old-machine"),
            &legacy_payload,
            None,
        )
        .unwrap();
        let new_shared_payload = temporary.path().join("new-shared-payload");
        let reused = prepare_openclaw_machine_payload_with_roots(
            &paths,
            &temporary.path().join("new-machine"),
            &new_shared_payload,
            Some(&legacy_payload),
        )
        .unwrap();

        assert_eq!(reused.node, legacy.node);
        assert_eq!(reused.entry, legacy.entry);
        assert!(!new_shared_payload.exists());
    }

    #[test]
    fn stale_start_with_dead_supervisor_is_normalized_immediately() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("dev2"));
        paths.prepare_engine_layout(Engine::OpenClaw).unwrap();
        let mut state = stopped_state(&paths, Engine::OpenClaw);
        state.status = RuntimeStatus::Starting;
        state.supervisor_pid = 999_999;
        state.updated_at_unix_ms = now_unix_ms();
        state.message = Some("监督进程已启动".to_string());
        write_state(&paths, &state).unwrap();

        let normalized = status(&paths, Engine::OpenClaw).unwrap();

        assert!(matches!(normalized.status, RuntimeStatus::Stopped));
        assert_eq!(normalized.supervisor_pid, 0);
        assert_eq!(
            normalized.message.as_deref(),
            Some("检测到旧状态文件，实际进程已经退出")
        );
    }

    #[test]
    fn same_length_stale_supervisor_runtime_is_replaced_by_content() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("portable-workstation-runtime.exe");
        let runtime_root = temporary.path().join("local-ntfs/runtime");
        fs::write(&source, b"runtime-v1").unwrap();

        let destination = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();
        fs::write(&source, b"runtime-v2").unwrap();
        assert_eq!(
            fs::metadata(&source).unwrap().len(),
            fs::metadata(&destination).unwrap().len()
        );

        let replaced = stage_supervisor_binary(&source, &runtime_root, Engine::Codex).unwrap();

        assert_eq!(replaced, destination);
        assert_eq!(fs::read(replaced).unwrap(), b"runtime-v2");
    }

    #[test]
    fn openclaw_health_wait_is_longer_until_gateway_warms() {
        let temporary = tempdir().unwrap();

        assert_eq!(
            openclaw_health_wait(temporary.path()),
            OPENCLAW_COLD_HEALTH_WAIT
        );
        fs::write(openclaw_warm_marker(temporary.path()), b"ready\n").unwrap();
        assert_eq!(openclaw_health_wait(temporary.path()), OPENCLAW_HEALTH_WAIT);
    }

    #[test]
    fn openclaw_control_ui_detection_ignores_old_connections() {
        let temporary = tempdir().unwrap();
        let log_root = temporary.path().join("temp/openclaw");
        fs::create_dir_all(&log_root).unwrap();
        let log = log_root.join("openclaw-test.log");
        fs::write(&log, b"client=openclaw-control-ui\n").unwrap();
        let checkpoint = openclaw_control_ui_checkpoint(temporary.path());

        assert!(!openclaw_control_ui_connected_since(
            temporary.path(),
            &checkpoint
        ));
        OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"client=openclaw-control-ui\n")
            .unwrap();
        assert!(openclaw_control_ui_connected_since(
            temporary.path(),
            &checkpoint
        ));
    }
}
