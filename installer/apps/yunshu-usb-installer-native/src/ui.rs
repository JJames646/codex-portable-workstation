use crate::disk::{DiskIdentity, NativeTargetDiskInfo, list_installer_targets, revalidate_target};
use crate::format::format_fat32_to_exfat;
use crate::host::{CompatibilityLevel, HostInfo, detect_host};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::ffi::{OsStr, c_void};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, COLOR_WINDOW, CreateFontW, DEFAULT_GUI_FONT, FW_SEMIBOLD, GetStockObject,
    HBRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, InitCommonControls, PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_GROUPBOX, BS_PUSHBUTTON,
    CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CBS_HASSTRINGS, CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowTextW, HMENU,
    IDC_ARROW, IDYES, KillTimer, LoadCursorW, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONWARNING, MB_OK, MB_YESNO, MSG, MessageBoxW, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_NCCREATE, WM_NCDESTROY,
    WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE,
    WS_MINIMIZEBOX, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use yunshu_usb_writer_core::{
    InstallRequest, PayloadArchitecture, VerifiedInstaller, WriteJobStatus, WritePhase,
    install_from_verified_installer_checked, public_key_from_hex,
    verify_single_exe_installer_with_public_key,
};

const MAIN_CLASS: &str = "YunshuNativeUsbInstallerWindow";
const FORMAT_CLASS: &str = "YunshuNativeUsbFormatConfirmWindow";
const TIMER_ID: usize = 1;

const ID_TARGETS: i32 = 1001;
const ID_REFRESH: i32 = 1002;
const ID_INSTALL: i32 = 1003;
const ID_CANCEL: i32 = 1004;
const ID_OPEN_USB: i32 = 1005;
const ID_RUN_HELPER: i32 = 1006;

const ID_FORMAT_BACKUP: i32 = 2001;
const ID_FORMAT_TEXT: i32 = 2002;
const ID_FORMAT_CONFIRM: i32 = 2003;
const ID_FORMAT_CANCEL: i32 = 2004;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnableWindow(hwnd: HWND, enable: i32) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Verifying,
    Scanning,
    Installing,
    Idle,
}

enum UiEvent {
    Verified(Result<Box<VerifiedInstaller>, String>),
    Targets(Result<Vec<NativeTargetDiskInfo>, String>),
    Status(String),
    FormatCompleted,
    Progress(WriteJobStatus),
    InstallFinished(Result<(WriteJobStatus, CompletedTarget, bool), String>),
}

struct CompletedTarget {
    identity: DiskIdentity,
}

struct Controls {
    version: HWND,
    targets: HWND,
    refresh: HWND,
    target_info: HWND,
    progress: HWND,
    status: HWND,
    install: HWND,
    cancel: HWND,
    open_usb: HWND,
    run_helper: HWND,
}

impl Controls {
    fn empty() -> Self {
        Self {
            version: null_mut(),
            targets: null_mut(),
            refresh: null_mut(),
            target_info: null_mut(),
            progress: null_mut(),
            status: null_mut(),
            install: null_mut(),
            cancel: null_mut(),
            open_usb: null_mut(),
            run_helper: null_mut(),
        }
    }
}

struct AppState {
    controls: Controls,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    host: HostInfo,
    activity: Activity,
    verified: Option<VerifiedInstaller>,
    targets: Vec<NativeTargetDiskInfo>,
    cancel: Arc<AtomicBool>,
    completed_target: Option<CompletedTarget>,
    risk_trial: bool,
    format_in_progress: bool,
    format_completed: bool,
    close_after_cancel: bool,
}

pub fn run() -> Result<()> {
    // 让管理员生成链可以在优化后的原生模板中确认固定模板标记。
    std::hint::black_box(&crate::NATIVE_TEMPLATE_MARKER);
    let host = detect_host()?;
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        return Err(std::io::Error::last_os_error()).context("无法读取当前程序模块");
    }

    unsafe { InitCommonControls() };
    register_main_class(instance)?;

    let (tx, rx) = mpsc::channel();
    let mut state = Box::new(AppState {
        controls: Controls::empty(),
        tx,
        rx,
        host,
        activity: Activity::Verifying,
        verified: None,
        targets: Vec::new(),
        cancel: Arc::new(AtomicBool::new(false)),
        completed_target: None,
        risk_trial: false,
        format_in_progress: false,
        format_completed: false,
        close_after_cancel: false,
    });
    let class = wide(MAIN_CLASS);
    let title = wide("云枢AI便携工作站一键安装");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            480,
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut AppState).cast::<c_void>(),
        )
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error()).context("无法创建安装器窗口");
    }

    unsafe { ShowWindow(hwnd, SW_SHOW) };
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result == -1 {
            return Err(std::io::Error::last_os_error()).context("Windows消息循环异常");
        }
        if result == 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

pub fn show_fatal_error(message: &str) {
    message_box(
        null_mut(),
        message,
        "云枢AI一键写入U盘",
        MB_OK | MB_ICONERROR,
    );
}

unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            if create.is_null() {
                return 0;
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            1
        }
        WM_CREATE => {
            let Some(state) = (unsafe { app_state(hwnd) }) else {
                return -1;
            };
            if let Err(error) = create_main_controls(hwnd, state) {
                show_error(hwnd, &error.to_string());
                return -1;
            }
            unsafe { SetTimer(hwnd, TIMER_ID, 80, None) };
            start_verification(state);
            0
        }
        WM_TIMER if wparam == TIMER_ID => {
            if let Some(state) = unsafe { app_state(hwnd) } {
                drain_events(hwnd, state);
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = unsafe { app_state(hwnd) } {
                handle_command(hwnd, state, wparam);
            }
            0
        }
        WM_CLOSE => {
            let installing = unsafe { app_state(hwnd) }
                .is_some_and(|state| state.activity == Activity::Installing);
            if installing {
                let result = message_box(
                    hwnd,
                    "工作站正在写入U盘。\n\n现在关闭会先请求安全取消并清理暂存区。是否继续？",
                    "确认取消安装",
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                );
                if result == IDYES
                    && let Some(state) = unsafe { app_state(hwnd) }
                {
                    state.close_after_cancel = true;
                    state.cancel.store(true, Ordering::Relaxed);
                    set_text(
                        state.controls.status,
                        if state.format_in_progress {
                            "已请求关闭；Windows格式化完成后将停止，请勿拔出U盘……"
                        } else {
                            "正在安全取消，完成后将自动关闭，请勿拔出U盘……"
                        },
                    );
                }
            } else {
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                KillTimer(hwnd, TIMER_ID);
                PostQuitMessage(0);
            }
            0
        }
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_main_controls(hwnd: HWND, state: &mut AppState) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(null()) };
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let heading_font = create_ui_font(20, FW_SEMIBOLD as i32);
    create_control(
        hwnd,
        instance,
        "STATIC",
        "云枢AI便携工作站一键安装",
        WS_CHILD | WS_VISIBLE,
        28,
        22,
        630,
        34,
        0,
        heading_font,
    )?;
    state.controls.version = create_control(
        hwnd,
        instance,
        "STATIC",
        "正在校验安装包完整性，请稍候……",
        WS_CHILD | WS_VISIBLE,
        30,
        58,
        630,
        24,
        0,
        font,
    )?;
    create_control(
        hwnd,
        instance,
        "BUTTON",
        "安装位置",
        WS_CHILD | WS_VISIBLE | BS_GROUPBOX as u32,
        24,
        92,
        636,
        224,
        0,
        font,
    )?;
    create_control(
        hwnd,
        instance,
        "STATIC",
        "选择目标U盘",
        WS_CHILD | WS_VISIBLE,
        44,
        122,
        200,
        22,
        0,
        font,
    )?;
    state.controls.targets = create_control(
        hwnd,
        instance,
        "COMBOBOX",
        "",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | CBS_DROPDOWNLIST as u32
            | CBS_HASSTRINGS as u32,
        44,
        146,
        480,
        220,
        ID_TARGETS,
        font,
    )?;
    state.controls.refresh = create_control(
        hwnd,
        instance,
        "BUTTON",
        "刷新",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
        536,
        146,
        104,
        32,
        ID_REFRESH,
        font,
    )?;
    state.controls.target_info = create_control(
        hwnd,
        instance,
        "STATIC",
        "等待安装包校验完成后扫描USB移动磁盘。",
        WS_CHILD | WS_VISIBLE,
        44,
        190,
        596,
        52,
        0,
        font,
    )?;
    state.controls.progress = create_control_raw(
        hwnd,
        instance,
        PROGRESS_CLASSW,
        null(),
        WS_CHILD | WS_VISIBLE,
        44,
        250,
        596,
        20,
        0,
        font,
    )?;
    unsafe {
        SendMessageW(state.controls.progress, PBM_SETRANGE32, 0, 1000);
        SendMessageW(state.controls.progress, PBM_SETPOS, 0, 0);
    }
    state.controls.status = create_control(
        hwnd,
        instance,
        "STATIC",
        "正在验证签名、清单和ZIP64载荷……",
        WS_CHILD | WS_VISIBLE,
        44,
        278,
        596,
        32,
        0,
        font,
    )?;
    state.controls.install = create_control(
        hwnd,
        instance,
        "BUTTON",
        "开始安装",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        28,
        334,
        150,
        38,
        ID_INSTALL,
        font,
    )?;
    state.controls.cancel = create_control(
        hwnd,
        instance,
        "BUTTON",
        "取消",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
        190,
        334,
        92,
        38,
        ID_CANCEL,
        font,
    )?;
    state.controls.open_usb = create_control(
        hwnd,
        instance,
        "BUTTON",
        "打开U盘",
        WS_CHILD | WS_TABSTOP | BS_PUSHBUTTON as u32,
        294,
        334,
        124,
        38,
        ID_OPEN_USB,
        font,
    )?;
    state.controls.run_helper = create_control(
        hwnd,
        instance,
        "BUTTON",
        "运行环境助手",
        WS_CHILD | WS_TABSTOP | BS_PUSHBUTTON as u32,
        430,
        334,
        154,
        38,
        ID_RUN_HELPER,
        font,
    )?;
    create_control(
        hwnd,
        instance,
        "STATIC",
        "不会格式化NTFS/exFAT，也不会删除U盘中的其他文件。安装完成后，首次使用或换电脑请运行U盘根目录的环境助手。",
        WS_CHILD | WS_VISIBLE,
        28,
        394,
        632,
        36,
        0,
        font,
    )?;
    update_control_state(state);
    Ok(())
}

