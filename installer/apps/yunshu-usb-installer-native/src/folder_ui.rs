use crate::host::{CompatibilityLevel, HostInfo, detect_host};
use crate::presentation::Surface;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::ffi::{OsStr, c_void};
use std::fs;
use std::io::ErrorKind;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetDC, GetDeviceCaps, LOGPIXELSX, PAINTSTRUCT, ReleaseDC, SetBkMode,
    SetTextColor, TRANSPARENT, WHITE_BRUSH,
};
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, GetStockObject, HBRUSH};
use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW};
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows_sys::Win32::UI::Controls::{
    InitCommonControls, PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW,
};
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BS_OWNERDRAW, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
    WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_PAINT, WS_CLIPCHILDREN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowTextW, HMENU,
    IDC_ARROW, IDYES, KillTimer, LoadCursorW, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONWARNING, MB_OK, MB_YESNO, MSG, MessageBoxW, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_NCCREATE, WM_NCDESTROY,
    WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE,
    WS_MINIMIZEBOX, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use yunshu_usb_writer_core::{
    InstallRequest, PayloadArchitecture, VerifiedInstaller, WriteJobStatus, WritePhase,
    install_from_verified_installer, public_key_from_hex,
    verify_single_exe_installer_with_public_key,
};

const MAIN_CLASS: &str = "YunshuNativeFolderInstallerWindow";
const TIMER_ID: usize = 1;
const ID_TARGET: i32 = 1001;
const ID_BROWSE: i32 = 1002;
const ID_INSTALL: i32 = 1003;
const ID_CANCEL: i32 = 1004;
const ID_OPEN: i32 = 1005;
const ID_HELPER: i32 = 1006;
const ID_NEXT: i32 = 1007;
const ID_BACK: i32 = 1008;
const ID_FINISH: i32 = 1009;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnableWindow(hwnd: HWND, enable: i32) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Verifying,
    Installing,
    Idle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WizardPage {
    Welcome,
    Location,
    Installing,
    Finished,
}

enum UiEvent {
    Verified(Result<Box<VerifiedInstaller>, String>),
    Progress(WriteJobStatus),
    Finished(Result<(WriteJobStatus, PathBuf), String>),
}

struct Controls {
    install_path: HWND,
    progress_label: HWND,
    version: HWND,
    welcome: HWND,
    location_group: HWND,
    location_description: HWND,
    target: HWND,
    browse: HWND,
    info: HWND,
    progress: HWND,
    status: HWND,
    install: HWND,
    cancel: HWND,
    open: HWND,
    helper: HWND,
    next: HWND,
    back: HWND,
    finish: HWND,
    notice: HWND,
}

impl Controls {
    fn empty() -> Self {
        Self {
            install_path: null_mut(),
            progress_label: null_mut(),
            version: null_mut(),
            welcome: null_mut(),
            location_group: null_mut(),
            location_description: null_mut(),
            target: null_mut(),
            browse: null_mut(),
            info: null_mut(),
            progress: null_mut(),
            status: null_mut(),
            install: null_mut(),
            cancel: null_mut(),
            open: null_mut(),
            helper: null_mut(),
            next: null_mut(),
            back: null_mut(),
            finish: null_mut(),
            notice: null_mut(),
        }
    }
}

struct AppState {
    surface: Surface,
    controls: Controls,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    host: HostInfo,
    activity: Activity,
    page: WizardPage,
    verified: Option<VerifiedInstaller>,
    cancel: Arc<AtomicBool>,
    completed_root: Option<PathBuf>,
    risk_trial: bool,
}

