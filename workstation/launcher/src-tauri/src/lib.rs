mod chat;
mod chat_provider;
mod commands;
mod error;
mod factory_reset;
mod first_use;
mod image_provider;
mod imagegen;
mod imagegen_repository;
mod license;
mod tutorials;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use commands::AppState;
use serde::Serialize;
use tauri::RunEvent;
use yunshu_workstation_runtime::model::Engine;
use yunshu_workstation_runtime::paths::WorkstationPaths;

fn resolve_paths() -> anyhow::Result<WorkstationPaths> {
    let explicit_root = std::env::var_os("YUNSHU_WORKSTATION_ROOT").map(PathBuf::from);
    WorkstationPaths::resolve(explicit_root)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherSmokeReport {
    schema_version: u32,
    ok: bool,
    embedded_frontend: bool,
    launcher_version: &'static str,
    runtime_build_id: &'static str,
    generated_at: String,
    root: String,
    diagnosis: yunshu_workstation_runtime::runtime::DiagnoseResult,
}

pub const fn embedded_frontend_enabled() -> bool {
    cfg!(feature = "custom-protocol")
}

pub const fn runtime_build_id() -> &'static str {
    yunshu_workstation_runtime::RUNTIME_BUILD_ID
}

pub fn run_smoke() -> anyhow::Result<PathBuf> {
    let paths = resolve_paths()?;
    let diagnosis = yunshu_workstation_runtime::runtime::diagnose(&paths)?;
    let embedded_frontend = embedded_frontend_enabled();
    let report = LauncherSmokeReport {
        schema_version: 1,
        ok: embedded_frontend && diagnosis.detections.iter().all(|item| item.ready),
        embedded_frontend,
        launcher_version: env!("CARGO_PKG_VERSION"),
        runtime_build_id: yunshu_workstation_runtime::RUNTIME_BUILD_ID,
        generated_at: chrono::Utc::now().to_rfc3339(),
        root: paths.root.display().to_string(),
        diagnosis,
    };
    std::fs::create_dir_all(&paths.manifests)?;
    let destination = paths.manifests.join("phase5-launcher-smoke.json");
    let temporary = destination.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(&report)?;
    bytes.push(b'\n');
    std::fs::write(&temporary, bytes)?;
    yunshu_workstation_runtime::windows_process::replace_file_atomic(&temporary, &destination)?;
    if !report.ok {
        if !report.embedded_frontend {
            anyhow::bail!("启动器未启用 custom-protocol，不能作为离线便携版部署");
        }
        anyhow::bail!("三引擎主体检测未全部通过");
    }
    Ok(destination)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = resolve_paths().unwrap_or_else(|error| {
        panic!("无法解析云枢工作站根目录：{error}");
    });
    let exit_paths = paths.clone();
    let factory_reset_runtime = factory_reset::FactoryResetRuntime::default();
    let exit_factory_reset_runtime = factory_reset_runtime.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { paths })
        .manage(imagegen::ImageGenerationRuntime::default())
        .manage(factory_reset_runtime)
        .setup(|app| {
            if let Some(milliseconds) = std::env::var("YUNSHU_WORKSTATION_EXIT_AFTER_READY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(milliseconds));
                    handle.exit(0);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::workstation_bootstrap,
            commands::workstation_paths,
            commands::workstation_version,
            commands::workstation_diagnose,
            commands::engine_detect_all,
            commands::engine_status_all,
            commands::engine_probe,
            commands::engine_start,
            commands::openclaw_open_control,
            commands::engine_stop,
            commands::engine_stop_all,
            commands::codex_locale_status,
            commands::codex_locale_apply_seed,
            commands::codex_proxy_status,
            commands::codex_proxy_subscription_scan,
            commands::codex_proxy_node_select,
            commands::codex_proxy_start_once,
            commands::codex_proxy_enable_persistent,
            commands::codex_proxy_disable,
            commands::codex_proxy_clear,
            factory_reset::factory_seed_verify,
            factory_reset::factory_reset_preview,
            factory_reset::factory_reset_execute,
            factory_reset::factory_reset_status,
            factory_reset::factory_reset_rollback,
            commands::provider_get,
            commands::provider_get_all,
            commands::provider_save,
            commands::provider_discover_models,
            commands::provider_test,
            commands::provider_apply,
            commands::bridge_status,
            commands::skills_categories,
            commands::skills_recommended,
            commands::skills_search,
            commands::skills_detail,
            commands::skills_inspect_local,
            commands::skills_install,
            commands::skills_list_installed,
            commands::skills_uninstall,
            commands::skills_repair,
            commands::plugins_status,
            commands::plugins_list,
            commands::plugins_detail,
            commands::plugins_install,
            commands::plugins_install_batch,
            commands::plugins_installed,
            commands::plugins_repair,
            commands::tutorial_list,
            commands::tutorial_read,
            commands::tutorial_asset_read,
            commands::tutorial_search,
            commands::ui_preferences_load,
            commands::ui_preferences_save,
            commands::logs_open,
            commands::backups_open,
            commands::open_external_url,
            commands::announcement_list,
            license::license_status,
            license::license_activate,
            license::license_check_in,
            license::license_latest_announcement,
            license::license_mark_announcement_read,
            chat::chat_initialize,
            chat::chat_list_threads,
            chat::chat_create_thread,
            chat::chat_load_thread,
            chat::chat_send,
            chat::chat_delete_thread,
            chat::chat_rename_thread,
            chat::chat_export_thread,
            chat::chat_import_attachments,
            chat::chat_remove_attachment,
            chat::chat_set_thread_provider,
            chat_provider::chat_provider_list,
            chat_provider::chat_provider_save,
            chat_provider::chat_provider_set_active,
            chat_provider::chat_provider_discover_models,
            chat_provider::chat_provider_test,
            image_provider::image_provider_list,
            image_provider::image_provider_save,
            image_provider::image_provider_set_active,
            image_provider::image_provider_discover_models,
            image_provider::image_provider_test,
            imagegen::imagegen_initialize,
            imagegen::imagegen_generate,
            imagegen::imagegen_cancel,
            imagegen::imagegen_list_history,
            imagegen::imagegen_read_asset,
            imagegen::imagegen_read_reference,
            imagegen::imagegen_delete_job,
            imagegen::imagegen_clear_history,
            imagegen::imagegen_export_asset,
            imagegen::imagegen_open_output_directory,
        ])
        .build(tauri::generate_context!())
        .expect("云枢AI便携工作站运行失败");
    let exit_cleanup_started = Arc::new(AtomicBool::new(false));
    app.run(move |handle, event| {
        let RunEvent::ExitRequested { api, .. } = event else {
            return;
        };
        if exit_factory_reset_runtime.is_active() {
            api.prevent_exit();
            return;
        }
        if exit_cleanup_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let proxy_active = yunshu_workstation_runtime::codex_proxy::status(&exit_paths)
            .is_ok_and(|status| status.active);
        if !proxy_active {
            return;
        }
        api.prevent_exit();
        let handle = handle.clone();
        let paths = exit_paths.clone();
        std::thread::spawn(move || {
            let _ = yunshu_workstation_runtime::runtime::stop(
                &paths,
                Engine::Codex,
                Duration::from_secs(75),
            );
            handle.exit(0);
        });
    });
}
