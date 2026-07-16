pub mod commands;
pub mod install;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

const TRAY_ID: &str = "codex_plus_tray";
const MANAGER_GUARD_RETRY_ATTEMPTS: usize = 8;
const MANAGER_GUARD_RETRY_DELAY_MS: u64 = 150;
const MANAGER_EXIT_WATCHDOG_MS: u64 = 750;
const STARTUP_STABILITY_EXIT_ENV: &str = "CODEX_PLUS_MANAGER_EXIT_AFTER_READY_MS";

const TRAY_MENU_SHOW: &str = "tray_show_main";
const TRAY_MENU_QUIT: &str = "tray_quit_app";
static MANAGER_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn run() {
    install_panic_logger();
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.start",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "executable_path": std::env::current_exe().ok()
        }),
    );
    let Some(_guard) = acquire_single_instance_guard() else {
        return;
    };
    let show_update = commands::startup_should_show_update();
    let run_result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let url = if show_update {
                "/index.html?showUpdate=1"
            } else {
                "/index.html"
            };
            let mut main_window_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
                    .title("Codex 便携工作站")
                    .inner_size(1280.0, 860.0)
                    .min_inner_size(960.0, 720.0);
            if let Some(icon) = app.default_window_icon().cloned() {
                main_window_builder = main_window_builder.icon(icon)?;
            }
            let main_window = main_window_builder.build()?;
            install_tray(app)?;
            register_main_window_events(main_window);
            schedule_startup_stability_exit(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_version,
            commands::startup_options,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::save_settings,
            commands::load_ccs_providers,
            commands::import_ccs_providers,
            commands::load_pending_provider_import,
            commands::confirm_pending_provider_import,
            commands::dismiss_pending_provider_import,
            commands::list_local_sessions,
            commands::list_zed_remote_projects,
            commands::open_zed_remote,
            commands::forget_zed_remote_project,
            commands::delete_local_session,
            commands::load_provider_sync_targets,
            commands::preview_session_index_cleanup,
            commands::apply_session_index_cleanup,
            commands::sync_providers_now,
            commands::load_ads,
            commands::refresh_script_market,
            commands::install_market_script,
            commands::set_user_script_enabled,
            commands::delete_user_script,
            commands::open_external_url,
            commands::install_entrypoints,
            commands::uninstall_entrypoints,
            commands::repair_shortcuts,
            commands::plugin_marketplace_status,
            commands::repair_plugin_marketplace,
            commands::remote_plugin_marketplace_status,
            commands::repair_remote_plugin_marketplace,
            commands::check_update,
            commands::load_update_preferences,
            commands::set_ignored_update_version,
            commands::perform_update,
            commands::load_watcher_state,
            commands::install_watcher,
            commands::uninstall_watcher,
            commands::enable_watcher,
            commands::disable_watcher,
            commands::read_latest_logs,
            commands::copy_diagnostics,
            commands::reset_settings,
            commands::reset_image_overlay_settings,
            commands::relay_status,
            commands::read_relay_files,
            commands::check_env_conflicts,
            commands::check_relay_environment,
            commands::remove_env_conflicts,
            commands::save_relay_file,
            commands::write_diagnostic_event,
            commands::backfill_relay_profile_from_live,
            commands::list_context_entries,
            commands::read_live_context_entries,
            commands::sync_live_context_entries,
            commands::upsert_context_entry,
            commands::delete_context_entry,
            commands::extract_relay_common_config,
            commands::test_relay_profile,
            commands::diagnose_relay_profile,
            commands::test_stepwise_settings,
            commands::fetch_relay_profile_models,
            commands::switch_relay_profile,
            commands::apply_relay_injection,
            commands::apply_pure_api_injection,
            commands::clear_relay_injection,
            manager_exit_app,
            manager_hide_to_tray,
            update_tray_labels
        ])
        .run(tauri::generate_context!());
    match run_result {
        Ok(()) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.exit_completed",
                serde_json::json!({}),
            );
        }
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.run_failed",
                serde_json::json!({
                    "error": error.to_string()
                }),
            );
        }
    }
}