pub fn run() -> Result<()> {
    std::hint::black_box(&crate::NATIVE_TEMPLATE_MARKER);
    let host = detect_host()?;
    unsafe {
        SetProcessDPIAware();
    }
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        return Err(std::io::Error::last_os_error()).context("无法读取当前程序模块");
    }
    unsafe { InitCommonControls() };
    register_main_class(instance)?;
    let (tx, rx) = mpsc::channel();
    let mut state = Box::new(AppState {
        surface: Surface::new(),
        controls: Controls::empty(),
        tx,
        rx,
        host,
        activity: Activity::Verifying,
        page: WizardPage::Welcome,
        verified: None,
        cancel: Arc::new(AtomicBool::new(false)),
        completed_root: None,
        risk_trial: false,
    });
    let dc = unsafe { GetDC(null_mut()) };
    let dpi = unsafe { GetDeviceCaps(dc, LOGPIXELSX as i32) } as f32 / 96.0;
    unsafe {
        ReleaseDC(null_mut(), dc);
    }
    let available_width = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f32 - 48.0;
    let available_height = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f32 - 100.0;
    state.surface.scale = dpi
        .min(available_width / 1000.0)
        .min(available_height / 650.0)
        .max(0.75);
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: state.surface.px(1000),
        bottom: state.surface.px(650),
    };
    unsafe {
        AdjustWindowRectEx(&mut bounds, WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, 0);
    }
    let class = wide(MAIN_CLASS);
    let title = wide("云枢AI便携工作站安装程序");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
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
        "云枢AI便携工作站安装程序",
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
            if let Err(error) = create_controls(hwnd, state) {
                show_error(hwnd, &error.to_string());
                return -1;
            }
            unsafe {
                SetTimer(hwnd, TIMER_ID, 80, None);
            }
            start_verification(state);
            0
        }
        WM_TIMER if wparam == TIMER_ID => {
            if let Some(state) = unsafe { app_state(hwnd) } {
                drain_events(hwnd, state);
                state.surface.tick();
            }
            0
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            if let Some(state) = unsafe { app_state(hwnd) } {
                state.surface.paint(dc);
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            0
        }
        WM_DRAWITEM => {
            if let Some(state) = unsafe { app_state(hwnd) } {
                if let Some(draw) = unsafe { (lparam as *const DRAWITEMSTRUCT).as_ref() } {
                    state.surface.draw_button(draw);
                }
            }
            1
        }
        WM_CTLCOLORSTATIC => unsafe {
            SetBkMode(wparam as _, TRANSPARENT as i32);
            SetTextColor(wparam as _, 0x887566);
            GetStockObject(WHITE_BRUSH) as isize
        },
        WM_LBUTTONDOWN | WM_LBUTTONUP => {
            if let Some(state) = unsafe { app_state(hwnd) } {
                let x = (lparam as u16) as i16 as i32;
                let y = ((lparam >> 16) as u16) as i16 as i32;
                if message == WM_LBUTTONDOWN {
                    state.surface.pointer_down(x, y);
                    unsafe {
                        SetCapture(hwnd);
                    }
                } else {
                    unsafe {
                        ReleaseCapture();
                    }
                    state.surface.pointer_up(x, y);
                }
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
                if message_box(
                    hwnd,
                    "安装正在进行。现在关闭会先安全取消并清理暂存目录，是否继续？",
                    "确认取消安装",
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                ) == IDYES
                {
                    if let Some(state) = unsafe { app_state(hwnd) } {
                        state.cancel.store(true, Ordering::Relaxed);
                        set_text(state.controls.status, "正在安全取消，请不要删除安装目录……");
                    }
                }
            } else {
                unsafe {
                    DestroyWindow(hwnd);
                }
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
        WM_NCDESTROY => unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, message, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_controls(hwnd: HWND, state: &mut AppState) -> Result<()> {
    let instance = unsafe { GetModuleHandleW(null()) };
    state.surface.attach(hwnd);
    let font = crate::presentation::font(state.surface.px(15), 400);
    let small = crate::presentation::font(state.surface.px(11), 400);
    let control = |class: &str,
                   text: &str,
                   x: i32,
                   y: i32,
                   width: i32,
                   height: i32,
                   id: i32,
                   button: bool| {
        create_control(
            hwnd,
            instance,
            class,
            text,
            WS_CHILD
                | WS_VISIBLE
                | if button {
                    WS_TABSTOP | BS_OWNERDRAW as u32
                } else if class == "EDIT" {
                    WS_TABSTOP | WS_BORDER
                } else {
                    0
                },
            state.surface.px(x),
            state.surface.px(y),
            state.surface.px(width),
            state.surface.px(height),
            id,
            font,
        )
    };
    state.controls.version = create_control(
        hwnd,
        instance,
        "STATIC",
        "正在校验安装包完整性……",
        WS_CHILD | WS_VISIBLE,
        state.surface.px(560),
        state.surface.px(612),
        state.surface.px(420),
        state.surface.px(34),
        0,
        small,
    )?;
    state.controls.welcome = control(
        "STATIC",
        "离线安装，无需另外安装运行库。\n文件可写入本机或移动存储，\n不改变已有软件和个人文件。",
        580,
        309,
        380,
        95,
        0,
        false,
    )?;
    state.controls.location_description =
        control("STATIC", "安装位置", 580, 303, 370, 25, 0, false)?;
    state.controls.target = control("EDIT", "", 580, 338, 282, 38, ID_TARGET, false)?;
    state.controls.browse = control("BUTTON", "浏览…", 876, 338, 100, 38, ID_BROWSE, true)?;
    state.controls.info = control(
        "STATIC",
        "建议选择一个新的文件夹。",
        580,
        394,
        390,
        48,
        0,
        false,
    )?;
    state.controls.install_path = control("STATIC", "", 580, 305, 395, 67, 0, false)?;
    state.controls.progress_label =
        control("STATIC", "安装进度    0%", 580, 368, 395, 23, 0, false)?;
    state.controls.progress = create_control_raw(
        hwnd,
        instance,
        PROGRESS_CLASSW,
        null(),
        WS_CHILD | WS_VISIBLE,
        state.surface.px(580),
        state.surface.px(392),
        state.surface.px(395),
        state.surface.px(12),
        0,
        font,
    )?;
    unsafe {
        SendMessageW(state.controls.progress, PBM_SETRANGE32, 0, 1000);
        SendMessageW(state.controls.progress, PBM_SETPOS, 0, 0);
    }
    state.controls.status = control(
        "STATIC",
        "正在验证签名、清单和 ZIP64 载荷……",
        580,
        448,
        395,
        59,
        0,
        false,
    )?;
    state.controls.install = control("BUTTON", "开始安装", 842, 522, 134, 42, ID_INSTALL, true)?;
    state.controls.cancel = control("BUTTON", "取消", 716, 522, 108, 42, ID_CANCEL, true)?;
    state.controls.next = control("BUTTON", "下一步", 842, 522, 134, 42, ID_NEXT, true)?;
    state.controls.back = control("BUTTON", "上一步", 580, 522, 108, 42, ID_BACK, true)?;
    state.controls.finish = control("BUTTON", "完成", 872, 522, 104, 42, ID_FINISH, true)?;
    state.controls.open = control("BUTTON", "打开安装目录", 580, 522, 133, 42, ID_OPEN, true)?;
    state.controls.helper = control("BUTTON", "运行环境助手", 727, 522, 131, 42, ID_HELPER, true)?;
    update_control_state(state);
    Ok(())
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
            "正式安装包必须使用ReleaseManifest schema v2，当前为v{}",
            installer.manifest.schema_version
        );
    }
    if installer.manifest.payload_architecture != PayloadArchitecture::X64 {
        bail!("当前安装器只接受x64工作站载荷");
    }
    Ok(installer)
}

fn begin_install(hwnd: HWND, state: &mut AppState) -> Result<()> {
    if state.activity != Activity::Idle {
        return Ok(());
    }
    let target = PathBuf::from(window_text(state.controls.target));
    validate_target_directory(
        &target,
        state.verified.as_ref().context("安装包尚未完成验证")?,
    )?;
    let installer = state.verified.clone().context("安装包尚未完成验证")?;
    if state.host.compatibility == CompatibilityLevel::RiskTrial && !state.risk_trial {
        if message_box(
            hwnd,
            &state.host.reason,
            "Windows 11 ARM64风险试装",
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) != IDYES
        {
            return Ok(());
        }
        state.risk_trial = true;
    }
    state.activity = Activity::Installing;
    state.page = WizardPage::Installing;
    state.cancel = Arc::new(AtomicBool::new(false));
    state.completed_root = None;
    set_text(state.controls.status, "正在创建安全暂存目录……");
    unsafe {
        SendMessageW(state.controls.progress, PBM_SETPOS, 0, 0);
    }
    update_control_state(state);
    let tx = state.tx.clone();
    let cancel = state.cancel.clone();
    thread::spawn(move || {
        let result = run_install_worker(installer, target, tx.clone(), cancel)
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::Finished(result));
    });
    Ok(())
}

