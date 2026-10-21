use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;
use yunshu_usb_writer_core::{
    InstallRequest, TargetDiskInfo, WriteJobStatus, WritePhase, cleanup_owned_staging,
    install_from_verified_installer, list_usb_targets, public_key_from_hex,
    verify_single_exe_installer_with_public_key,
};

#[derive(Default)]
struct InstallerState {
    jobs: Mutex<HashMap<String, InstallerJob>>,
}

#[derive(Clone)]
struct InstallerJob {
    status: Arc<Mutex<InstallerJobStatus>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfVerifyResult {
    product_version: String,
    build_id: String,
    file_count: u64,
    expanded_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallerJobStatus {
    job_id: String,
    phase: String,
    message: String,
    target_root: String,
    processed_files: u64,
    total_files: u64,
    processed_bytes: u64,
    total_bytes: u64,
    error: Option<String>,
}

impl InstallerJobStatus {
    fn update(&mut self, value: &WriteJobStatus) {
        self.phase = match value.phase {
            WritePhase::Preflight => "preflight",
            WritePhase::Staging => "staging",
            WritePhase::Writing => "writing",
            WritePhase::Verifying => "verifying",
            WritePhase::Committing => "committing",
            WritePhase::Completed => "completed",
            WritePhase::Cancelled => "cancelled",
            WritePhase::Failed => "failed",
        }
        .into();
        self.message.clone_from(&value.message);
        self.processed_files = value.processed_files;
        self.total_files = value.total_files;
        self.processed_bytes = value.processed_bytes;
        self.total_bytes = value.total_bytes;
        self.error.clone_from(&value.error);
    }
}

#[tauri::command]
async fn installer_self_verify() -> Result<SelfVerifyResult, String> {
    run_blocking(move || {
        let installer = verify_current_installer(&std::env::current_exe()?)?;
        Ok(SelfVerifyResult {
            product_version: installer.manifest.product_version,
            build_id: installer.manifest.build_id,
            file_count: installer.manifest.file_count,
            expanded_bytes: installer.manifest.expanded_bytes,
        })
    })
    .await
}

#[tauri::command]
async fn installer_usb_targets_list() -> Result<Vec<TargetDiskInfo>, String> {
    run_blocking(move || {
        let current_exe = std::env::current_exe()?;
        let installer = verify_current_installer(&current_exe)?;
        list_usb_targets(installer.manifest.expanded_bytes, Some(&current_exe))
    })
    .await
}

#[tauri::command]
async fn installer_install_start(
    state: State<'_, InstallerState>,
    target_root: String,
) -> Result<String, String> {
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let verified = run_blocking({
        let current_exe = current_exe.clone();
        move || verify_current_installer(&current_exe)
    })
    .await?;
    let targets = list_usb_targets(verified.manifest.expanded_bytes, Some(&current_exe))
        .map_err(|error| error.to_string())?;
    let target = targets
        .iter()
        .find(|value| value.root.eq_ignore_ascii_case(&target_root))
        .ok_or("目标盘已消失或不是USB磁盘")?;
    if !target.eligible {
        return Err(target
            .reason
            .clone()
            .unwrap_or_else(|| "目标盘不可用".into()));
    }

    let request = InstallRequest::new(PathBuf::from(&target_root));
    let job_id = request.operation_id.clone();
    let status = Arc::new(Mutex::new(InstallerJobStatus {
        job_id: job_id.clone(),
        phase: "queued".into(),
        message: "等待开始".into(),
        target_root: target_root.clone(),
        processed_files: 0,
        total_files: verified.manifest.file_count,
        processed_bytes: 0,
        total_bytes: verified.manifest.expanded_bytes,
        error: None,
    }));
    let cancelled = Arc::new(AtomicBool::new(false));
    state.jobs.lock().map_err(|_| "任务状态锁已损坏")?.insert(
        job_id.clone(),
        InstallerJob {
            status: status.clone(),
            cancelled: cancelled.clone(),
        },
    );
    std::thread::spawn(move || {
        let update = status.clone();
        let result = install_from_verified_installer(
            &verified,
            &request,
            move |write_status| {
                if let Ok(mut current) = update.lock() {
                    current.update(write_status);
                }
            },
            || cancelled.load(Ordering::Relaxed),
        );
        if let Err(error) = result {
            if let Ok(mut current) = status.lock() {
                current.phase = "failed".into();
                current.message = "安装失败，未发布半成品工作站目录".into();
                current.error = Some(error.to_string());
            }
        } else if !cancelled.load(Ordering::Relaxed) {
            open_target(&request.target_root);
        }
    });
    Ok(job_id)
}

#[tauri::command]
fn installer_install_status(
    state: State<'_, InstallerState>,
    job_id: String,
) -> Result<InstallerJobStatus, String> {
    let jobs = state.jobs.lock().map_err(|_| "任务状态锁已损坏")?;
    let job = jobs.get(&job_id).ok_or("未找到安装任务")?;
    let status = job.status.lock().map_err(|_| "任务状态锁已损坏")?.clone();
    Ok(status)
}

#[tauri::command]
fn installer_install_cancel(
    state: State<'_, InstallerState>,
    job_id: String,
) -> Result<(), String> {
    let jobs = state.jobs.lock().map_err(|_| "任务状态锁已损坏")?;
    let job = jobs.get(&job_id).ok_or("未找到安装任务")?;
    job.cancelled.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn installer_orphan_staging_cleanup(
    target_root: String,
    operation_id: String,
) -> Result<bool, String> {
    let staging = Path::new(&target_root).join(format!(".yunshu-write-{operation_id}.partial"));
    cleanup_owned_staging(&staging, &operation_id).map_err(|error| error.to_string())
}

fn open_target(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(not(windows))]
    let _ = path;
}

fn verify_current_installer(
    path: &Path,
) -> anyhow::Result<yunshu_usb_writer_core::VerifiedInstaller> {
    let public_key = option_env!("YUNSHU_INSTALLER_PUBLIC_KEY_HEX")
        .ok_or_else(|| anyhow::anyhow!("安装器模板未配置可信签名公钥"))?;
    let public_key = public_key_from_hex(public_key)?;
    verify_single_exe_installer_with_public_key(path, &public_key)
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("后台任务异常: {error}"))?
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(InstallerState::default())
        .invoke_handler(tauri::generate_handler![
            installer_self_verify,
            installer_usb_targets_list,
            installer_install_start,
            installer_install_status,
            installer_install_cancel,
            installer_orphan_staging_cleanup,
        ])
        .run(tauri::generate_context!())
        .expect("启动云枢AI一键写入U盘失败");
}
