use anyhow::Context;
use chrono::Utc;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;
use yunshu_usb_writer_core::{
    InstallRequest, MasterValidationResult, ReleaseManifest, TargetDiskInfo, WriteJobStatus,
    WritePhase, build_release_manifest, build_single_exe_installer, cleanup_owned_staging,
    install_from_master, list_usb_targets, load_signing_key, validate_installer_template,
    validate_master,
};

#[derive(Default)]
struct AdminState {
    jobs: Mutex<HashMap<String, ManagedJob>>,
}

#[derive(Clone)]
struct ManagedJob {
    status: Arc<Mutex<AdminJobStatus>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminJobStatus {
    job_id: String,
    kind: String,
    phase: String,
    message: String,
    target: Option<String>,
    processed_files: u64,
    total_files: u64,
    processed_bytes: u64,
    total_bytes: u64,
    output_path: Option<String>,
    error: Option<String>,
}

impl AdminJobStatus {
    fn queued(
        job_id: String,
        kind: &str,
        target: Option<String>,
        manifest: &ReleaseManifest,
    ) -> Self {
        Self {
            job_id,
            kind: kind.into(),
            phase: "queued".into(),
            message: "等待开始".into(),
            target,
            processed_files: 0,
            total_files: manifest.file_count,
            processed_bytes: 0,
            total_bytes: manifest.expanded_bytes,
            output_path: None,
            error: None,
        }
    }