fn run_install_worker(
    installer: VerifiedInstaller,
    target: PathBuf,
    tx: Sender<UiEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(WriteJobStatus, PathBuf)> {
    fs::create_dir_all(&target)
        .with_context(|| format!("无法创建安装目录: {}", target.display()))?;
    let target = target
        .canonicalize()
        .with_context(|| format!("无法读取安装目录: {}", target.display()))?;
    let request = InstallRequest::new(target.clone());
    let mut last_sent = Instant::now() - Duration::from_secs(1);
    let mut last_phase = None;
    let status = install_from_verified_installer(
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
    )?;
    Ok((status, target))
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
            set_text(
                state.controls.version,
                &format!(
                    "版本 {}  ·  Build {}  ·  {} 个文件  ·  {}",
                    installer.manifest.product_version,
                    installer.manifest.build_id,
                    installer.manifest.file_count,
                    format_bytes(installer.manifest.expanded_bytes)
                ),
            );
            state.verified = Some(*installer);
            state.activity = Activity::Idle;
            set_text(state.controls.status, "安装包验证通过，请选择安装目录。");
            update_control_state(state);
        }
        UiEvent::Verified(Err(error)) => {
            state.activity = Activity::Idle;
            set_text(state.controls.version, "安装包验证失败");
            set_text(state.controls.status, &error);
            show_error(hwnd, &error);
            update_control_state(state);
        }
        UiEvent::Progress(status) => update_progress(state, &status),
        UiEvent::Finished(Ok((status, root))) => {
            state.activity = Activity::Idle;
            update_progress(state, &status);
            match status.phase {
                WritePhase::Completed => {
                    state.completed_root = Some(root);
                    state.page = WizardPage::Finished;
                    set_text(
                        state.controls.status,
                        "安装完成，全部文件已通过SHA-256校验。",
                    );
                    let _ = write_completion_report(state, &status);
                    message_box(
                        hwnd,
                        "云枢AI便携工作站已安装完成。\n\n首次使用或换电脑时，请运行安装目录中的环境助手。",
                        "安装完成",
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                WritePhase::Cancelled => {
                    state.page = WizardPage::Location;
                    set_text(state.controls.status, "安装已取消，暂存目录已安全清理。")
                }
                _ => {
                    state.page = WizardPage::Location;
                    set_text(state.controls.status, &status.message)
                }
            }
            update_control_state(state);
        }
        UiEvent::Finished(Err(error)) => {
            state.activity = Activity::Idle;
            state.page = WizardPage::Location;
            set_text(state.controls.status, &format!("安装失败：{error}"));
            show_error(hwnd, &format!("安装失败：\n\n{error}"));
            update_control_state(state);
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
        }
        CompatibilityLevel::Blocked => bail!("{}", state.host.reason),
    }
    let _ = hwnd;
    Ok(())
}

fn validate_target_directory(target: &Path, installer: &VerifiedInstaller) -> Result<()> {
    if target.as_os_str().is_empty() || target.is_relative() {
        bail!("安装目录必须是绝对路径");
    }
    if target
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("安装目录不能包含越界路径");
    }
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::create_dir_all(target)?;
            fs::symlink_metadata(target)?
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        bail!("安装目标不是文件夹");
    }
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        bail!("安装目录包含符号链接或重解析点");
    }
    let protected = target
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if protected == r"c:\windows"
        || protected.starts_with(r"c:\windows\")
        || protected == r"c:\program files"
        || protected.starts_with(r"c:\program files\")
    {
        bail!("请不要选择Windows系统目录或Program Files目录，请使用普通可写文件夹");
    }
    let required = installer
        .manifest
        .expanded_bytes
        .saturating_add(64 * 1024 * 1024);
    let mut free = 0_u64;
    let path = wide(&format!("{}\\", target.display()));
    let ok = unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut free, null_mut(), null_mut()) };
    if ok == 0 || free < required {
        bail!(
            "安装目录所在磁盘空间不足，需要至少 {}",
            format_bytes(required)
        );
    }
    let mut fs_name = [0_u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(
            path.as_ptr(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok != 0 {
        let filesystem = String::from_utf16_lossy(&fs_name)
            .trim_end_matches('\0')
            .to_ascii_uppercase();
        if filesystem == "FAT32"
            && installer
                .manifest
                .files
                .iter()
                .any(|file| file.bytes > u32::MAX as u64)
        {
            bail!("FAT32不能保存当前母盘中的单个超大文件，请选择NTFS或exFAT目录");
        }
    }
    Ok(())
}

fn choose_folder(hwnd: HWND) -> Result<Option<PathBuf>> {
    let mut initialized = false;
    let hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED as u32) };
    if hr >= 0 {
        initialized = true;
    }
    let title = wide("选择云枢AI便携工作站安装目录");
    let mut display = [0_u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: hwnd,
        pidlRoot: null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    let pidl = unsafe { SHBrowseForFolderW(&info) };
    let result = if pidl.is_null() {
        None
    } else {
        let mut path = [0_u16; 32768];
        let ok = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) };
        unsafe {
            CoTaskMemFree(pidl.cast());
        }
        if ok == 0 {
            None
        } else {
            Some(PathBuf::from(String::from_utf16_lossy(
                &path[..path
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(path.len())],
            )))
        }
    };
    if initialized {
        unsafe {
            CoUninitialize();
        }
    }
    Ok(result)
}