fn create_ui_font(height: i32, weight: i32) -> *mut c_void {
    let face = wide("Segoe UI");
    let font = unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    };
    if font.is_null() {
        unsafe { GetStockObject(DEFAULT_GUI_FONT) }
    } else {
        font
    }
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
    font: *mut c_void,
) -> Result<HWND> {
    let class = wide(class);
    let text = wide(text);
    create_control_raw(
        parent,
        instance,
        class.as_ptr(),
        text.as_ptr(),
        style,
        x,
        y,
        width,
        height,
        id,
        font,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_control_raw(
    parent: HWND,
    instance: HINSTANCE,
    class: *const u16,
    text: *const u16,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
    font: *mut c_void,
) -> Result<HWND> {
    let control = unsafe {
        CreateWindowExW(
            if style & WS_BORDER != 0 {
                WS_EX_CLIENTEDGE
            } else {
                0
            },
            class,
            text,
            style,
            x,
            y,
            width,
            height,
            parent,
            id_to_menu(id),
            instance,
            null(),
        )
    };
    if control.is_null() {
        return Err(std::io::Error::last_os_error()).context("无法创建Windows原生控件");
    }
    unsafe { SendMessageW(control, WM_SETFONT, font as usize, 1) };
    Ok(control)
}

fn start_verification(state: &mut AppState) {
    state.activity = Activity::Verifying;
    update_control_state(state);
    let tx = state.tx.clone();
    thread::spawn(move || {
        let result = verify_current_installer()
            .map(Box::new)
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::Verified(result));
    });
}

fn verify_current_installer() -> Result<VerifiedInstaller> {
    let public_key =
        option_env!("YUNSHU_INSTALLER_PUBLIC_KEY_HEX").context("安装器模板未配置可信签名公钥")?;
    let public_key = public_key_from_hex(public_key)?;
    let installer =
        verify_single_exe_installer_with_public_key(&std::env::current_exe()?, &public_key)?;
    if installer.manifest.schema_version != 2 {
        bail!(
            "正式用户安装包必须使用ReleaseManifest schema v2，当前为v{}",
            installer.manifest.schema_version
        );
    }
    if installer.manifest.payload_architecture != PayloadArchitecture::X64 {
        bail!("当前原生安装器只接受x64工作站载荷");
    }
    Ok(installer)
}

fn begin_target_scan(state: &mut AppState) {
    let Some(installer) = state.verified.as_ref() else {
        return;
    };
    state.activity = Activity::Scanning;
    state.completed_target = None;
    state.format_in_progress = false;
    state.format_completed = false;
    set_text(
        state.controls.status,
        "正在使用Windows原生磁盘接口扫描USB……",
    );
    update_control_state(state);
    let required_bytes = installer.manifest.expanded_bytes;
    let tx = state.tx.clone();
    thread::spawn(move || {
        let result = std::env::current_exe()
            .context("无法读取安装器路径")
            .and_then(|path| list_installer_targets(required_bytes, &path))
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::Targets(result));
    });
}

fn begin_install(hwnd: HWND, state: &mut AppState) -> Result<()> {
    if state.activity != Activity::Idle {
        return Ok(());
    }
    let target = selected_target(state).context("请先选择目标U盘")?.clone();
    if !target.install_eligible && !target.format_eligible {
        bail!(
            "{}",
            target.reason.as_deref().unwrap_or("当前目标U盘不可用")
        );
    }
    if target.format_eligible && !confirm_fat32_format(hwnd, &target)? {
        return Ok(());
    }

    let installer = state.verified.clone().context("安装包尚未完成验证")?;
    state.activity = Activity::Installing;
    state.completed_target = None;
    state.cancel = Arc::new(AtomicBool::new(false));
    state.format_in_progress = target.format_eligible;
    state.format_completed = false;
    state.close_after_cancel = false;
    set_text(
        state.controls.status,
        if target.format_eligible {
            "等待Windows管理员授权并格式化为exFAT……"
        } else {
            "正在重新确认目标USB物理身份……"
        },
    );
    unsafe { SendMessageW(state.controls.progress, PBM_SETPOS, 0, 0) };
    update_control_state(state);

    let tx = state.tx.clone();
    let cancel = state.cancel.clone();
    thread::spawn(move || {
        let result = run_install_worker(installer, target, tx.clone(), cancel)
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::InstallFinished(result));
    });
    Ok(())
}