    fn update_from_write(&mut self, status: &WriteJobStatus) {
        self.phase = match status.phase {
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
        self.message.clone_from(&status.message);
        self.processed_files = status.processed_files;
        self.total_files = status.total_files;
        self.processed_bytes = status.processed_bytes;
        self.total_bytes = status.total_bytes;
        self.error.clone_from(&status.error);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchStartResult {
    batch_id: String,
    job_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestBuildResult {
    manifest: ReleaseManifest,
    validation: MasterValidationResult,
}

#[tauri::command]
async fn admin_master_validate(path: String) -> Result<MasterValidationResult, String> {
    run_blocking(move || validate_master(Path::new(&path))).await
}

#[tauri::command]
async fn admin_release_manifest_build(
    master_path: String,
    product_version: String,
) -> Result<ManifestBuildResult, String> {
    run_blocking(move || {
        let validation = validate_master(Path::new(&master_path))?;
        if !validation.valid {
            anyhow::bail!("母盘检查未通过，不能生成发布清单");
        }
        let factory_count = validation
            .factory
            .as_ref()
            .map(|value| value.file_count)
            .unwrap_or(0);
        let build_id = validation.build_id.clone().unwrap_or_default();
        let manifest = build_release_manifest(
            Path::new(&master_path),
            &product_version,
            &build_id,
            factory_count,
        )?;
        Ok(ManifestBuildResult {
            manifest,
            validation,
        })
    })
    .await
}

#[tauri::command]
async fn admin_usb_targets_list(
    required_bytes: u64,
    master_path: Option<String>,
) -> Result<Vec<TargetDiskInfo>, String> {
    run_blocking(move || list_usb_targets(required_bytes, master_path.as_deref().map(Path::new)))
        .await
}

#[tauri::command]
async fn admin_batch_write_start(
    state: State<'_, AdminState>,
    master_path: String,
    product_version: String,
    target_roots: Vec<String>,
    max_parallel: usize,
) -> Result<BatchStartResult, String> {
    if target_roots.is_empty() {
        return Err("请至少选择一个目标U盘".into());
    }
    if !(1..=4).contains(&max_parallel) {
        return Err("并行数量必须在1到4之间".into());
    }
    let manifest = run_blocking({
        let master_path = master_path.clone();
        let product_version = product_version.clone();
        move || prepare_release(&master_path, &product_version)
    })
    .await?;
    let targets = list_usb_targets(manifest.expanded_bytes, Some(Path::new(&master_path)))
        .map_err(|error| error.to_string())?;
    validate_selected_targets(&targets, &target_roots)?;

    let batch_id = format!("batch-{}", Utc::now().format("%Y%m%d-%H%M%S"));
    let mut queue = VecDeque::new();
    let mut job_ids = Vec::new();
    {
        let mut jobs = state.jobs.lock().map_err(|_| "任务状态锁已损坏")?;
        for root in target_roots {
            let request = InstallRequest::new(PathBuf::from(&root));
            let job_id = request.operation_id.clone();
            let status = Arc::new(Mutex::new(AdminJobStatus::queued(
                job_id.clone(),
                "usbWrite",
                Some(root),
                &manifest,
            )));
            let cancelled = Arc::new(AtomicBool::new(false));
            jobs.insert(
                job_id.clone(),
                ManagedJob {
                    status: status.clone(),
                    cancelled: cancelled.clone(),
                },
            );
            queue.push_back((request, status, cancelled));
            job_ids.push(job_id);
        }
    }
    let master = PathBuf::from(master_path);
    std::thread::spawn(move || {
        while !queue.is_empty() {
            let mut handles = Vec::new();
            for _ in 0..max_parallel {
                let Some((request, status, cancelled)) = queue.pop_front() else {
                    break;
                };
                let current_manifest = manifest.clone();
                let current_master = master.clone();
                handles.push(std::thread::spawn(move || {
                    let update = status.clone();
                    let result = install_from_master(
                        &current_master,
                        &current_manifest,
                        &request,
                        move |write_status| {
                            if let Ok(mut current) = update.lock() {
                                current.update_from_write(write_status);
                            }
                        },
                        || cancelled.load(Ordering::Relaxed),
                    );
                    if let Err(error) = result
                        && let Ok(mut current) = status.lock()
                    {
                        current.phase = "failed".into();
                        current.message = "该U盘写入失败".into();
                        current.error = Some(error.to_string());
                    }
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
    });
    Ok(BatchStartResult { batch_id, job_ids })
}

#[tauri::command]
async fn admin_user_installer_build(
    state: State<'_, AdminState>,
    master_path: String,
    product_version: String,
    output_directory: String,
) -> Result<String, String> {
    validate_version_for_filename(&product_version)?;
    let manifest = run_blocking({
        let master_path = master_path.clone();
        let product_version = product_version.clone();
        move || prepare_release(&master_path, &product_version)
    })
    .await?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let status = Arc::new(Mutex::new(AdminJobStatus::queued(
        job_id.clone(),
        "installerBuild",
        None,
        &manifest,
    )));
    let cancelled = Arc::new(AtomicBool::new(false));
    state.jobs.lock().map_err(|_| "任务状态锁已损坏")?.insert(
        job_id.clone(),
        ManagedJob {
            status: status.clone(),
            cancelled: cancelled.clone(),
        },
    );

    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<PathBuf> {
            if let Ok(mut current) = status.lock() {
                current.phase = "packaging".into();
                current.message = "正在生成ZIP64载荷并签名，超大母盘可能需要较长时间".into();
            }
            if cancelled.load(Ordering::Relaxed) {
                anyhow::bail!("任务已取消");
            }
            let current_exe = std::env::current_exe()?;
            let tool_root = current_exe.parent().context("管理员工具路径无效")?;
            let stub = tool_root.join("templates/云枢AI一键写入U盘.exe");
            let key_path = tool_root.join("admin-data/production-signing.key");
            let output = Path::new(&output_directory).join(format!(
                "云枢AI便携工作站一键安装包-{}.exe",
                product_version.trim()
            ));
            let key = load_signing_key(&key_path)
                .context("管理员目录缺少生产签名私钥，请重新部署完整管理员生产工具")?;
            let template_validation =
                validate_installer_template(&stub, &key.verifying_key().to_bytes())?;
            let verified = build_single_exe_installer(
                Path::new(&master_path),
                &manifest,
                &stub,
                &output,
                &key,
            )?;
            if verified.manifest.schema_version != 2 {
                anyhow::bail!("新原生用户安装包必须使用ReleaseManifest schema v2");
            }
            let report = serde_json::json!({
                "schemaVersion": 2,
                "status": "completed",
                "generatedAt": Utc::now().to_rfc3339(),
                "productVersion": verified.manifest.product_version,
                "buildId": verified.manifest.build_id,
                "fileCount": verified.manifest.file_count,
                "expandedBytes": verified.manifest.expanded_bytes,
                "payloadArchitecture": verified.manifest.payload_architecture,
                "footerSchemaVersion": verified.footer.schema_version,
                "manifestSchemaVersion": verified.manifest.schema_version,
                "nativeTemplateValidation": template_validation,
                "installerFileName": output.file_name().unwrap_or_default().to_string_lossy(),
            });
            fs::write(
                output.with_extension("build-report.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            Ok(output)
        })();
        if let Ok(mut current) = status.lock() {
            match result {
                Ok(output) => {
                    current.phase = "completed".into();
                    current.message = "用户一键安装包生成完成".into();
                    current.processed_files = current.total_files;
                    current.processed_bytes = current.total_bytes;
                    current.output_path = Some(output.to_string_lossy().into_owned());
                }
                Err(error) => {
                    current.phase = if cancelled.load(Ordering::Relaxed) {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    current.message = "用户一键安装包生成失败".into();
                    current.error = Some(error.to_string());
                }
            }
        }
    });
    Ok(job_id)
}

#[tauri::command]
fn admin_job_status(
    state: State<'_, AdminState>,
    job_id: String,
) -> Result<AdminJobStatus, String> {
    let jobs = state.jobs.lock().map_err(|_| "任务状态锁已损坏")?;
    let job = jobs.get(&job_id).ok_or("未找到任务")?;
    let status = job.status.lock().map_err(|_| "任务状态锁已损坏")?.clone();
    Ok(status)
}

#[tauri::command]
fn admin_job_cancel(state: State<'_, AdminState>, job_id: String) -> Result<(), String> {
    let jobs = state.jobs.lock().map_err(|_| "任务状态锁已损坏")?;
    let job = jobs.get(&job_id).ok_or("未找到任务")?;
    job.cancelled.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn admin_orphan_staging_cleanup(target_root: String, operation_id: String) -> Result<bool, String> {
    let staging = Path::new(&target_root).join(format!(".yunshu-write-{operation_id}.partial"));
    cleanup_owned_staging(&staging, &operation_id).map_err(|error| error.to_string())
}

fn prepare_release(master_path: &str, product_version: &str) -> anyhow::Result<ReleaseManifest> {
    let validation = validate_master(Path::new(master_path))?;
    if !validation.valid {
        anyhow::bail!("母盘检查未通过，拒绝写入");
    }
    build_release_manifest(
        Path::new(master_path),
        product_version,
        validation.build_id.as_deref().unwrap_or_default(),
        validation
            .factory
            .as_ref()
            .map(|value| value.file_count)
            .unwrap_or(0),
    )
}

fn validate_selected_targets(targets: &[TargetDiskInfo], roots: &[String]) -> Result<(), String> {
    let mut disks = HashSet::new();
    for root in roots {
        let target = targets
            .iter()
            .find(|target| target.root.eq_ignore_ascii_case(root))
            .ok_or_else(|| format!("目标盘已消失或不是USB磁盘: {root}"))?;
        if !target.eligible {
            return Err(target
                .reason
                .clone()
                .unwrap_or_else(|| "目标盘不可用".into()));
        }
        if !disks.insert(target.disk_number) {
            return Err("同一物理磁盘的多个分区不能同时选择".into());
        }
    }
    Ok(())
}

fn validate_version_for_filename(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.chars().any(|character| {
            matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        })
    {
        return Err("版本号为空或包含Windows文件名不允许的字符".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(root: &str, disk_number: u32, eligible: bool) -> TargetDiskInfo {
        TargetDiskInfo {
            root: root.into(),
            label: "TEST".into(),
            filesystem: "NTFS".into(),
            total_bytes: 100,
            free_bytes: 80,
            disk_number,
            model: "USB".into(),
            serial: format!("SERIAL-{disk_number}"),
            bus_type: "USB".into(),
            read_only: false,
            system_disk: false,
            boot_partition: false,
            eligible,
            reason: (!eligible).then(|| "目标盘不可用".into()),
        }
    }

    #[test]
    fn rejects_two_partitions_on_the_same_physical_disk() {
        let targets = vec![target("U:\\", 7, true), target("V:\\", 7, true)];
        let roots = vec!["U:\\".into(), "V:\\".into()];
        assert!(validate_selected_targets(&targets, &roots).is_err());
    }

    #[test]
    fn rejects_ineligible_or_missing_targets() {
        let targets = vec![target("U:\\", 7, false)];
        assert!(validate_selected_targets(&targets, &["U:\\".into()]).is_err());
        assert!(validate_selected_targets(&targets, &["V:\\".into()]).is_err());
    }
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
        .manage(AdminState::default())
        .invoke_handler(tauri::generate_handler![
            admin_master_validate,
            admin_release_manifest_build,
            admin_usb_targets_list,
            admin_batch_write_start,
            admin_user_installer_build,
            admin_job_status,
            admin_job_cancel,
            admin_orphan_staging_cleanup,
        ])
        .run(tauri::generate_context!())
        .expect("启动云枢AI管理员U盘生产工具失败");
}