fn handle_command(hwnd: HWND, state: &mut AppState, wparam: WPARAM) {
    let id = (wparam & 0xffff) as i32;
    let notification = ((wparam >> 16) & 0xffff) as u32;
    match (id, notification) {
        (ID_TARGET, _) => update_control_state(state),
        (ID_NEXT, BN_CLICKED) if state.page == WizardPage::Welcome => {
            if state.verified.is_some() {
                state.page = WizardPage::Location;
                set_text(state.controls.status, "请选择安装目录，然后点击开始安装。");
                update_control_state(state);
            }
        }
        (ID_BACK, BN_CLICKED) if state.page == WizardPage::Location => {
            state.page = WizardPage::Welcome;
            update_control_state(state);
        }
        (ID_BROWSE, BN_CLICKED) if state.activity == Activity::Idle => match choose_folder(hwnd) {
            Ok(Some(path)) => set_text(state.controls.target, &path.to_string_lossy()),
            Ok(None) => {}
            Err(error) => show_error(hwnd, &error.to_string()),
        },
        (ID_INSTALL, BN_CLICKED) => {
            if let Err(error) = begin_install(hwnd, state) {
                show_error(hwnd, &error.to_string());
            }
        }
        (ID_CANCEL, BN_CLICKED) if state.activity == Activity::Installing => {
            state.cancel.store(true, Ordering::Relaxed);
            set_text(state.controls.status, "正在安全取消，请稍候……");
        }
        (ID_CANCEL, BN_CLICKED) => unsafe {
            DestroyWindow(hwnd);
        },
        (ID_FINISH, BN_CLICKED) => unsafe {
            DestroyWindow(hwnd);
        },
        (ID_OPEN, BN_CLICKED) => {
            if let Some(root) = state.completed_root.as_ref() {
                if let Err(error) = open_in_explorer(root) {
                    show_error(hwnd, &error.to_string());
                }
            }
        }
        (ID_HELPER, BN_CLICKED) => {
            if let Some(root) = state.completed_root.as_ref() {
                if let Err(error) = run_environment_helper(state, root) {
                    show_error(hwnd, &error.to_string());
                }
            }
        }
        _ => {}
    }
}

