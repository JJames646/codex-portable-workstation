use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;
use yunshu_workstation_runtime::bridge;
use yunshu_workstation_runtime::paths::{WorkstationPaths, available_disk_space};
use yunshu_workstation_runtime::runtime;
use yunshu_workstation_runtime::vault::PortableVault;
use yunshu_workstation_runtime::windows_process::replace_file_atomic;

use crate::chat;
use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};
use crate::imagegen::{self, ImageGenerationRuntime};
use crate::license::INSTALLATION_PRIVATE_KEY_REF;

const PREVIEW_TTL_MS: i64 = 10 * 60 * 1_000;
const MIN_FREE_MARGIN: u64 = 64 * 1024 * 1024;
const FACTORY_SCHEMA_VERSION: u32 = 2;
const STATUS_SCHEMA_VERSION: u32 = 1;
const RECOVERY_SCHEMA_VERSION: u32 = 1;
const MANAGED_OPENCLAW_RUNTIME_LINK_DIR: &str = "data/.openclaw/plugin-skills";
const REQUIRED_TARGETS: [&str; 7] = [
    "codexHome",
    "codexBrowserData",
    "claudeHome",
    "openClawHome",
    "chatData",
    "imageStudioData",
    "uiData",
];

const CLEAN_ITEMS: [&str; 10] = [
    "三引擎的项目、会话、账号状态和个人配置",
    "工作区内的个人项目",
    "三引擎、AI 对话和 AI 生图的供应商与 API Key",
    "AI 对话历史、附件和 AI 生图历史、图片与引用",
    "用户安装的 Skills 与插件状态",
    "Codex 代理订阅、节点和持续模式",
    "界面偏好与公告已读、公告缓存",
    "日志、缓存和运行状态",
    "旧的用户备份",
    "Codex 当前电脑的旧运行镜像关联",
];

const PRESERVE_ITEMS: [&str; 9] = [
    "工作站安装 ID 与当前授权",
    "授权安装私钥和安装元数据",
    "三引擎程序主体、启动器和 Runtime",
    "Mihomo 主体、许可证与对应源码",
    "正式预装 Skills、插件和离线载荷",
    "教程、中文资源和产品目录结构",
    "只读 Factory/Seeds 出厂资源",
    "恢复点（仅失败时保留）",
    "工作站目录之外的任何文件",
];

#[derive(Debug, Clone, Default)]
pub struct FactoryResetRuntime {
    active: Arc<AtomicBool>,
    preview: Arc<Mutex<Option<StoredPreview>>>,
}