fn run_install_worker(
    installer: VerifiedInstaller,
    selected: NativeTargetDiskInfo,
    tx: Sender<UiEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(WriteJobStatus, CompletedTarget, bool)> {
    let current_exe = std::env::current_exe()?;
    let required_bytes = installer.manifest.expanded_bytes;
    let formatted = selected.format_eligible;
    let target = if formatted {
        let _ = tx.send(UiEvent::Status(
            "正在格式化FAT32目标，期间请勿拔出U盘……".into(),
        ));
        let target = format_fat32_to_exfat(&selected.identity(), required_bytes, &current_exe)?;
        let _ = tx.send(UiEvent::FormatCompleted);
        target
    } else {
        let values = list_installer_targets(required_bytes, &current_exe)?;
        let current = revalidate_target(&selected.identity(), &values)?.clone();
        if !current.install_eligible {
            bail!(
                "{}",
                current.reason.as_deref().unwrap_or("目标U盘状态已改变")
            );
        }
        current
    };
    let target_root = PathBuf::from(&target.root);
    let request = InstallRequest::new(target_root.clone());
    let expected_identity = target.identity();
    let expected_root = target.root.clone();
    let guard_exe = current_exe.clone();
    let mut last_sent = Instant::now() - Duration::from_secs(1);
    let mut last_phase = None;
    let status = install_from_verified_installer_checked(
        &installer,
        &request,
        |status| {
            let phase_changed = last_phase != Some(status.phase);
            if phase_changed || last_sent.elapsed() >= Duration::from_millis(100) {
                let _ = tx.send(UiEvent::Progress(status.clone()));
                last_sent = Instant::now();
                last_phase = Some(status.phase);
            }
        },
        || cancel.load(Ordering::Relaxed),
        || {
            let values = list_installer_targets(required_bytes, &guard_exe)?;
            let current = revalidate_target(&expected_identity, &values)?;
            if !current.root.eq_ignore_ascii_case(&expected_root) || !current.install_eligible {
                bail!("目标USB盘符、物理身份或可写状态已发生变化，拒绝提交");
            }
            Ok(())
        },
    )?;
    Ok((
        status,
        CompletedTarget {
            identity: expected_identity,
        },
        formatted,
    ))
}

fn drain_events(hwnd: HWND, state: &mut AppState) {
    loop {
        let event = match state.rx.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                state.activity = Activity::Idle;
                set_text(state.controls.status, "后台任务通道异常关闭");
                update_control_state(state);
                break;
            }
        };
        handle_event(hwnd, state, event);
    }
}