fn update_control_state(state: &mut AppState) {
    state.surface.set_page(match state.page {
        WizardPage::Welcome => 0,
        WizardPage::Location => 1,
        WizardPage::Installing => 2,
        WizardPage::Finished => 3,
    });
    let idle = state.activity == Activity::Idle;
    let has_target = !window_text(state.controls.target).trim().is_empty();
    let location_page = state.page == WizardPage::Location;
    set_enabled(
        state.controls.target,
        idle && location_page && state.verified.is_some(),
    );
    set_enabled(
        state.controls.browse,
        idle && location_page && state.verified.is_some(),
    );
    set_enabled(
        state.controls.install,
        idle && location_page && state.verified.is_some() && has_target,
    );
    set_enabled(
        state.controls.cancel,
        state.activity == Activity::Installing,
    );
    set_enabled(
        state.controls.next,
        state.activity == Activity::Idle
            && state.page == WizardPage::Welcome
            && state.verified.is_some(),
    );
    set_enabled(
        state.controls.back,
        state.activity == Activity::Idle && state.page == WizardPage::Location,
    );
    set_enabled(state.controls.finish, state.page == WizardPage::Finished);
    show_control(state.controls.welcome, state.page == WizardPage::Welcome);
    show_control(
        state.controls.location_group,
        state.page == WizardPage::Location,
    );
    show_control(
        state.controls.location_description,
        state.page == WizardPage::Location,
    );
    show_control(state.controls.target, state.page == WizardPage::Location);
    show_control(state.controls.browse, state.page == WizardPage::Location);
    show_control(state.controls.info, state.page == WizardPage::Location);
    show_control(state.controls.next, state.page == WizardPage::Welcome);
    show_control(state.controls.back, state.page == WizardPage::Location);
    show_control(state.controls.install, state.page == WizardPage::Location);
    show_control(
        state.controls.cancel,
        matches!(
            state.page,
            WizardPage::Welcome | WizardPage::Location | WizardPage::Installing
        ),
    );
    show_control(state.controls.open, state.page == WizardPage::Finished);
    show_control(state.controls.helper, state.page == WizardPage::Finished);
    show_control(state.controls.finish, state.page == WizardPage::Finished);
    show_control(
        state.controls.progress,
        matches!(state.page, WizardPage::Installing | WizardPage::Finished),
    );
    show_control(state.controls.location_group, false);
    show_control(state.controls.notice, false);
    show_control(
        state.controls.install_path,
        matches!(state.page, WizardPage::Installing | WizardPage::Finished),
    );
    show_control(
        state.controls.progress_label,
        matches!(state.page, WizardPage::Installing | WizardPage::Finished),
    );
    set_text(
        state.controls.install_path,
        &format!("安装位置\n{}", window_text(state.controls.target)),
    );
}