impl FactoryResetRuntime {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
struct StoredPreview {
    preview: FactoryResetPreview,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactorySeedStatus {
    pub state: FactorySeedState,
    pub message: String,
    pub manifest_path: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub runtime_build_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactorySeedState {
    Ready,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactoryResetPreview {
    pub preview_id: String,
    pub expires_at_unix_ms: i64,
    pub seed: FactorySeedStatus,
    pub clean: Vec<String>,
    pub preserve: Vec<String>,
    pub blockers: Vec<String>,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub can_execute: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactoryResetExecuteInput {
    pub preview_id: String,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactoryResetRollbackInput {
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FactoryResetStatus {
    pub schema_version: u32,
    pub operation_id: Option<String>,
    pub state: FactoryResetState,
    pub current_step: Option<FactoryResetStep>,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub message: String,
    pub error: Option<String>,
    pub can_rollback: bool,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryResetState {
    Idle,
    Running,
    Completed,
    Failed,
    RollingBack,
    RolledBack,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FactoryResetStep {
    StopServices,
    CreateRestorePoint,
    CleanUserData,
    RestoreFactoryDefaults,
    IntegrityCheck,
}

impl FactoryResetStatus {
    fn idle() -> Self {
        Self {
            schema_version: STATUS_SCHEMA_VERSION,
            operation_id: None,
            state: FactoryResetState::Idle,
            current_step: None,
            completed_steps: 0,
            total_steps: 5,
            message: "尚未执行恢复出厂设置".to_string(),
            error: None,
            can_rollback: false,
            updated_at_unix_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactorySeedManifest {
    schema_version: u32,
    runtime_build_id: String,
    codex_sha256: String,
    component_hashes: FactoryComponentHashes,
    roots: Vec<FactorySeedRoot>,
    files: Vec<FactorySeedFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryComponentHashes {
    codex_desktop: String,
    claude_code: String,
    openclaw_node: String,
    openclaw_entry: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactorySeedRoot {
    source: String,
    target_root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactorySeedFile {
    source: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug)]
struct VerifiedSeed {
    status: FactorySeedStatus,
    manifest: FactorySeedManifest,
    manifest_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct FactorySeedIdentity {
    pub manifest_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryManifest {
    schema_version: u32,
    operation_id: String,
    entries: Vec<RecoveryEntry>,
    files: Vec<RecoveryFileHash>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryEntry {
    relative_path: String,
    existed: bool,
    kind: RecoveryEntryKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryFileHash {
    relative_path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum RecoveryEntryKind {
    Directory,
    File,
}

#[tauri::command]
pub async fn factory_seed_verify(state: State<'_, AppState>) -> CommandResult<FactorySeedStatus> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || inspect_seed(&paths))
        .await
        .map_err(|error| CommandError::from_message(format!("恢复资源校验任务异常：{error}")))
}

#[tauri::command]
pub fn factory_reset_preview(
    state: State<'_, AppState>,
    runtime_state: State<'_, FactoryResetRuntime>,
) -> CommandResult<FactoryResetPreview> {
    if runtime_state.is_active() {
        return Err(CommandError::from_message("恢复事务正在执行，不能重复预览"));
    }
    if matches!(
        read_status(&state.paths).map_err(CommandError::from)?.state,
        FactoryResetState::Running | FactoryResetState::RollingBack
    ) {
        return Err(CommandError::from_message(
            "检测到未完成的恢复事务，请先检查状态并从恢复点回滚",
        ));
    }
    let preview = build_preview(&state.paths).map_err(CommandError::from)?;
    let mut stored = runtime_state
        .preview
        .lock()
        .map_err(|_| CommandError::from_message("恢复预览状态异常"))?;
    *stored = Some(StoredPreview {
        preview: preview.clone(),
    });
    Ok(preview)
}

#[tauri::command]
pub fn factory_reset_status(
    state: State<'_, AppState>,
    reset_runtime: State<'_, FactoryResetRuntime>,
) -> FactoryResetStatus {
    let mut status = read_status(&state.paths).unwrap_or_else(|error| FactoryResetStatus {
        state: FactoryResetState::Failed,
        message: "无法读取恢复事务状态".to_string(),
        error: Some(error.to_string()),
        ..FactoryResetStatus::idle()
    });
    if !reset_runtime.is_active()
        && matches!(
            status.state,
            FactoryResetState::Running | FactoryResetState::RollingBack
        )
    {
        status.state = FactoryResetState::Failed;
        status.message = "检测到上次未完成的恢复事务".to_string();
        status.error = Some("工作站在恢复过程中意外关闭，请从本次恢复点回滚".to_string());
        status.can_rollback = status.operation_id.as_deref().is_some_and(|operation_id| {
            recovery_dir(&state.paths, operation_id)
                .join("manifest.json")
                .is_file()
        });
        status.updated_at_unix_ms = now_ms();
        let _ = write_status(&state.paths, &status);
    }
    status
}

#[tauri::command]
pub async fn factory_reset_execute(
    app: State<'_, AppState>,
    reset_runtime: State<'_, FactoryResetRuntime>,
    image_runtime: State<'_, ImageGenerationRuntime>,
    input: FactoryResetExecuteInput,
) -> CommandResult<FactoryResetStatus> {
    if !input.acknowledged {
        return Err(CommandError::from_message("必须勾选确认后才能恢复出厂设置"));
    }
    if reset_runtime
        .active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(CommandError::from_message("恢复事务正在执行，请勿重复操作"));
    }
    let _active_guard = ActiveResetGuard(reset_runtime.active.clone());
    let preview = {
        let mut stored = reset_runtime
            .preview
            .lock()
            .map_err(|_| CommandError::from_message("恢复预览状态异常"))?;
        let preview = stored
            .take()
            .ok_or_else(|| CommandError::from_message("恢复预览不存在，请重新预览"))?
            .preview;
        if preview.preview_id != input.preview_id {
            return Err(CommandError::from_message("恢复预览已变化，请重新预览"));
        }
        preview
    };
    if preview.expires_at_unix_ms < now_ms() {
        return Err(CommandError::from_message(
            "恢复预览已超过十分钟，请重新预览",
        ));
    }
    if !preview.can_execute {
        return Err(CommandError::from_message(preview.blockers.join("；")));
    }

    let operation_id = Uuid::new_v4().to_string();
    let paths = app.paths.clone();
    let mut status = running_status(&operation_id, FactoryResetStep::StopServices, 0);
    write_status(&paths, &status).map_err(CommandError::from)?;

    let result = execute_transaction(&paths, &image_runtime, &mut status).await;
    match result {
        Ok(()) => {
            status.state = FactoryResetState::Completed;
            status.current_step = None;
            status.completed_steps = 5;
            status.message = "恢复出厂设置已完成".to_string();
            status.error = None;
            status.can_rollback = false;
            status.updated_at_unix_ms = now_ms();
            write_status(&paths, &status).map_err(CommandError::from)?;
            Ok(status)
        }
        Err(error) => {
            let recovery = recovery_dir(&paths, &operation_id);
            status.state = FactoryResetState::Failed;
            status.message = "恢复出厂设置失败".to_string();
            status.error = Some(error.to_string());
            status.can_rollback = recovery.join("manifest.json").is_file();
            status.updated_at_unix_ms = now_ms();
            let _ = write_status(&paths, &status);
            Err(CommandError::from(error))
        }
    }
}

#[tauri::command]
pub async fn factory_reset_rollback(
    app: State<'_, AppState>,
    reset_runtime: State<'_, FactoryResetRuntime>,
    input: FactoryResetRollbackInput,
) -> CommandResult<FactoryResetStatus> {
    if reset_runtime
        .active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(CommandError::from_message("恢复事务正在执行，请稍后再试"));
    }
    let _active_guard = ActiveResetGuard(reset_runtime.active.clone());
    let paths = app.paths.clone();
    let current = read_status(&paths).map_err(CommandError::from)?;
    if current.state != FactoryResetState::Failed
        || !current.can_rollback
        || current.operation_id.as_deref() != Some(input.operation_id.trim())
    {
        return Err(CommandError::from_message("当前没有可回滚的恢复事务"));
    }
    let mut status = current;
    status.state = FactoryResetState::RollingBack;
    status.message = "正在从恢复点回滚".to_string();
    status.updated_at_unix_ms = now_ms();
    write_status(&paths, &status).map_err(CommandError::from)?;

    let operation_id = input.operation_id.trim().to_string();
    let restore_paths = paths.clone();
    tauri::async_runtime::spawn_blocking(move || rollback_recovery(&restore_paths, &operation_id))
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)?;

    status.state = FactoryResetState::RolledBack;
    status.current_step = None;
    status.message = "已从恢复点回滚".to_string();
    status.error = None;
    status.can_rollback = false;
    status.updated_at_unix_ms = now_ms();
    write_status(&paths, &status).map_err(CommandError::from)?;
    Ok(status)
}

struct ActiveResetGuard(Arc<AtomicBool>);

impl Drop for ActiveResetGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

async fn execute_transaction(
    paths: &WorkstationPaths,
    image_runtime: &ImageGenerationRuntime,
    status: &mut FactoryResetStatus,
) -> Result<()> {
    let seed = verify_seed(paths)?;
    let preview = build_preview(paths)?;
    if !preview.can_execute {
        bail!("{}", preview.blockers.join("；"));
    }
    if chat::active_request_count() > 0 {
        bail!("AI 对话仍在生成，请等待完成后再恢复");
    }
    image_runtime.cancel_all();
    for _ in 0..50 {
        if image_runtime.active_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if image_runtime.active_count() > 0 {
        bail!("AI 生图任务未能受控停止，请稍后重试");
    }

    let stop_paths = paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        runtime::stop_all(&stop_paths, Duration::from_secs(75)).map(|_| ())
    })
    .await
    .context("停止三引擎任务异常")??;
    bridge::stop(paths).await?;
    complete_step(paths, status, FactoryResetStep::CreateRestorePoint, 1)?;

    let operation_id = status
        .operation_id
        .as_deref()
        .ok_or_else(|| anyhow!("恢复事务缺少 operation-id"))?
        .to_string();
    let backup_paths = paths.clone();
    let backup_operation = operation_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        create_recovery_point(&backup_paths, &backup_operation)
    })
    .await
    .context("创建恢复点任务异常")??;
    complete_step(paths, status, FactoryResetStep::CleanUserData, 2)?;

    let clean_paths = paths.clone();
    tauri::async_runtime::spawn_blocking(move || clean_user_data(&clean_paths))
        .await
        .context("清理用户数据任务异常")??;
    complete_step(paths, status, FactoryResetStep::RestoreFactoryDefaults, 3)?;

    let restore_paths = paths.clone();
    tauri::async_runtime::spawn_blocking(move || restore_seed(&restore_paths, &seed))
        .await
        .context("恢复出厂默认数据任务异常")??;
    yunshu_workstation_runtime::locale_init::apply_seed(paths)
        .context("恢复 Codex 中文默认状态失败")?;
    chat::initialize_storage(paths)?;
    imagegen::initialize_storage(paths)?;
    complete_step(paths, status, FactoryResetStep::IntegrityCheck, 4)?;

    verify_post_reset(paths)?;
    let recovery = recovery_dir(paths, &operation_id);
    fs::remove_dir_all(&recovery)
        .with_context(|| format!("恢复完成但无法删除临时恢复点 {}", recovery.display()))?;
    Ok(())
}

fn complete_step(
    paths: &WorkstationPaths,
    status: &mut FactoryResetStatus,
    next_step: FactoryResetStep,
    completed_steps: usize,
) -> Result<()> {
    status.current_step = Some(next_step);
    status.completed_steps = completed_steps;
    status.message = step_message(next_step).to_string();
    status.updated_at_unix_ms = now_ms();
    write_status(paths, status)
}

fn running_status(
    operation_id: &str,
    current_step: FactoryResetStep,
    completed_steps: usize,
) -> FactoryResetStatus {
    FactoryResetStatus {
        schema_version: STATUS_SCHEMA_VERSION,
        operation_id: Some(operation_id.to_string()),
        state: FactoryResetState::Running,
        current_step: Some(current_step),
        completed_steps,
        total_steps: 5,
        message: step_message(current_step).to_string(),
        error: None,
        can_rollback: false,
        updated_at_unix_ms: now_ms(),
    }
}

fn step_message(step: FactoryResetStep) -> &'static str {
    match step {
        FactoryResetStep::StopServices => "正在停止服务",
        FactoryResetStep::CreateRestorePoint => "正在生成恢复点",
        FactoryResetStep::CleanUserData => "正在清理用户数据",
        FactoryResetStep::RestoreFactoryDefaults => "正在恢复出厂默认数据",
        FactoryResetStep::IntegrityCheck => "正在执行完整性检查",
    }
}

fn inspect_seed(paths: &WorkstationPaths) -> FactorySeedStatus {
    match verify_seed(paths) {
        Ok(seed) => seed.status,
        Err(error) => {
            let manifest = factory_manifest_path(paths);
            let missing = !manifest.is_file();
            FactorySeedStatus {
                state: if missing {
                    FactorySeedState::Missing
                } else {
                    FactorySeedState::Invalid
                },
                message: if missing {
                    "恢复资源未就绪".to_string()
                } else {
                    format!("恢复资源校验失败：{error}")
                },
                manifest_path: manifest.display().to_string(),
                file_count: 0,
                total_bytes: 0,
                runtime_build_id: None,
            }
        }
    }
}

fn verify_seed(paths: &WorkstationPaths) -> Result<VerifiedSeed> {
    validate_product_root(paths)?;
    let manifest_path = factory_manifest_path(paths);
    if !manifest_path.is_file() {
        bail!("恢复资源未就绪：缺少 {}", manifest_path.display());
    }
    validate_path_under_root(&paths.root, &manifest_path)?;
    let manifest: FactorySeedManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("无法读取出厂种子清单 {}", manifest_path.display()))?,
    )
    .context("出厂种子清单无法解析")?;
    if manifest.schema_version != FACTORY_SCHEMA_VERSION {
        bail!("出厂种子清单版本不支持");
    }
    if manifest.runtime_build_id != yunshu_workstation_runtime::RUNTIME_BUILD_ID {
        bail!("出厂种子与当前 Runtime Build ID 不一致");
    }
    let codex_hash = sha256_file(&paths.codex_desktop)?;
    if !eq_sha256(&manifest.codex_sha256, &codex_hash) {
        bail!("出厂种子与当前 Codex 主体不匹配");
    }
    for (name, expected, executable) in [
        (
            "Codex 桌面端",
            &manifest.component_hashes.codex_desktop,
            &paths.codex_desktop,
        ),
        (
            "Claude Code",
            &manifest.component_hashes.claude_code,
            &paths.claude_exe,
        ),
        (
            "OpenClaw Node",
            &manifest.component_hashes.openclaw_node,
            &paths.openclaw_node,
        ),
        (
            "OpenClaw 入口",
            &manifest.component_hashes.openclaw_entry,
            &paths.openclaw_entry,
        ),
    ] {
        let actual = sha256_file(executable).with_context(|| format!("无法校验{name}主体"))?;
        if !eq_sha256(expected, &actual) {
            bail!("出厂种子与当前{name}主体不匹配");
        }
    }
    if manifest.roots.len() != REQUIRED_TARGETS.len() {
        bail!("出厂种子必须声明七个默认数据根目录");
    }
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("出厂种子清单目录无效"))?
        .to_path_buf();
    let mut target_names = BTreeSet::new();
    let mut declared_files = BTreeMap::<String, &FactorySeedFile>::new();
    for root in &manifest.roots {
        safe_relative(&root.source)?;
        if !REQUIRED_TARGETS.contains(&root.target_root.as_str())
            || !target_names.insert(root.target_root.clone())
        {
            bail!("出厂种子目标目录重复或无效：{}", root.target_root);
        }
        let source = manifest_dir.join(&root.source);
        validate_path_under_root(&paths.root, &source)?;
        if !source.is_dir() {
            bail!("出厂种子目录不存在：{}", source.display());
        }
    }
    for target in REQUIRED_TARGETS {
        if !target_names.contains(target) {
            bail!("出厂种子缺少目标目录：{target}");
        }
    }
    let mut total_bytes = 0_u64;
    for file in &manifest.files {
        safe_relative(&file.source)?;
        if declared_files.insert(file.source.clone(), file).is_some() {
            bail!("出厂种子文件重复：{}", file.source);
        }
        let source = manifest_dir.join(&file.source);
        validate_path_under_root(&paths.root, &source)?;
        let metadata = fs::metadata(&source)
            .with_context(|| format!("出厂种子文件不存在：{}", source.display()))?;
        if !metadata.is_file() || metadata.len() != file.bytes {
            bail!("出厂种子文件大小不一致：{}", file.source);
        }
        let hash = sha256_file(&source)?;
        if !eq_sha256(&file.sha256, &hash) {
            bail!("出厂种子文件哈希不一致：{}", file.source);
        }
        total_bytes = total_bytes.saturating_add(file.bytes);
    }
    let mut actual_files = BTreeSet::new();
    for root in &manifest.roots {
        let source_root = manifest_dir.join(&root.source);
        for file in list_files(&source_root)? {
            let relative = file
                .strip_prefix(&manifest_dir)
                .context("出厂种子文件越出清单目录")?;
            actual_files.insert(relative_to_manifest(relative)?);
        }
    }
    let declared = declared_files.keys().cloned().collect::<BTreeSet<_>>();
    if actual_files != declared {
        bail!("出厂种子清单与目录中的实际文件不一致");
    }
    if manifest.files.is_empty() {
        bail!("出厂种子清单不能是空清单");
    }
    Ok(VerifiedSeed {
        status: FactorySeedStatus {
            state: FactorySeedState::Ready,
            message: "恢复资源已就绪".to_string(),
            manifest_path: manifest_path.display().to_string(),
            file_count: manifest.files.len(),
            total_bytes,
            runtime_build_id: Some(manifest.runtime_build_id.clone()),
        },
        manifest,
        manifest_dir,
    })
}

pub(crate) fn verify_factory_seed_identity(
    paths: &WorkstationPaths,
) -> Result<FactorySeedIdentity> {
    verify_seed(paths)?;
    Ok(FactorySeedIdentity {
        manifest_sha256: sha256_file(&factory_manifest_path(paths))?,
    })
}

pub(crate) fn restore_factory_seed_defaults(
    paths: &WorkstationPaths,
) -> Result<FactorySeedIdentity> {
    let verified = verify_seed(paths)?;
    let identity = FactorySeedIdentity {
        manifest_sha256: sha256_file(&factory_manifest_path(paths))?,
    };
    restore_seed(paths, &verified)?;
    Ok(identity)
}

fn build_preview(paths: &WorkstationPaths) -> Result<FactoryResetPreview> {
    let seed_status = inspect_seed(paths);
    let backup_bytes = mutable_data_size(paths)?;
    let required_bytes = backup_bytes
        .saturating_add(seed_status.total_bytes)
        .saturating_add(
            MIN_FREE_MARGIN.max((backup_bytes.saturating_add(seed_status.total_bytes)) / 10),
        );
    let available_bytes = available_disk_space(&paths.root)?;
    let mut blockers = Vec::new();
    if seed_status.state != FactorySeedState::Ready {
        blockers.push(seed_status.message.clone());
    }
    if available_bytes < required_bytes {
        blockers.push(format!(
            "移动盘空间不足：需要至少 {required_bytes} 字节，当前可用 {available_bytes} 字节"
        ));
    }
    if chat::active_request_count() > 0 {
        blockers.push("AI 对话仍在生成".to_string());
    }
    Ok(FactoryResetPreview {
        preview_id: Uuid::new_v4().to_string(),
        expires_at_unix_ms: now_ms() + PREVIEW_TTL_MS,
        seed: seed_status,
        clean: CLEAN_ITEMS.iter().map(|value| value.to_string()).collect(),
        preserve: PRESERVE_ITEMS
            .iter()
            .map(|value| value.to_string())
            .collect(),
        can_execute: blockers.is_empty(),
        blockers,
        required_bytes,
        available_bytes,
    })
}

fn create_recovery_point(paths: &WorkstationPaths, operation_id: &str) -> Result<()> {
    validate_product_root(paths)?;
    let destination = recovery_dir(paths, operation_id);
    if destination.exists() {
        bail!("恢复点已经存在：{}", destination.display());
    }
    fs::create_dir_all(destination.join("files"))?;
    let mut entries = Vec::new();
    for (relative, kind) in recovery_entries() {
        let source = paths.root.join(&relative);
        validate_path_under_root(&paths.root, &source)?;
        let existed = source.exists();
        if existed {
            let target = destination.join("files").join(&relative);
            match kind {
                RecoveryEntryKind::Directory => {
                    copy_tree_filtered(paths, &source, &target, &relative)?;
                }
                RecoveryEntryKind::File => {
                    copy_file_checked(&source, &target)?;
                }
            }
        }
        entries.push(RecoveryEntry {
            relative_path: relative_to_manifest(&relative)?,
            existed,
            kind,
        });
    }
    let files_root = destination.join("files");
    let mut files = Vec::new();
    for file in list_files(&files_root)? {
        let relative = file.strip_prefix(&files_root)?;
        files.push(RecoveryFileHash {
            relative_path: relative_to_manifest(relative)?,
            bytes: fs::metadata(&file)?.len(),
            sha256: sha256_file(&file)?,
        });
    }
    let manifest = RecoveryManifest {
        schema_version: RECOVERY_SCHEMA_VERSION,
        operation_id: operation_id.to_string(),
        entries,
        files,
    };
    write_json_atomic(&destination.join("manifest.json"), &manifest)
}

fn clean_user_data(paths: &WorkstationPaths) -> Result<()> {
    validate_product_root(paths)?;
    unlink_managed_openclaw_runtime_links(paths)?;
    for directory in reset_directories(paths) {
        reset_directory(paths, &directory)?;
    }
    clear_directory_except(paths, &paths.runtime_state, Some("factory-reset"))?;
    clear_directory_except(paths, &paths.backups, Some(".factory-reset"))?;
    for file in reset_files(paths) {
        remove_file_checked(paths, &file)?;
    }
    let mut vault = PortableVault::open(paths)?;
    vault.retain_only(&[INSTALLATION_PRIVATE_KEY_REF]);
    vault.save(paths)?;
    paths.rotate_factory_generation_id()?;
    Ok(())
}

fn restore_seed(paths: &WorkstationPaths, seed: &VerifiedSeed) -> Result<()> {
    for root in &seed.manifest.roots {
        let source = seed.manifest_dir.join(&root.source);
        let target = target_root(paths, &root.target_root)?;
        copy_tree(&source, &target)?;
    }
    Ok(())
}

fn verify_post_reset(paths: &WorkstationPaths) -> Result<()> {
    verify_seed(paths)?;
    if paths.read_installation_id().is_none() {
        bail!("恢复后工作站安装 ID 丢失");
    }
    let vault = PortableVault::open(paths)?;
    if !vault.contains(INSTALLATION_PRIVATE_KEY_REF) {
        bail!("恢复后授权安装私钥丢失");
    }
    if !paths.data.join("chat").join("chat.sqlite").is_file()
        || fs::metadata(paths.data.join("chat").join("chat.sqlite"))?.len() == 0
    {
        bail!("恢复后 AI 对话数据库未正确初始化");
    }
    let image_database = paths
        .data
        .join("integrations")
        .join("image-studio")
        .join("history.sqlite");
    if !image_database.is_file() || fs::metadata(&image_database)?.len() == 0 {
        bail!("恢复后 AI 生图数据库未正确初始化");
    }
    paths.ensure_payload_exists()?;
    Ok(())
}

fn rollback_recovery(paths: &WorkstationPaths, operation_id: &str) -> Result<()> {
    validate_product_root(paths)?;
    let recovery = recovery_dir(paths, operation_id);
    let manifest_path = recovery.join("manifest.json");
    let manifest: RecoveryManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema_version != RECOVERY_SCHEMA_VERSION || manifest.operation_id != operation_id {
        bail!("恢复点清单与当前事务不匹配");
    }
    verify_recovery_files(&recovery, &manifest)?;
    for entry in &manifest.entries {
        let relative = safe_relative(&entry.relative_path)?;
        let target = paths.root.join(&relative);
        validate_path_under_root(&paths.root, &target)?;
        if relative == Path::new("backups") {
            clear_directory_except(paths, &target, Some(".factory-reset"))?;
            if entry.existed {
                copy_tree(&recovery.join("files").join(&relative), &target)?;
            }
            continue;
        }
        remove_existing(&target)?;
        if entry.existed {
            let source = recovery.join("files").join(&relative);
            match entry.kind {
                RecoveryEntryKind::Directory => copy_tree(&source, &target)?,
                RecoveryEntryKind::File => copy_file_checked(&source, &target)?,
            }
        }
    }
    fs::remove_dir_all(&recovery)?;
    Ok(())
}

fn verify_recovery_files(recovery: &Path, manifest: &RecoveryManifest) -> Result<()> {
    let files_root = recovery.join("files");
    let mut declared = BTreeSet::new();
    for file in &manifest.files {
        let relative = safe_relative(&file.relative_path)?;
        if !declared.insert(file.relative_path.clone()) {
            bail!("恢复点文件清单存在重复项：{}", file.relative_path);
        }
        let path = files_root.join(relative);
        let metadata =
            fs::metadata(&path).with_context(|| format!("恢复点文件缺失：{}", path.display()))?;
        if !metadata.is_file() || metadata.len() != file.bytes {
            bail!("恢复点文件大小校验失败：{}", file.relative_path);
        }
        if !eq_sha256(&file.sha256, &sha256_file(&path)?) {
            bail!("恢复点文件哈希校验失败：{}", file.relative_path);
        }
    }
    let actual = list_files(&files_root)?
        .into_iter()
        .map(|path| {
            relative_to_manifest(
                path.strip_prefix(&files_root)
                    .expect("恢复点文件应位于 files 目录"),
            )
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if actual != declared {
        bail!("恢复点文件清单与实际文件不一致");
    }
    Ok(())
}

fn recovery_entries() -> Vec<(PathBuf, RecoveryEntryKind)> {
    vec![
        (PathBuf::from("data/.codex"), RecoveryEntryKind::Directory),
        (PathBuf::from("data/Codex"), RecoveryEntryKind::Directory),
        (PathBuf::from("data/.claude"), RecoveryEntryKind::Directory),
        (
            PathBuf::from("data/.openclaw"),
            RecoveryEntryKind::Directory,
        ),
        (PathBuf::from("data/chat"), RecoveryEntryKind::Directory),
        (
            PathBuf::from("data/integrations/image-studio"),
            RecoveryEntryKind::Directory,
        ),
        (PathBuf::from("data/skills"), RecoveryEntryKind::Directory),
        (PathBuf::from("data/plugins"), RecoveryEntryKind::Directory),
        (PathBuf::from("data/ui"), RecoveryEntryKind::Directory),
        (PathBuf::from("data/runtime"), RecoveryEntryKind::Directory),
        (PathBuf::from("workspace"), RecoveryEntryKind::Directory),
        (
            PathBuf::from("config/providers"),
            RecoveryEntryKind::Directory,
        ),
        (PathBuf::from("cache"), RecoveryEntryKind::Directory),
        (PathBuf::from("logs"), RecoveryEntryKind::Directory),
        (PathBuf::from("backups"), RecoveryEntryKind::Directory),
        (
            PathBuf::from("config/codex-proxy.json"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("config/codex-init-proxy.json"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("config/codex-init-proxy-once.json"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("data/license/announcement-state.json"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("data/license/latest-announcement.json"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("data/secrets/provider-vault.bin"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("data/secrets/vault.key"),
            RecoveryEntryKind::File,
        ),
        (
            PathBuf::from("data/factory-generation-id"),
            RecoveryEntryKind::File,
        ),
    ]
}

fn reset_directories(paths: &WorkstationPaths) -> Vec<PathBuf> {
    vec![
        paths.codex_home.clone(),
        paths.codex_browser_data.clone(),
        paths.claude_home.clone(),
        paths.openclaw_home.clone(),
        paths.data.join("chat"),
        paths.data.join("integrations").join("image-studio"),
        paths.workspace.clone(),
        paths.provider_config.clone(),
        paths.skills_data.clone(),
        paths.plugins_data.clone(),
        paths.data.join("ui"),
        paths.cache.clone(),
        paths.logs.clone(),
    ]
}

fn reset_files(paths: &WorkstationPaths) -> Vec<PathBuf> {
    vec![
        paths.config.join("codex-proxy.json"),
        paths.config.join("codex-init-proxy.json"),
        paths.config.join("codex-init-proxy-once.json"),
        paths.data.join("license").join("announcement-state.json"),
        paths.data.join("license").join("latest-announcement.json"),
    ]
}

fn target_root(paths: &WorkstationPaths, name: &str) -> Result<PathBuf> {
    match name {
        "codexHome" => Ok(paths.codex_home.clone()),
        "codexBrowserData" => Ok(paths.codex_browser_data.clone()),
        "claudeHome" => Ok(paths.claude_home.clone()),
        "openClawHome" => Ok(paths.openclaw_home.clone()),
        "chatData" => Ok(paths.data.join("chat")),
        "imageStudioData" => Ok(paths.data.join("integrations").join("image-studio")),
        "uiData" => Ok(paths.data.join("ui")),
        _ => bail!("未知出厂种子目标：{name}"),
    }
}

fn mutable_data_size(paths: &WorkstationPaths) -> Result<u64> {
    let mut total = 0_u64;
    for (relative, _) in recovery_entries() {
        let path = paths.root.join(&relative);
        if path.exists() {
            total = total.saturating_add(path_size_filtered(&path, &relative)?);
        }
    }
    Ok(total)
}

fn path_size_filtered(path: &Path, relative: &Path) -> Result<u64> {
    if relative == Path::new(MANAGED_OPENCLAW_RUNTIME_LINK_DIR) {
        return Ok(0);
    }
    let metadata = fs::symlink_metadata(path)?;
    reject_reparse(path, &metadata)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total = total.saturating_add(path_size_filtered(
            &entry.path(),
            &relative.join(entry.file_name()),
        )?);
    }
    Ok(total)
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    if !source.is_dir() {
        fs::create_dir_all(target)?;
        return Ok(());
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        reject_reparse(&source_path, &metadata)?;
        if metadata.is_dir() {
            copy_tree(&source_path, &target_path)?;
        } else if metadata.is_file() {
            copy_file_checked(&source_path, &target_path)?;
        } else {
            bail!("不支持的文件类型：{}", source_path.display());
        }
    }
    Ok(())
}

fn copy_tree_filtered(
    paths: &WorkstationPaths,
    source: &Path,
    target: &Path,
    relative: &Path,
) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if relative == Path::new("backups") && name == ".factory-reset" {
            continue;
        }
        if relative == Path::new("data/runtime") && name == "factory-reset" {
            continue;
        }
        let entry_relative = relative.join(&name);
        if entry_relative == Path::new(MANAGED_OPENCLAW_RUNTIME_LINK_DIR) {
            continue;
        }
        let source_path = entry.path();
        validate_path_under_root(&paths.root, &source_path)?;
        let target_path = target.join(&name);
        let metadata = fs::symlink_metadata(&source_path)?;
        reject_reparse(&source_path, &metadata)?;
        if metadata.is_dir() {
            copy_tree_filtered(paths, &source_path, &target_path, &entry_relative)?;
        } else if metadata.is_file() {
            copy_file_checked(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn copy_file_checked(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    reject_reparse(source, &metadata)?;
    if !metadata.is_file() {
        bail!("不是普通文件：{}", source.display());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)?;
    Ok(())
}

fn reset_directory(paths: &WorkstationPaths, directory: &Path) -> Result<()> {
    validate_path_under_root(&paths.root, directory)?;
    remove_existing(directory)?;
    fs::create_dir_all(directory)?;
    Ok(())
}

fn unlink_managed_openclaw_runtime_links(paths: &WorkstationPaths) -> Result<()> {
    let directory = paths.root.join(MANAGED_OPENCLAW_RUNTIME_LINK_DIR);
    validate_path_under_root(&paths.root, &directory)?;
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    reject_reparse(&directory, &metadata)?;
    unlink_reparse_points_in_managed_directory(&directory)
}

fn unlink_reparse_points_in_managed_directory(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || windows_reparse_point(&metadata) {
            if metadata.is_dir() || windows_directory_attribute(&metadata) {
                fs::remove_dir(&path).with_context(|| {
                    format!("无法解除 OpenClaw 运行时目录联接：{}", path.display())
                })?;
            } else {
                fs::remove_file(&path).with_context(|| {
                    format!("无法解除 OpenClaw 运行时文件联接：{}", path.display())
                })?;
            }
        } else if metadata.is_dir() {
            unlink_reparse_points_in_managed_directory(&path)?;
        }
    }
    Ok(())
}

fn clear_directory_except(
    paths: &WorkstationPaths,
    directory: &Path,
    preserved_name: Option<&str>,
) -> Result<()> {
    validate_path_under_root(&paths.root, directory)?;
    fs::create_dir_all(directory)?;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if preserved_name.is_some_and(|name| entry.file_name() == name) {
            continue;
        }
        remove_existing(&entry.path())?;
    }
    Ok(())
}

fn remove_file_checked(paths: &WorkstationPaths, file: &Path) -> Result<()> {
    validate_path_under_root(&paths.root, file)?;
    if file.exists() {
        let metadata = fs::symlink_metadata(file)?;
        reject_reparse(file, &metadata)?;
        if !metadata.is_file() {
            bail!("预期为文件但实际不是：{}", file.display());
        }
        fs::remove_file(file)?;
    }
    Ok(())
}

fn remove_existing(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    reject_reparse(path, &metadata)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        bail!("不支持删除的文件类型：{}", path.display());
    }
    Ok(())
}

fn validate_product_root(paths: &WorkstationPaths) -> Result<()> {
    if !paths.root.is_absolute() || !paths.root.is_dir() {
        bail!("工作站产品根目录无效：{}", paths.root.display());
    }
    let metadata = fs::symlink_metadata(&paths.root)?;
    reject_reparse(&paths.root, &metadata)
}

fn validate_path_under_root(root: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(root) {
        bail!("目标路径越出工作站目录：{}", path.display());
    }
    let canonical_root = fs::canonicalize(root)?;
    let relative = path.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            bail!("目标路径包含非法组件：{}", path.display());
        };
        current.push(value);
        if current.exists() {
            let metadata = fs::symlink_metadata(&current)?;
            reject_reparse(&current, &metadata)?;
            let canonical = fs::canonicalize(&current)?;
            if !canonical.starts_with(&canonical_root) {
                bail!("目标路径通过重解析点越出工作站目录：{}", current.display());
            }
        }
    }
    Ok(())
}

fn safe_relative(value: &str) -> Result<PathBuf> {
    if value.trim().is_empty() || value.contains('\\') || value.contains(':') {
        bail!("清单相对路径无效：{value}");
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("清单相对路径无效：{value}");
    }
    Ok(path.to_path_buf())
}

fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        reject_reparse(&path, &metadata)?;
        if metadata.is_dir() {
            files.extend(list_files(&path)?);
        } else if metadata.is_file() {
            files.push(path);
        } else {
            bail!("出厂种子包含不支持的文件类型：{}", path.display());
        }
    }
    Ok(files)
}

fn reject_reparse(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || windows_reparse_point(metadata) {
        bail!("不允许通过符号链接或重解析点操作：{}", path.display());
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

#[cfg(windows)]
fn windows_directory_attribute(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x10 != 0
}

#[cfg(not(windows))]
fn windows_directory_attribute(_metadata: &fs::Metadata) -> bool {
    false
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("无法读取文件 {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn eq_sha256(expected: &str, actual: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(actual)
        && expected.trim().len() == 64
        && expected
            .trim()
            .chars()
            .all(|value| value.is_ascii_hexdigit())
}

fn relative_to_manifest(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(value) = component else {
            bail!("无法序列化相对路径：{}", path.display());
        };
        parts.push(value.to_string_lossy().to_string());
    }
    Ok(parts.join("/"))
}

fn factory_manifest_path(paths: &WorkstationPaths) -> PathBuf {
    paths.root.join("Factory").join("manifest.json")
}

fn recovery_dir(paths: &WorkstationPaths, operation_id: &str) -> PathBuf {
    paths.backups.join(".factory-reset").join(operation_id)
}

fn status_file(paths: &WorkstationPaths) -> PathBuf {
    paths
        .runtime_state
        .join("factory-reset")
        .join("status.json")
}

fn read_status(paths: &WorkstationPaths) -> Result<FactoryResetStatus> {
    let file = status_file(paths);
    if !file.is_file() {
        return Ok(FactoryResetStatus::idle());
    }
    serde_json::from_slice(&fs::read(&file)?).context("恢复事务状态文件无法解析")
}

fn write_status(paths: &WorkstationPaths, status: &FactoryResetStatus) -> Result<()> {
    write_json_atomic(&status_file(paths), status)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, path)?;
    Ok(())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use zeroize::Zeroizing;

    #[cfg(windows)]
    fn create_junction(link: &Path, target: &Path) {
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn create_fixture() -> (tempfile::TempDir, WorkstationPaths) {
        let temporary = tempdir().unwrap();
        let root = temporary.path().join("中文 路径");
        let paths = WorkstationPaths::from_root(root);
        paths.prepare_layout().unwrap();
        for executable in [
            &paths.codex_desktop,
            &paths.codex_cli,
            &paths.claude_exe,
            &paths.openclaw_node,
            &paths.openclaw_entry,
            &paths.bundled_node,
            &paths.bundled_python,
        ] {
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::write(executable, b"fixture").unwrap();
        }
        create_seed(&paths);
        (temporary, paths)
    }

    fn create_seed(paths: &WorkstationPaths) {
        let factory = paths.root.join("Factory");
        let roots = [
            ("Seeds/Codex/home", "codexHome"),
            ("Seeds/Codex/browser", "codexBrowserData"),
            ("Seeds/Claude", "claudeHome"),
            ("Seeds/OpenClaw", "openClawHome"),
            ("Seeds/Chat", "chatData"),
            ("Seeds/ImageStudio", "imageStudioData"),
            ("Seeds/UI", "uiData"),
        ];
        let mut files = Vec::new();
        for (index, (source, _)) in roots.iter().enumerate() {
            let directory = factory.join(source);
            fs::create_dir_all(&directory).unwrap();
            let file = directory.join(format!("seed-{index}.json"));
            fs::write(&file, format!("{{\"seed\":{index}}}\n")).unwrap();
            files.push(FactorySeedFile {
                source: relative_to_manifest(file.strip_prefix(&factory).unwrap()).unwrap(),
                bytes: fs::metadata(&file).unwrap().len(),
                sha256: sha256_file(&file).unwrap(),
            });
        }
        let manifest = serde_json::json!({
            "schemaVersion": FACTORY_SCHEMA_VERSION,
            "runtimeBuildId": yunshu_workstation_runtime::RUNTIME_BUILD_ID,
            "codexSha256": sha256_file(&paths.codex_desktop).unwrap(),
            "componentHashes": {
                "codexDesktop": sha256_file(&paths.codex_desktop).unwrap(),
                "claudeCode": sha256_file(&paths.claude_exe).unwrap(),
                "openclawNode": sha256_file(&paths.openclaw_node).unwrap(),
                "openclawEntry": sha256_file(&paths.openclaw_entry).unwrap(),
            },
            "roots": roots.iter().map(|(source, target)| serde_json::json!({
                "source": source,
                "targetRoot": target,
            })).collect::<Vec<_>>(),
            "files": files.iter().map(|file| serde_json::json!({
                "source": file.source,
                "bytes": file.bytes,
                "sha256": file.sha256,
            })).collect::<Vec<_>>(),
        });
        write_json_atomic(&factory.join("manifest.json"), &manifest).unwrap();
    }

    #[test]
    fn seed_verification_accepts_complete_chinese_path_fixture() {
        let (_temporary, paths) = create_fixture();
        let seed = verify_seed(&paths).unwrap();
        assert_eq!(seed.status.state, FactorySeedState::Ready);
        assert_eq!(seed.status.file_count, 7);
    }

    #[test]
    fn seed_verification_rejects_hash_mismatch() {
        let (_temporary, paths) = create_fixture();
        fs::write(
            paths.root.join("Factory/Seeds/Codex/home/seed-0.json"),
            b"changed",
        )
        .unwrap();
        assert!(
            verify_seed(&paths)
                .unwrap_err()
                .to_string()
                .contains("大小不一致")
        );
    }

    #[test]
    fn preview_reports_missing_seed_without_modifying_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("missing"));
        paths.prepare_layout().unwrap();
        let sentinel = paths.workspace.join("keep.txt");
        fs::write(&sentinel, b"keep").unwrap();
        let preview = build_preview(&paths).unwrap();
        assert!(!preview.can_execute);
        assert_eq!(preview.seed.state, FactorySeedState::Missing);
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
    }

    #[cfg(windows)]
    #[test]
    fn managed_openclaw_runtime_junction_is_skipped_and_unlinked() {
        let (temporary, paths) = create_fixture();
        let outside = temporary.path().join("outside-openclaw-cache");
        let sentinel = outside.join("sentinel.txt");
        fs::create_dir_all(&outside).unwrap();
        fs::write(&sentinel, b"outside").unwrap();
        let junction = paths
            .openclaw_home
            .join("plugin-skills")
            .join("browser-automation");
        create_junction(&junction, &outside);

        build_preview(&paths).unwrap();
        create_recovery_point(&paths, "managed-junction").unwrap();
        assert!(
            !recovery_dir(&paths, "managed-junction")
                .join("files/data/.openclaw/plugin-skills/browser-automation")
                .exists()
        );

        clean_user_data(&paths).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
        assert!(!junction.exists());
    }

    #[cfg(windows)]
    #[test]
    fn unknown_junction_still_blocks_factory_preview() {
        let (temporary, paths) = create_fixture();
        let outside = temporary.path().join("outside-unknown");
        fs::create_dir_all(&outside).unwrap();
        let junction = paths.workspace.join("unknown-link");
        create_junction(&junction, &outside);

        assert!(
            build_preview(&paths)
                .unwrap_err()
                .to_string()
                .contains("重解析点")
        );
    }

    #[test]
    fn clean_preserves_installation_and_only_license_private_key() {
        let (_temporary, paths) = create_fixture();
        let installation_id = paths.ensure_installation_id().unwrap();
        fs::write(paths.workspace.join("personal.txt"), b"personal").unwrap();
        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set(
                INSTALLATION_PRIVATE_KEY_REF,
                Zeroizing::new("license-private".to_string()),
            )
            .unwrap();
        vault
            .set("provider-codex", Zeroizing::new("secret".to_string()))
            .unwrap();
        vault.save(&paths).unwrap();

        clean_user_data(&paths).unwrap();

        assert_eq!(
            paths.read_installation_id().as_deref(),
            Some(installation_id.as_str())
        );
        assert!(!paths.workspace.join("personal.txt").exists());
        assert!(paths.read_factory_generation_id().is_some());
        let reopened = PortableVault::open(&paths).unwrap();
        assert!(reopened.contains(INSTALLATION_PRIVATE_KEY_REF));
        assert!(!reopened.contains("provider-codex"));
    }

    #[test]
    fn recovery_point_restores_generation_and_personal_data() {
        let (_temporary, paths) = create_fixture();
        fs::write(paths.workspace.join("personal.txt"), b"personal").unwrap();
        fs::write(
            paths.factory_generation_file(),
            b"11111111111111111111111111111111\n",
        )
        .unwrap();
        create_recovery_point(&paths, "operation").unwrap();
        clean_user_data(&paths).unwrap();
        rollback_recovery(&paths, "operation").unwrap();
        assert_eq!(
            fs::read(paths.workspace.join("personal.txt")).unwrap(),
            b"personal"
        );
        assert_eq!(
            paths.read_factory_generation_id().as_deref(),
            Some("11111111111111111111111111111111")
        );
    }

    #[test]
    fn isolated_reset_restores_seed_and_never_touches_outside_sentinel() {
        let (temporary, paths) = create_fixture();
        let sentinel = temporary.path().join("outside-sentinel.txt");
        fs::write(&sentinel, b"outside").unwrap();
        fs::create_dir_all(paths.data.join("license")).unwrap();
        fs::write(paths.data.join("license/entitlement.json"), b"entitlement").unwrap();
        fs::write(
            paths.data.join("license/installation.json"),
            b"installation",
        )
        .unwrap();
        fs::write(paths.workspace.join("personal.txt"), b"personal").unwrap();
        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set(
                INSTALLATION_PRIVATE_KEY_REF,
                Zeroizing::new("license-private".to_string()),
            )
            .unwrap();
        vault
            .set("provider-openclaw", Zeroizing::new("secret".to_string()))
            .unwrap();
        vault.save(&paths).unwrap();

        let seed = verify_seed(&paths).unwrap();
        clean_user_data(&paths).unwrap();
        restore_seed(&paths, &seed).unwrap();
        chat::initialize_storage(&paths).unwrap();
        imagegen::initialize_storage(&paths).unwrap();
        verify_post_reset(&paths).unwrap();

        assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
        assert_eq!(
            fs::read(paths.data.join("license/entitlement.json")).unwrap(),
            b"entitlement"
        );
        assert_eq!(
            fs::read(paths.data.join("license/installation.json")).unwrap(),
            b"installation"
        );
        assert!(!paths.workspace.join("personal.txt").exists());
        assert!(
            fs::metadata(paths.data.join("chat/chat.sqlite"))
                .unwrap()
                .len()
                > 0
        );
        assert!(
            fs::metadata(paths.data.join("integrations/image-studio/history.sqlite"))
                .unwrap()
                .len()
                > 0
        );
    }

    #[test]
    fn seed_verification_rejects_parent_path_escape() {
        let (_temporary, paths) = create_fixture();
        let manifest_path = factory_manifest_path(&paths);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["roots"][0]["source"] = serde_json::Value::String("../outside".to_string());
        write_json_atomic(&manifest_path, &manifest).unwrap();
        assert!(
            verify_seed(&paths)
                .unwrap_err()
                .to_string()
                .contains("相对路径无效")
        );
    }

    #[test]
    #[ignore = "requires YUNSHU_STAGE7_RESET_TEST_ROOT pointing to a disposable initialized candidate copy"]
    fn isolated_stage7_factory_reset_preserves_identity_and_restores_locale() {
        let root = std::env::var_os("YUNSHU_STAGE7_RESET_TEST_ROOT")
            .map(PathBuf::from)
            .expect("YUNSHU_STAGE7_RESET_TEST_ROOT is required");
        let paths = WorkstationPaths::from_root(root);
        crate::first_use::ensure_initialized(&paths).unwrap();
        let installation_id = paths.read_installation_id().unwrap();
        let factory_hash = sha256_file(&factory_manifest_path(&paths)).unwrap();
        let locale_manifest = paths.root.join("Factory/Seeds/Codex/manifest.json");
        let locale_hash = sha256_file(&locale_manifest).unwrap();

        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set(
                INSTALLATION_PRIVATE_KEY_REF,
                Zeroizing::new("stage7-license-private".to_string()),
            )
            .unwrap();
        vault
            .set(
                "provider-codex",
                Zeroizing::new("stage7-provider-secret".to_string()),
            )
            .unwrap();
        vault.save(&paths).unwrap();
        fs::create_dir_all(&paths.workspace).unwrap();
        fs::write(paths.workspace.join("STAGE7-PERSONAL.txt"), b"remove").unwrap();

        let seed = verify_seed(&paths).unwrap();
        clean_user_data(&paths).unwrap();
        restore_seed(&paths, &seed).unwrap();
        yunshu_workstation_runtime::locale_init::apply_seed(&paths).unwrap();
        chat::initialize_storage(&paths).unwrap();
        imagegen::initialize_storage(&paths).unwrap();
        verify_post_reset(&paths).unwrap();

        assert_eq!(
            paths.read_installation_id().as_deref(),
            Some(installation_id.as_str())
        );
        assert!(!paths.workspace.join("STAGE7-PERSONAL.txt").exists());
        let reopened = PortableVault::open(&paths).unwrap();
        assert!(reopened.contains(INSTALLATION_PRIVATE_KEY_REF));
        assert!(!reopened.contains("provider-codex"));
        assert_eq!(
            sha256_file(&factory_manifest_path(&paths)).unwrap(),
            factory_hash
        );
        assert_eq!(sha256_file(&locale_manifest).unwrap(), locale_hash);
        assert!(paths.data.join("initialization.json").is_file());
    }
}
