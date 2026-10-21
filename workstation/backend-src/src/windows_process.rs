use std::process::{Child, Command};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessWindowProbe {
    pub ready: bool,
    pub process_id: Option<u32>,
    pub window_handle: Option<u64>,
    pub title: Option<String>,
    pub visible: bool,
    pub minimized: bool,
    pub responsive: bool,
}

#[cfg(windows)]
mod platform {
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Child, Command};
    use std::ptr::null;

    use anyhow::{Context, Result};

    use super::ProcessWindowProbe;

    type Handle = *mut c_void;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const STILL_ACTIVE: u32 = 259;
    const SW_SHOWNORMAL: i32 = 1;
    const SW_RESTORE: i32 = 9;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const WM_CLOSE: u32 = 0x0010;
    const WM_NULL: u32 = 0;
    const MAX_PATH: usize = 260;

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        base_priority: i32,
        flags: u32,
        executable_file: [u16; MAX_PATH],
    }

    #[derive(Debug)]
    struct WindowCandidate {
        process_id: u32,
        handle: Handle,
        title: Option<String>,
        visible: bool,
        minimized: bool,
        responsive: bool,
        width: i32,
        height: i32,
    }

    struct WindowEnumeration<'a> {
        process_ids: &'a HashSet<u32>,
        candidates: Vec<WindowCandidate>,
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    #[link(name = "User32")]
    unsafe extern "system" {
        fn EnumWindows(
            callback: Option<unsafe extern "system" fn(Handle, isize) -> i32>,
            parameter: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(window: Handle, process_id: *mut u32) -> u32;
        fn IsWindowVisible(window: Handle) -> i32;
        fn IsIconic(window: Handle) -> i32;
        fn GetWindowRect(window: Handle, rect: *mut Rect) -> i32;
        fn GetWindowTextLengthW(window: Handle) -> i32;
        fn GetWindowTextW(window: Handle, text: *mut u16, maximum_count: i32) -> i32;
        fn SendMessageTimeoutW(
            window: Handle,
            message: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout_ms: u32,
            result: *mut usize,
        ) -> usize;
        fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
        fn ShowWindow(window: Handle, command: i32) -> i32;
        fn SetForegroundWindow(window: Handle) -> i32;
    }

    #[link(name = "Shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            window: Handle,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_command: i32,
        ) -> Handle;
    }

    #[derive(Debug)]
    pub struct ProcessJob {
        handle: Handle,
    }

    unsafe impl Send for ProcessJob {}

    impl ProcessJob {
        pub fn attach(child: &Child) -> Result<Self> {
            let handle = unsafe { CreateJobObjectW(null(), null()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error()).context("CreateJobObjectW失败");
            }

            let mut limits: ExtendedLimitInformation = unsafe { zeroed() };
            limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&limits as *const ExtendedLimitInformation).cast(),
                    size_of::<ExtendedLimitInformation>() as u32,
                )
            };
            if configured == 0 {
                let error = std::io::Error::last_os_error();
                unsafe {
                    CloseHandle(handle);
                }
                return Err(error).context("设置Job Object失败");
            }

            let assigned =
                unsafe { AssignProcessToJobObject(handle, child.as_raw_handle().cast()) };
            if assigned == 0 {
                let error = std::io::Error::last_os_error();
                unsafe {
                    CloseHandle(handle);
                }
                return Err(error).context("把引擎加入Job Object失败");
            }
            Ok(Self { handle })
        }

        pub fn terminate(&self, exit_code: u32) -> Result<()> {
            let terminated = unsafe { TerminateJobObject(self.handle, exit_code) };
            if terminated == 0 {
                return Err(std::io::Error::last_os_error()).context("终止Job Object失败");
            }
            Ok(())
        }
    }

    impl Drop for ProcessJob {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
                self.handle = null::<c_void>() as Handle;
            }
        }
    }

    pub fn configure_command(command: &mut Command, open_new_console: bool) {
        let flags = if open_new_console {
            CREATE_NEW_CONSOLE
        } else {
            CREATE_NO_WINDOW
        };
        command.creation_flags(flags);
    }

    pub fn process_is_alive(process_id: u32) -> bool {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0_u32;
        let succeeded = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
        unsafe {
            CloseHandle(handle);
        }
        succeeded != 0 && exit_code == STILL_ACTIVE
    }

    pub fn probe_process_tree_window(process_id: u32) -> ProcessWindowProbe {
        let process_ids = process_tree_ids(process_id);
        if process_ids.is_empty() {
            return ProcessWindowProbe::default();
        }
        let mut candidates = enumerate_windows(&process_ids);
        candidates.sort_by_key(|candidate| {
            (
                !candidate.visible,
                candidate.minimized,
                -(candidate.width.saturating_mul(candidate.height)),
            )
        });
        let Some(mut candidate) = candidates.into_iter().next() else {
            return ProcessWindowProbe::default();
        };
        // 只探测最可能的主窗口。旧实现会对进程树中每个窗口同步等待 800ms，
        // Electron 卡顿时一次状态查询可能阻塞数秒，并进一步拖住启动器。
        candidate.responsive = window_is_responsive(candidate.handle);
        let ready = candidate.visible
            && !candidate.minimized
            && candidate.responsive
            && candidate.width >= 200
            && candidate.height >= 120;
        ProcessWindowProbe {
            ready,
            process_id: Some(candidate.process_id),
            window_handle: Some(candidate.handle as usize as u64),
            title: candidate.title,
            visible: candidate.visible,
            minimized: candidate.minimized,
            responsive: candidate.responsive,
        }
    }

    pub fn restore_process_tree_window(process_id: u32) -> Result<bool> {
        let process_ids = process_tree_ids(process_id);
        let mut candidates = enumerate_windows(&process_ids);
        candidates.retain(|candidate| {
            candidate.width >= 200
                && candidate.height >= 120
                && (candidate.visible || candidate.title.is_some())
        });
        candidates.sort_by_key(|candidate| {
            (
                !candidate.visible,
                candidate.title.is_none(),
                -(candidate.width.saturating_mul(candidate.height)),
            )
        });
        let Some(candidate) = candidates.into_iter().next() else {
            return Ok(false);
        };
        unsafe {
            ShowWindow(candidate.handle, SW_RESTORE);
            SetForegroundWindow(candidate.handle);
        }
        Ok(true)
    }

    pub fn open_url_native(url: &str) -> Result<()> {
        let operation = std::ffi::OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = std::ffi::OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                null::<c_void>() as Handle,
                operation.as_ptr(),
                target.as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            anyhow::bail!("Windows无法通过默认浏览器打开本地Control UI，ShellExecuteW={result}");
        }
        Ok(())
    }

    fn process_tree_ids(root_process_id: u32) -> HashSet<u32> {
        let mut process_ids = HashSet::from([root_process_id]);
        let mut parents = HashMap::new();
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == (-1_isize as Handle) || snapshot.is_null() {
            return process_ids;
        }
        let mut entry: ProcessEntry32W = unsafe { zeroed() };
        entry.size = size_of::<ProcessEntry32W>() as u32;
        let mut available = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while available {
            parents.insert(entry.process_id, entry.parent_process_id);
            available = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe {
            CloseHandle(snapshot);
        }
        loop {
            let before = process_ids.len();
            for (child, parent) in &parents {
                if process_ids.contains(parent) {
                    process_ids.insert(*child);
                }
            }
            if process_ids.len() == before {
                break;
            }
        }
        process_ids
    }

    fn enumerate_windows(process_ids: &HashSet<u32>) -> Vec<WindowCandidate> {
        let mut enumeration = WindowEnumeration {
            process_ids,
            candidates: Vec::new(),
        };
        unsafe {
            EnumWindows(
                Some(enum_window_callback),
                (&mut enumeration as *mut WindowEnumeration<'_>) as isize,
            );
        }
        enumeration.candidates
    }

    unsafe extern "system" fn enum_window_callback(window: Handle, parameter: isize) -> i32 {
        let enumeration = unsafe { &mut *(parameter as *mut WindowEnumeration<'_>) };
        let mut process_id = 0_u32;
        unsafe {
            GetWindowThreadProcessId(window, &mut process_id);
        }
        if !enumeration.process_ids.contains(&process_id) {
            return 1;
        }
        let mut rect: Rect = unsafe { zeroed() };
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return 1;
        }
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if width <= 0 || height <= 0 {
            return 1;
        }
        let visible = unsafe { IsWindowVisible(window) } != 0;
        let minimized = unsafe { IsIconic(window) } != 0;
        enumeration.candidates.push(WindowCandidate {
            process_id,
            handle: window,
            title: window_title(window),
            visible,
            minimized,
            responsive: false,
            width,
            height,
        });
        1
    }

    fn window_is_responsive(window: Handle) -> bool {
        let mut response = 0_usize;
        (unsafe {
            SendMessageTimeoutW(window, WM_NULL, 0, 0, SMTO_ABORTIFHUNG, 120, &mut response)
        }) != 0
    }

    fn window_title(window: Handle) -> Option<String> {
        let length = unsafe { GetWindowTextLengthW(window) };
        if length <= 0 {
            return None;
        }
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
        if copied <= 0 {
            return None;
        }
        let title = String::from_utf16_lossy(&buffer[..copied as usize]);
        (!title.trim().is_empty()).then_some(title)
    }

    pub fn request_graceful_tree_stop(process_id: u32) -> Result<bool> {
        // 对 Electron 主窗口先发送 WM_CLOSE，让官方程序自行刷新 SQLite、LevelDB
        // 和插件 marketplace。只有没有可关闭窗口的 headless 引擎才回退到 taskkill /T。
        let process_ids = process_tree_ids(process_id);
        let mut requested = false;
        for candidate in enumerate_windows(&process_ids) {
            if unsafe { PostMessageW(candidate.handle, WM_CLOSE, 0, 0) } != 0 {
                requested = true;
            }
        }
        if requested {
            return Ok(true);
        }

        let system_root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .context("当前子进程环境缺少SystemRoot/WINDIR，无法定位taskkill.exe")?;
        let taskkill = std::path::PathBuf::from(system_root)
            .join("System32")
            .join("taskkill.exe");
        let status = Command::new(taskkill)
            .args(["/PID", &process_id.to_string(), "/T"])
            .status()
            .context("无法请求引擎进程树正常退出")?;
        Ok(status.success())
    }

    pub fn replace_file_atomic(source: &Path, destination: &Path) -> Result<()> {
        let source_wide = win32_extended_path(source);
        let destination_wide = win32_extended_path(destination);
        let replaced = unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "无法原子替换状态文件 {} -> {}",
                    source.display(),
                    destination.display()
                )
            });
        }
        Ok(())
    }

    /// Win32 文件 API 即使在系统已启用 LongPathsEnabled 时，也不能保证自动接受
    /// 超过 MAX_PATH 的普通 `C:\...` 路径。显式添加 `\\?\`（UNC 使用
    /// `\\?\UNC\`）后，深层 Skills、插件和 Python 包才能安全进行原子替换。
    fn win32_extended_path(path: &Path) -> Vec<u16> {
        let backslash = b'\\' as u16;
        let slash = b'/' as u16;
        let question = b'?' as u16;
        let dot = b'.' as u16;
        // `\\?\` 会关闭 Win32 的路径规范化，必须自行把 `/` 转成 `\`。
        let raw = path
            .as_os_str()
            .encode_wide()
            .map(|unit| if unit == slash { backslash } else { unit })
            .collect::<Vec<_>>();
        let already_verbatim = raw.starts_with(&[backslash, backslash, question, backslash])
            || raw.starts_with(&[backslash, backslash, dot, backslash]);

        let mut extended = if already_verbatim || !path.is_absolute() {
            raw
        } else if raw.starts_with(&[backslash, backslash]) {
            let mut value = r"\\?\UNC\".encode_utf16().collect::<Vec<_>>();
            value.extend_from_slice(&raw[2..]);
            value
        } else {
            let mut value = r"\\?\".encode_utf16().collect::<Vec<_>>();
            value.extend_from_slice(&raw);
            value
        };
        extended.push(0);
        extended
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;
    use std::process::{Child, Command};

    use anyhow::{Context, Result};

    use super::ProcessWindowProbe;

    #[derive(Debug, Default)]
    pub struct ProcessJob;

    impl ProcessJob {
        pub fn attach(_child: &Child) -> Result<Self> {
            Ok(Self)
        }

        pub fn terminate(&self, _exit_code: u32) -> Result<()> {
            Ok(())
        }
    }

    pub fn configure_command(_command: &mut Command, _open_new_console: bool) {}

    pub fn process_is_alive(process_id: u32) -> bool {
        Command::new("kill")
            .args(["-0", &process_id.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    }

    pub fn probe_process_tree_window(_process_id: u32) -> ProcessWindowProbe {
        ProcessWindowProbe::default()
    }

    pub fn restore_process_tree_window(_process_id: u32) -> Result<bool> {
        Ok(false)
    }

    pub fn open_url_native(url: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        let mut command = Command::new("open");
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = Command::new("xdg-open");
        command
            .arg(url)
            .spawn()
            .context("无法通过默认浏览器打开本地Control UI")?;
        Ok(())
    }

    pub fn request_graceful_tree_stop(process_id: u32) -> Result<bool> {
        Ok(Command::new("kill")
            .args(["-TERM", &process_id.to_string()])
            .status()?
            .success())
    }

    pub fn replace_file_atomic(source: &Path, destination: &Path) -> Result<()> {
        if destination.exists() {
            std::fs::remove_file(destination)
                .with_context(|| format!("无法替换 {}", destination.display()))?;
        }
        std::fs::rename(source, destination)
            .with_context(|| format!("无法提交 {}", destination.display()))
    }
}

pub use platform::ProcessJob;

pub fn configure_command(command: &mut Command, open_new_console: bool) {
    platform::configure_command(command, open_new_console);
}

pub fn process_is_alive(process_id: u32) -> bool {
    platform::process_is_alive(process_id)
}

pub fn probe_process_tree_window(process_id: u32) -> ProcessWindowProbe {
    platform::probe_process_tree_window(process_id)
}

pub fn restore_process_tree_window(process_id: u32) -> Result<bool> {
    platform::restore_process_tree_window(process_id)
        .with_context(|| format!("无法恢复受控进程树窗口 PID={process_id}"))
}

pub fn open_url_native(url: &str) -> Result<()> {
    platform::open_url_native(url)
}

pub fn request_graceful_tree_stop(process_id: u32) -> Result<bool> {
    platform::request_graceful_tree_stop(process_id)
        .with_context(|| format!("无法停止受控进程树 PID={process_id}"))
}

pub fn replace_file_atomic(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    platform::replace_file_atomic(source, destination)
}

pub fn force_terminate_job(job: &ProcessJob, exit_code: u32) -> Result<()> {
    job.terminate(exit_code)
}

pub fn terminate_child_as_fallback(child: &mut Child) -> Result<()> {
    child.kill().context("最终兜底终止受控根进程失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_file_atomic_replaces_existing_state_file() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("state.json.tmp");
        let destination = temporary.path().join("state.json");
        std::fs::write(&source, b"new-state").unwrap();
        std::fs::write(&destination, b"old-state").unwrap();

        replace_file_atomic(&source, &destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new-state");
        assert!(!source.exists());
    }

    #[cfg(windows)]
    #[test]
    fn replace_file_atomic_supports_windows_paths_longer_than_max_path() {
        use std::os::windows::ffi::OsStrExt;

        let temporary = tempfile::tempdir().unwrap();
        let mut directory = temporary.path().to_path_buf();
        for index in 0..12 {
            directory = directory.join(format!("deep-plugin-directory-{index:02}"));
        }
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("plugin-script.mjs.tmp");
        let destination = directory.join("plugin-script.mjs");
        assert!(destination.as_os_str().encode_wide().count() > 260);
        std::fs::write(&source, b"new-plugin").unwrap();
        std::fs::write(&destination, b"old-plugin").unwrap();

        replace_file_atomic(&source, &destination).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"new-plugin");
    }
}