fn install_tray<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, TRAY_MENU_SHOW, "显示主窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出程序", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TRAY_MENU_SHOW => {
                show_main_window(app);
            }
            TRAY_MENU_QUIT => {
                request_manager_exit(app, "tray_menu");
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                show_main_window(&tray.app_handle());
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon);
    }

    let _ = tray_builder.build(app)?;
    Ok(())
}

fn register_main_window_events<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    let app_handle = window.app_handle().clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.close_requested",
                serde_json::json!({}),
            );
            request_manager_exit(&app_handle, "window_close");
        }
        _ => {}
    });
}

#[tauri::command]
fn manager_exit_app<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    request_manager_exit(&app, "renderer_command");
}

#[tauri::command]
fn manager_hide_to_tray<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    let _ = window.hide();
}

#[tauri::command]
fn update_tray_labels<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    show_label: String,
    quit_label: String,
    window_title: String,
) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let show_item = MenuItem::with_id(&app, TRAY_MENU_SHOW, &show_label, true, None::<&str>);
        let quit_item = MenuItem::with_id(&app, TRAY_MENU_QUIT, &quit_label, true, None::<&str>);
        if let (Ok(show), Ok(quit)) = (show_item, quit_item) {
            if let Ok(menu) = Menu::with_items(&app, &[&show, &quit]) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&window_title);
    }
}

fn show_main_window<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn request_manager_exit<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>, source: &str) {
    if MANAGER_EXIT_REQUESTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.exit_requested",
        serde_json::json!({
            "source": source
        }),
    );
    if let Some(tray) = app_handle.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(false);
    }
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.destroy();
    }
    arm_exit_watchdog(source.to_string());
    app_handle.exit(0);
}

fn arm_exit_watchdog(source: String) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(MANAGER_EXIT_WATCHDOG_MS));
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "manager.exit_forced",
            serde_json::json!({
                "source": source,
                "watchdog_ms": MANAGER_EXIT_WATCHDOG_MS
            }),
        );
        std::process::exit(0);
    });
}

fn schedule_startup_stability_exit<R: tauri::Runtime>(app_handle: tauri::AppHandle<R>) {
    let Some(delay_ms) = startup_stability_exit_delay_ms() else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        request_manager_exit(&app_handle, "startup_stability_probe");
    });
}

fn startup_stability_exit_delay_ms() -> Option<u64> {
    let value = std::env::var(STARTUP_STABILITY_EXIT_ENV).ok();
    parse_startup_stability_exit_delay_ms(value.as_deref())
}

fn parse_startup_stability_exit_delay_ms(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (100..=30_000).contains(value))
}

/// Restores and focuses an existing manager window on Windows.
///
/// This is a no-op on other platforms.
pub fn focus_existing_manager_window() -> bool {
    #[cfg(windows)]
    {
        let current_process_id = std::process::id();
        for process in codex_plus_core::windows_enumerate_processes() {
            if process.process_id == current_process_id {
                continue;
            }
            if is_manager_process_name(&process.exe_file)
                && codex_plus_core::windows_activate_process_window(process.process_id)
            {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.existing_window_activated",
                    serde_json::json!({
                        "process_id": process.process_id,
                        "executable_name": process.exe_file
                    }),
                );
                return true;
            }
        }
    }
    false
}

fn is_manager_process_name(executable_name: &str) -> bool {
    let executable_name = executable_name.to_ascii_lowercase();
    executable_name == "codex-plus-plus-manager.exe"
        || (executable_name.starts_with("codex-plus-plus-manager.")
            && executable_name.ends_with(".exe"))
}

fn install_panic_logger() {
    std::panic::set_hook(Box::new(|panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_string())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非字符串 panic payload".to_string());
        let location = panic_info.location().map(|location| {
            serde_json::json!({
                "file": location.file(),
                "line": location.line(),
                "column": location.column()
            })
        });
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "manager.panic",
            serde_json::json!({
                "payload": payload,
                "location": location
            }),
        );
    }));
}