fn handle_event(hwnd: HWND, state: &mut AppState, event: UiEvent) {
    match event {
        UiEvent::Verified(Ok(installer)) => {
            if let Err(error) = approve_host(hwnd, state, &installer) {
                state.activity = Activity::Idle;
                set_text(state.controls.status, &error.to_string());
                show_error(hwnd, &error.to_string());
                update_control_state(state);
                return;
            }
            let version = format!(
                "版本 {}  ·  Build {}  ·  {} 个文件  ·  {}",
                installer.manifest.product_version,
                installer.manifest.build_id,
                installer.manifest.file_count,
                format_bytes(installer.manifest.expanded_bytes)
            );
            set_text(state.controls.version, &version);
            state.verified = Some(*installer);
            begin_target_scan(state);
        }
        UiEvent::Verified(Err(error)) => {
            state.activity = Activity::Idle;
            set_text(state.controls.version, "安装包验证失败");
            set_text(state.controls.status, &error);
            show_error(hwnd, &error);
            update_control_state(state);
        }
        UiEvent::Targets(Ok(targets)) => {
            state.activity = Activity::Idle;
            state.targets = targets;
            populate_targets(state);
            update_control_state(state);
        }
        UiEvent::Targets(Err(error)) => {
            state.activity = Activity::Idle;
            state.targets.clear();
            populate_targets(state);
            set_text(state.controls.status, &format!("扫描USB失败：{error}"));
            update_control_state(state);
        }
        UiEvent::Status(message) => set_text(state.controls.status, &message),
        UiEvent::FormatCompleted => {
            state.format_in_progress = false;
            state.format_completed = true;
            set_text(
                state.controls.status,
                if state.cancel.load(Ordering::Relaxed) {
                    "exFAT格式化已完成，正在安全取消后续写入……"
                } else {
                    "exFAT格式化已完成，正在写入工作站……"
                },
            );
        }
        UiEvent::Progress(status) => update_progress(state, &status),
        UiEvent::InstallFinished(Ok((status, completed_target, formatted))) => {
            state.activity = Activity::Idle;
            state.format_in_progress = false;
            state.format_completed = formatted;
            update_progress(state, &status);
            match status.phase {
                WritePhase::Completed => {
                    state.completed_target = Some(completed_target);
                    let report_result = write_completion_report(state, &status);
                    let status_text = if report_result.is_ok() {
                        "安装完成，全部文件已写入并通过SHA-256校验。"
                    } else {
                        "安装完成并通过SHA-256校验；本机安装报告写入失败。"
                    };
                    set_text(state.controls.status, status_text);
                    if close_if_requested(hwnd, state) {
                        return;
                    }
                    message_box(
                        hwnd,
                        if report_result.is_ok() {
                            "云枢AI便携工作站已完整写入U盘。\n\n首次使用或换电脑时，请运行U盘根目录的环境助手。"
                        } else {
                            "云枢AI便携工作站已完整写入U盘，但本机安装报告写入失败。\n\n首次使用或换电脑时，请运行U盘根目录的环境助手。"
                        },
                        "安装完成",
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                WritePhase::Cancelled => {
                    set_text(
                        state.controls.status,
                        if formatted {
                            "exFAT格式化已完成；工作站写入已取消，暂存区已安全清理。"
                        } else {
                            "写入已取消，暂存区已安全清理。"
                        },
                    );
                    if close_if_requested(hwnd, state) {
                        return;
                    }
                    if formatted {
                        message_box(
                            hwnd,
                            "U盘已按确认完成exFAT格式化，后续工作站写入已取消。目标列表将重新扫描。",
                            "写入已取消",
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    begin_target_scan(state);
                    return;
                }
                _ => set_text(state.controls.status, &status.message),
            }
            update_control_state(state);
        }
        UiEvent::InstallFinished(Err(error)) => {
            state.activity = Activity::Idle;
            state.format_in_progress = false;
            let refresh_target = state.format_completed;
            state.format_completed = false;
            set_text(state.controls.status, &format!("安装失败：{error}"));
            if close_if_requested(hwnd, state) {
                return;
            }
            show_error(hwnd, &format!("安装失败：\n\n{error}"));
            if refresh_target {
                begin_target_scan(state);
            } else {
                update_control_state(state);
            }
        }
    }
}

fn approve_host(hwnd: HWND, state: &mut AppState, installer: &VerifiedInstaller) -> Result<()> {
    let compatibility = &installer.manifest.host_compatibility;
    match state.host.compatibility {
        CompatibilityLevel::Supported => {
            if !compatibility
                .supported
                .iter()
                .any(|value| value == &state.host.compatibility_id)
            {
                bail!("当前发布清单未声明支持此Windows系统");
            }
        }
        CompatibilityLevel::RiskTrial => {
            if !compatibility
                .risk_trial
                .iter()
                .any(|value| value == &state.host.compatibility_id)
            {
                bail!("当前发布清单未允许此ARM64系统风险试装");
            }
            if !show_arm_risk_confirmation(hwnd, &state.host.reason)? {
                bail!("用户未同意Windows 11 ARM64风险试装");
            }
            state.risk_trial = true;
        }
        CompatibilityLevel::Blocked => bail!("{}", state.host.reason),
    }
    Ok(())
}

fn populate_targets(state: &mut AppState) {
    unsafe { SendMessageW(state.controls.targets, CB_RESETCONTENT, 0, 0) };
    let mut selected = None;
    for (index, target) in state.targets.iter().enumerate() {
        let status = if target.install_eligible {
            "可安装"
        } else if target.format_eligible {
            "需格式化"
        } else {
            "不可用"
        };
        let name = format!(
            "{}  {}  {}  {}  ({status})",
            target.root,
            display_or(&target.label, "未命名"),
            display_or(&target.model, "USB移动磁盘"),
            target.filesystem
        );
        let name = wide(&name);
        unsafe {
            SendMessageW(
                state.controls.targets,
                CB_ADDSTRING,
                0,
                name.as_ptr() as LPARAM,
            );
        }
        if selected.is_none() && (target.install_eligible || target.format_eligible) {
            selected = Some(index);
        }
    }
    if let Some(index) = selected.or((!state.targets.is_empty()).then_some(0)) {
        unsafe { SendMessageW(state.controls.targets, CB_SETCURSEL, index, 0) };
    }
    if state.targets.is_empty() {
        set_text(
            state.controls.target_info,
            "未检测到可识别的USB移动磁盘。请插入U盘后点击“刷新”。",
        );
        set_text(state.controls.status, "等待插入USB移动磁盘。");
    } else {
        update_selected_target(state);
        set_text(
            state.controls.status,
            if state.risk_trial {
                "安装包已验证；当前为Windows 11 ARM64风险试装模式。"
            } else {
                "安装包已验证。请选择目标U盘并开始安装。"
            },
        );
    }
}

fn update_selected_target(state: &mut AppState) {
    let Some(target) = selected_target(state) else {
        set_text(state.controls.target_info, "请选择目标U盘。");
        update_control_state(state);
        return;
    };
    let serial = display_or(&target.serial, "未提供序列号");
    let target_note = target.reason.as_deref().or_else(|| {
        target
            .boot_disk
            .then_some("检测到旧启动标记；本次不格式化，只写入工作站文件")
    });
    let detail = format!(
        "{} · {} · 总容量 {} · 可用 {}\n物理磁盘 #{} · 序列 {}{}",
        display_or(&target.model, "USB移动磁盘"),
        target.filesystem,
        format_bytes(target.total_bytes),
        format_bytes(target.free_bytes),
        target.disk_number,
        serial,
        target_note
            .map(|reason| format!(" · {reason}"))
            .unwrap_or_default()
    );
    set_text(state.controls.target_info, &detail);
    update_control_state(state);
}

fn selected_target(state: &AppState) -> Option<&NativeTargetDiskInfo> {
    let index = unsafe { SendMessageW(state.controls.targets, CB_GETCURSEL, 0, 0) };
    (index >= 0)
        .then_some(index as usize)
        .and_then(|index| state.targets.get(index))
}

fn update_progress(state: &AppState, status: &WriteJobStatus) {
    let position = match status.phase {
        WritePhase::Preflight => 20,
        WritePhase::Staging => 50,
        WritePhase::Writing => {
            let total = status.total_bytes.max(1);
            50 + ((status.processed_bytes.saturating_mul(850) / total).min(850) as usize)
        }
        WritePhase::Verifying => 930,
        WritePhase::Committing => 980,
        WritePhase::Completed => 1000,
        WritePhase::Cancelled | WritePhase::Failed => 0,
    };
    unsafe { SendMessageW(state.controls.progress, PBM_SETPOS, position, 0) };
    let message = if status.phase == WritePhase::Writing {
        format!(
            "{}（{} / {}，{} / {}）",
            status.message,
            status.processed_files,
            status.total_files,
            format_bytes(status.processed_bytes),
            format_bytes(status.total_bytes)
        )
    } else {
        status.message.clone()
    };
    set_text(state.controls.status, &message);
}

fn update_control_state(state: &AppState) {
    let idle = state.activity == Activity::Idle;
    let target_available = selected_target(state)
        .is_some_and(|target| target.install_eligible || target.format_eligible);
    set_enabled(state.controls.targets, idle && state.verified.is_some());
    set_enabled(state.controls.refresh, idle && state.verified.is_some());
    set_enabled(
        state.controls.install,
        idle && state.verified.is_some() && target_available,
    );
    set_enabled(
        state.controls.cancel,
        state.activity == Activity::Installing,
    );
    let completed = state.completed_target.is_some();
    unsafe {
        ShowWindow(
            state.controls.open_usb,
            if completed { SW_SHOW } else { SW_HIDE },
        );
        ShowWindow(
            state.controls.run_helper,
            if completed { SW_SHOW } else { SW_HIDE },
        );
    }
}

fn handle_command(hwnd: HWND, state: &mut AppState, wparam: WPARAM) {
    let id = (wparam & 0xffff) as i32;
    let notification = ((wparam >> 16) & 0xffff) as u32;
    match (id, notification) {
        (ID_TARGETS, CBN_SELCHANGE) => update_selected_target(state),
        (ID_REFRESH, BN_CLICKED) if state.activity == Activity::Idle => begin_target_scan(state),
        (ID_INSTALL, BN_CLICKED) => {
            if let Err(error) = begin_install(hwnd, state) {
                show_error(hwnd, &error.to_string());
            }
        }
        (ID_CANCEL, BN_CLICKED) if state.activity == Activity::Installing => {
            state.cancel.store(true, Ordering::Relaxed);
            set_text(
                state.controls.status,
                if state.format_in_progress {
                    "已收到取消请求；Windows格式化完成后将停止，请勿拔出U盘……"
                } else {
                    "正在安全取消，请勿拔出U盘……"
                },
            );
        }
        (ID_OPEN_USB, BN_CLICKED) => match revalidate_completed_target(state) {
            Ok(root) => {
                if let Err(error) = open_in_explorer(&root) {
                    show_error(hwnd, &error.to_string());
                }
            }
            Err(error) => show_error(hwnd, &error.to_string()),
        },
        (ID_RUN_HELPER, BN_CLICKED) => match revalidate_completed_target(state) {
            Ok(root) => {
                if let Err(error) = run_environment_helper(state, &root) {
                    show_error(hwnd, &error.to_string());
                }
            }
            Err(error) => show_error(hwnd, &error.to_string()),
        },
        _ => {}
    }
}

fn confirm_fat32_format(hwnd: HWND, target: &NativeTargetDiskInfo) -> Result<bool> {
    let boot_warning = if target.boot_disk {
        "\n\n检测到这个U盘带启动标记；格式化后原有启动功能也会消失。"
    } else {
        ""
    };
    let first = format!(
        "此U盘为FAT32，必须格式化为exFAT后才能安装。\n\n盘符：{}\n型号：{}\n序列号：{}\n容量：{}{}\n\n格式化会永久删除这个U盘中的全部文件。请先确认已经完成备份。是否继续？",
        target.root,
        display_or(&target.model, "USB移动磁盘"),
        display_or(&target.serial, "未提供"),
        format_bytes(target.total_bytes),
        boot_warning
    );
    if message_box(
        hwnd,
        &first,
        "FAT32格式化确认（第1步/2）",
        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
    ) != IDYES
    {
        return Ok(false);
    }
    show_format_confirmation(hwnd, target)
}

#[derive(Clone, Copy)]
enum ConfirmationKind {
    Fat32Format,
    Arm64RiskTrial,
}

struct FormatDialogState {
    kind: ConfirmationKind,
    target_text: String,
    backup: HWND,
    input: HWND,
    result: Option<bool>,
}

fn show_format_confirmation(parent: HWND, target: &NativeTargetDiskInfo) -> Result<bool> {
    show_checked_confirmation(
        parent,
        "FAT32格式化确认（第2步/2）",
        FormatDialogState {
            kind: ConfirmationKind::Fat32Format,
            target_text: format!(
                "即将清空并格式化：{}  {}  {}",
                target.root,
                display_or(&target.model, "USB移动磁盘"),
                display_or(&target.serial, "未提供序列号")
            ),
            backup: null_mut(),
            input: null_mut(),
            result: None,
        },
    )
}

fn show_arm_risk_confirmation(parent: HWND, reason: &str) -> Result<bool> {
    show_checked_confirmation(
        parent,
        "ARM64兼容风险确认",
        FormatDialogState {
            kind: ConfirmationKind::Arm64RiskTrial,
            target_text: format!(
                "检测到 Windows 11 ARM64。\n{reason}\n当前载荷为x64，三智能体兼容性尚未完整验证。"
            ),
            backup: null_mut(),
            input: null_mut(),
            result: None,
        },
    )
}

fn show_checked_confirmation(
    parent: HWND,
    window_title: &str,
    mut state: FormatDialogState,
) -> Result<bool> {
    let instance = unsafe { GetModuleHandleW(null()) };
    register_format_class(instance)?;
    let class = wide(FORMAT_CLASS);
    let title = wide(window_title);
    unsafe { EnableWindow(parent, 0) };
    let dialog = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            590,
            330,
            parent,
            null_mut(),
            instance,
            (&mut state as *mut FormatDialogState).cast::<c_void>(),
        )
    };
    if dialog.is_null() {
        unsafe { EnableWindow(parent, 1) };
        return Err(std::io::Error::last_os_error()).context("无法打开安全确认窗口");
    }
    unsafe { ShowWindow(dialog, SW_SHOW) };
    let mut message = MSG::default();
    while state.result.is_none() {
        let value = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if value <= 0 {
            state.result = Some(false);
            if value == 0 {
                unsafe { PostQuitMessage(message.wParam as i32) };
            }
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    unsafe {
        EnableWindow(parent, 1);
        ShowWindow(parent, SW_SHOW);
    }
    Ok(state.result.unwrap_or(false))
}

unsafe extern "system" fn format_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            if create.is_null() {
                return 0;
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            1
        }
        WM_CREATE => {
            let Some(state) = (unsafe { format_state(hwnd) }) else {
                return -1;
            };
            if let Err(error) = create_format_controls(hwnd, state) {
                show_error(hwnd, &error.to_string());
                state.result = Some(false);
                return -1;
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let notification = ((wparam >> 16) & 0xffff) as u32;
            if notification == BN_CLICKED
                && let Some(state) = unsafe { format_state(hwnd) }
            {
                match id {
                    ID_FORMAT_CONFIRM => {
                        let checked = unsafe {
                            SendMessageW(state.backup, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED
                        };
                        let typed = matches!(state.kind, ConfirmationKind::Fat32Format)
                            .then(|| window_text(state.input));
                        let valid = confirmation_is_valid(state.kind, checked, typed.as_deref());
                        if !valid {
                            let message = match state.kind {
                                ConfirmationKind::Fat32Format => {
                                    "请先勾选“我已备份U盘中的全部文件”，并在输入框中完整输入“格式化”。"
                                }
                                ConfirmationKind::Arm64RiskTrial => {
                                    "请先勾选“我已了解当前为ARM64风险试装”。"
                                }
                            };
                            message_box(hwnd, message, "确认信息不完整", MB_OK | MB_ICONWARNING);
                        } else {
                            state.result = Some(true);
                            unsafe { DestroyWindow(hwnd) };
                        }
                    }
                    ID_FORMAT_CANCEL => {
                        state.result = Some(false);
                        unsafe { DestroyWindow(hwnd) };
                    }
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            if let Some(state) = unsafe { format_state(hwnd) } {
                state.result = Some(false);
            }
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_format_controls(hwnd: HWND, state: &mut FormatDialogState) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(null()) };
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let (heading, checkbox_text, confirm_text) = match state.kind {
        ConfirmationKind::Fat32Format => (
            "最后确认：此操作不可撤销",
            "我已备份U盘中的全部文件",
            "确认格式化并安装",
        ),
        ConfirmationKind::Arm64RiskTrial => (
            "Windows 11 ARM64 风险试装",
            "我已了解当前为ARM64风险试装",
            "同意风险并继续",
        ),
    };
    create_control(
        hwnd,
        instance,
        "STATIC",
        heading,
        WS_CHILD | WS_VISIBLE,
        26,
        22,
        520,
        26,
        0,
        font,
    )?;
    create_control(
        hwnd,
        instance,
        "STATIC",
        &state.target_text,
        WS_CHILD | WS_VISIBLE,
        26,
        60,
        520,
        76,
        0,
        font,
    )?;
    state.backup = create_control(
        hwnd,
        instance,
        "BUTTON",
        checkbox_text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        26,
        142,
        520,
        28,
        ID_FORMAT_BACKUP,
        font,
    )?;
    if matches!(state.kind, ConfirmationKind::Fat32Format) {
        create_control(
            hwnd,
            instance,
            "STATIC",
            "请在下方输入“格式化”以确认：",
            WS_CHILD | WS_VISIBLE,
            26,
            174,
            300,
            24,
            0,
            font,
        )?;
        state.input = create_control(
            hwnd,
            instance,
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER,
            26,
            200,
            520,
            32,
            ID_FORMAT_TEXT,
            font,
        )?;
    }
    create_control(
        hwnd,
        instance,
        "BUTTON",
        confirm_text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        264,
        244,
        174,
        40,
        ID_FORMAT_CONFIRM,
        font,
    )?;
    create_control(
        hwnd,
        instance,
        "BUTTON",
        "取消",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
        450,
        244,
        96,
        40,
        ID_FORMAT_CANCEL,
        font,
    )?;
    Ok(())
}

fn confirmation_is_valid(kind: ConfirmationKind, checked: bool, typed: Option<&str>) -> bool {
    checked
        && match kind {
            ConfirmationKind::Fat32Format => typed.is_some_and(|value| value.trim() == "格式化"),
            ConfirmationKind::Arm64RiskTrial => true,
        }
}

fn close_if_requested(hwnd: HWND, state: &mut AppState) -> bool {
    if !state.close_after_cancel {
        return false;
    }
    state.close_after_cancel = false;
    unsafe { DestroyWindow(hwnd) };
    true
}

fn revalidate_completed_target(state: &AppState) -> Result<PathBuf> {
    let completed = state.completed_target.as_ref().context("尚未完成U盘安装")?;
    let installer = state.verified.as_ref().context("安装包信息不可用")?;
    let current_exe = std::env::current_exe().context("无法识别当前用户安装器")?;
    let targets = list_installer_targets(installer.manifest.expanded_bytes, &current_exe)?;
    let current = revalidate_target(&completed.identity, &targets)
        .context("完成安装的原USB已拔出或物理身份发生变化")?;
    Ok(PathBuf::from(&current.root))
}

fn run_environment_helper(state: &AppState, root: &Path) -> Result<()> {
    let manifest = &state
        .verified
        .as_ref()
        .context("安装包信息不可用")?
        .manifest;
    let mapped = manifest
        .root_mappings
        .iter()
        .map(|item| item.destination.as_str())
        .find(|value| {
            let lower = value.to_ascii_lowercase();
            lower.ends_with(".exe") && value.contains("环境") && value.contains("助手")
        })
        .map(|value| root.join(value.replace('/', "\\")));
    let helper = mapped.or_else(|| {
        [
            "云枢AI工作站环境助手.exe",
            "云枢AI工作站环境检测助手.exe",
            "云枢AI环境检测助手.exe",
        ]
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
    });
    let helper = helper.context("U盘根目录未找到环境助手")?;
    if !helper.is_file() {
        bail!("环境助手文件不存在: {}", helper.display());
    }
    std::process::Command::new(&helper)
        .spawn()
        .with_context(|| format!("无法启动环境助手: {}", helper.display()))?;
    Ok(())
}

fn open_in_explorer(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .with_context(|| format!("无法打开U盘: {}", path.display()))?;
    Ok(())
}

fn write_completion_report(state: &AppState, status: &WriteJobStatus) -> Result<PathBuf> {
    let installer = state.verified.as_ref().context("安装包信息不可用")?;
    let directory = std::env::temp_dir().join("YunshuAI-UsbInstaller/reports");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("install-{}.json", status.operation.operation_id));
    let report = serde_json::json!({
        "schemaVersion": 1,
        "createdAt": Utc::now().to_rfc3339(),
        "result": "completed",
        "operationId": status.operation.operation_id,
        "productVersion": installer.manifest.product_version,
        "buildId": installer.manifest.build_id,
        "payloadArchitecture": installer.manifest.payload_architecture,
        "hostCompatibility": state.host.compatibility_id,
        "riskTrial": state.risk_trial,
        "fileCount": status.total_files,
        "expandedBytes": status.total_bytes
    });
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok(path)
}

fn register_main_class(instance: HINSTANCE) -> Result<()> {
    register_window_class(instance, MAIN_CLASS, Some(main_window_proc))
}

fn register_format_class(instance: HINSTANCE) -> Result<()> {
    register_window_class(instance, FORMAT_CLASS, Some(format_window_proc))
}

fn register_window_class(
    instance: HINSTANCE,
    name: &str,
    procedure: windows_sys::Win32::UI::WindowsAndMessaging::WNDPROC,
) -> Result<()> {
    let class = wide(name);
    let definition = WNDCLASSW {
        style: 0,
        lpfnWndProc: procedure,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    if unsafe { RegisterClassW(&definition) } == 0 {
        let code = unsafe { GetLastError() };
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::from_raw_os_error(code as i32))
                .context("无法注册Windows原生窗口类");
        }
    }
    Ok(())
}

unsafe fn app_state<'a>(hwnd: HWND) -> Option<&'a mut AppState> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppState;
    unsafe { pointer.as_mut() }
}

unsafe fn format_state<'a>(hwnd: HWND) -> Option<&'a mut FormatDialogState> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut FormatDialogState;
    unsafe { pointer.as_mut() }
}

fn set_enabled(hwnd: HWND, enabled: bool) {
    if !hwnd.is_null() {
        unsafe { EnableWindow(hwnd, i32::from(enabled)) };
    }
}

fn set_text(hwnd: HWND, value: &str) {
    if hwnd.is_null() {
        return;
    }
    let value = wide(value);
    unsafe { SetWindowTextW(hwnd, value.as_ptr()) };
}

fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 128];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn message_box(hwnd: HWND, message: &str, title: &str, style: u32) -> i32 {
    let message = wide(message);
    let title = wide(title);
    unsafe { MessageBoxW(hwnd, message.as_ptr(), title.as_ptr(), style) }
}

fn show_error(hwnd: HWND, message: &str) {
    message_box(hwnd, message, "云枢AI一键写入U盘", MB_OK | MB_ICONERROR);
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn id_to_menu(id: i32) -> HMENU {
    if id == 0 {
        null_mut()
    } else {
        id as usize as HMENU
    }
}

fn display_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn format_bytes(value: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", value as f64 / GIB)
    } else {
        format!("{:.1} MB", value as f64 / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat32_confirmation_requires_checkbox_and_exact_phrase() {
        assert!(!confirmation_is_valid(
            ConfirmationKind::Fat32Format,
            false,
            Some("格式化")
        ));
        assert!(!confirmation_is_valid(
            ConfirmationKind::Fat32Format,
            true,
            Some("继续")
        ));
        assert!(confirmation_is_valid(
            ConfirmationKind::Fat32Format,
            true,
            Some(" 格式化 ")
        ));
    }

    #[test]
    fn arm64_risk_trial_requires_explicit_checkbox() {
        assert!(!confirmation_is_valid(
            ConfirmationKind::Arm64RiskTrial,
            false,
            None
        ));
        assert!(confirmation_is_valid(
            ConfirmationKind::Arm64RiskTrial,
            true,
            None
        ));
    }
}