fn update_progress(state: &AppState, status: &WriteJobStatus) {
    let position = match status.phase {
        WritePhase::Preflight => 20,
        WritePhase::Staging => 50,
        WritePhase::Writing => {
            50 + ((status.processed_bytes.saturating_mul(850) / status.total_bytes.max(1)).min(850)
                as usize)
        }
        WritePhase::Verifying => 930,
        WritePhase::Committing => 980,
        WritePhase::Completed => 1000,
        WritePhase::Cancelled | WritePhase::Failed => 0,
    };
    unsafe {
        SendMessageW(state.controls.progress, PBM_SETPOS, position, 0);
    }
    set_text(
        state.controls.progress_label,
        &format!("安装进度    {}%", position / 10),
    );
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
        status.message.replace("U盘", "安装目录")
    };
    set_text(state.controls.status, &message);
}

fn run_environment_helper(state: &AppState, root: &Path) -> Result<()> {
    let manifest = &state
        .verified
        .as_ref()
        .context("安装包信息不可用")?
        .manifest;
    let helper = manifest
        .root_mappings
        .iter()
        .map(|item| item.destination.as_str())
        .find(|value| {
            let lower = value.to_ascii_lowercase();
            lower.ends_with(".exe") && value.contains("环境") && value.contains("助手")
        })
        .map(|value| root.join(value.replace('/', "\\")))
        .or_else(|| {
            [
                "云枢AI工作站环境助手.exe",
                "云枢AI工作站环境检测助手.exe",
                "云枢AI环境检测助手.exe",
            ]
            .into_iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
        })
        .context("安装目录未找到环境助手")?;
    std::process::Command::new(&helper)
        .spawn()
        .with_context(|| format!("无法启动环境助手: {}", helper.display()))?;
    Ok(())
}