fn acquire_single_instance_guard() -> Option<codex_plus_core::ports::LoopbackPortGuard> {
    let guard_port = codex_plus_core::ports::manager_guard_port();
    for attempt in 0..=MANAGER_GUARD_RETRY_ATTEMPTS {
        match codex_plus_core::ports::acquire_resilient_loopback_port_guard(guard_port) {
            Ok(guard) => {
                if let Some(fallback_lock_path) = guard.fallback_path() {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.guard_fallback",
                        serde_json::json!({
                            "requested_guard_port": guard_port,
                            "fallback_lock_path": fallback_lock_path
                        }),
                    );
                }
                return Some(guard);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::WouldBlock
                ) =>
            {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.already_running",
                    serde_json::json!({
                        "guard_port": guard_port,
                        "attempt": attempt + 1
                    }),
                );
                if focus_existing_manager_window() {
                    return None;
                }
                if attempt < MANAGER_GUARD_RETRY_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(
                        MANAGER_GUARD_RETRY_DELAY_MS,
                    ));
                    continue;
                }
                match codex_plus_core::ports::acquire_loopback_lock_guard(guard_port) {
                    Ok(guard) => {
                        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                            "manager.guard_port_unavailable",
                            serde_json::json!({
                                "guard_port": guard_port,
                                "fallback_lock_path": guard.fallback_path()
                            }),
                        );
                        return Some(guard);
                    }
                    Err(lock_error) => {
                        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                            "manager.existing_window_not_found",
                            serde_json::json!({
                                "guard_port": guard_port,
                                "error": error.to_string(),
                                "lock_error": lock_error.to_string()
                            }),
                        );
                        return None;
                    }
                }
            }
            Err(error) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.guard_failed",
                    serde_json::json!({
                        "guard_port": guard_port,
                        "error": error.to_string()
                    }),
                );
                match std::net::TcpListener::bind(("127.0.0.1", 0)) {
                    Ok(listener) => {
                        return Some(codex_plus_core::ports::LoopbackPortGuard::listener(
                            listener,
                        ));
                    }
                    Err(fallback_error) => {
                        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                            "manager.guard_fallback_failed",
                            serde_json::json!({
                                "error": fallback_error.to_string()
                            }),
                        );
                        return None;
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_process_name_accepts_current_and_legacy_backup_names() {
        assert!(is_manager_process_name("codex-plus-plus-manager.exe"));
        assert!(is_manager_process_name(
            "codex-plus-plus-manager.phase-b-ui.bak.exe"
        ));
        assert!(is_manager_process_name("CODEX-PLUS-PLUS-MANAGER.OLD.EXE"));
    }

    #[test]
    fn manager_process_name_rejects_disabled_backups_and_unrelated_processes() {
        assert!(!is_manager_process_name(
            "codex-plus-plus-manager.exe.disabled"
        ));
        assert!(!is_manager_process_name("codex-plus-plus.exe"));
        assert!(!is_manager_process_name("manager.exe"));
    }

    #[test]
    fn startup_stability_exit_delay_is_bounded() {
        assert_eq!(
            parse_startup_stability_exit_delay_ms(Some("900")),
            Some(900)
        );
        assert_eq!(parse_startup_stability_exit_delay_ms(Some("50")), None);
        assert_eq!(parse_startup_stability_exit_delay_ms(Some("30001")), None);
        assert_eq!(parse_startup_stability_exit_delay_ms(Some("invalid")), None);
        assert_eq!(parse_startup_stability_exit_delay_ms(None), None);
    }

    #[test]
    fn exit_watchdog_finishes_before_guard_retry_window_closes() {
        let retry_window_ms = MANAGER_GUARD_RETRY_ATTEMPTS as u64 * MANAGER_GUARD_RETRY_DELAY_MS;
        assert!(MANAGER_EXIT_WATCHDOG_MS < retry_window_ms);
    }
}