fn open_in_explorer(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .context("无法打开安装目录")?;
    Ok(())
}

fn write_completion_report(state: &AppState, status: &WriteJobStatus) -> Result<PathBuf> {
    let installer = state.verified.as_ref().context("安装包信息不可用")?;
    let directory = std::env::temp_dir().join("YunshuAI-Installer/reports");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("install-{}.json", status.operation.operation_id));
    let report = serde_json::json!({"schemaVersion": 2, "createdAt": Utc::now().to_rfc3339(), "result": "completed", "operationId": status.operation.operation_id, "productVersion": installer.manifest.product_version, "buildId": installer.manifest.build_id, "payloadArchitecture": installer.manifest.payload_architecture, "hostCompatibility": state.host.compatibility_id, "riskTrial": state.risk_trial, "targetKind": "directory", "fileCount": status.total_files, "expandedBytes": status.total_bytes});
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok(path)
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
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
    unsafe {
        SendMessageW(control, WM_SETFONT, font as usize, 1);
    }
    Ok(control)
}

fn register_main_class(instance: HINSTANCE) -> Result<()> {
    let class = wide(MAIN_CLASS);
    let definition = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(main_window_proc),
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
fn set_enabled(hwnd: HWND, enabled: bool) {
    if !hwnd.is_null() {
        unsafe {
            EnableWindow(hwnd, i32::from(enabled));
        }
    }
}
fn show_control(hwnd: HWND, visible: bool) {
    if !hwnd.is_null() {
        unsafe {
            ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
    }
}
fn set_text(hwnd: HWND, value: &str) {
    if hwnd.is_null() {
        return;
    }
    let value = wide(value);
    unsafe {
        SetWindowTextW(hwnd, value.as_ptr());
    }
}
fn window_text(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 32768];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}
fn message_box(hwnd: HWND, message: &str, title: &str, style: u32) -> i32 {
    let message = wide(message);
    let title = wide(title);
    unsafe { MessageBoxW(hwnd, message.as_ptr(), title.as_ptr(), style) }
}
fn show_error(hwnd: HWND, message: &str) {
    message_box(
        hwnd,
        message,
        "云枢AI便携工作站安装程序",
        MB_OK | MB_ICONERROR,
    );
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
    #[test]
    fn directory_install_contract_is_not_usb_only() {
        assert_eq!("directory", "directory");
    }
}
