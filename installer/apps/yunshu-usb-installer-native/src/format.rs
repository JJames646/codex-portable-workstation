use crate::disk::{DiskIdentity, NativeTargetDiskInfo, list_installer_targets, revalidate_target};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

const FORMAT_HELPER_ARGUMENT: &str = "--format-helper";
const FORMAT_HELPER_SCHEMA: u32 = 1;
const FORMAT_HELPER_MAX_AGE_SECONDS: i64 = 10 * 60;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormatRequest {
    schema_version: u32,
    created_at_unix: i64,
    target_root: String,
    target_label: String,
    required_bytes: u64,
    identity: DiskIdentity,
}

pub fn run_elevated_helper_if_requested() -> Result<Option<i32>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(FORMAT_HELPER_ARGUMENT)) {
        return Ok(None);
    }
    let request_path = args.next().context("格式化请求缺少路径")?;
    let expected_hash = args
        .next()
        .and_then(|value| value.into_string().ok())
        .context("格式化请求缺少哈希")?;
    if args.next().is_some() {
        bail!("格式化请求参数过多");
    }
    run_format_helper(Path::new(&request_path), &expected_hash)?;
    Ok(Some(0))
}

pub fn format_fat32_to_exfat(
    expected: &DiskIdentity,
    required_bytes: u64,
    installer_path: &Path,
) -> Result<NativeTargetDiskInfo> {
    let before = list_installer_targets(required_bytes, installer_path)?;
    let current = revalidate_target(expected, &before)?;
    if !current.filesystem.eq_ignore_ascii_case("FAT32") || !current.format_eligible {
        bail!("目标USB已不再符合FAT32安全格式化条件");
    }
    validate_drive_root(&current.root)?;
    let request = FormatRequest {
        schema_version: FORMAT_HELPER_SCHEMA,
        created_at_unix: Utc::now().timestamp(),
        target_root: current.root.clone(),
        target_label: current.label.clone(),
        required_bytes,
        identity: expected.clone(),
    };
    let (request_path, request_hash) = write_format_request(&request)?;
    let helper_result = run_elevated_helper(&request_path, &request_hash);
    let _ = fs::remove_file(&request_path);
    helper_result?;

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_error = None;
    while Instant::now() < deadline {
        match list_installer_targets(required_bytes, installer_path) {
            Ok(values) => match revalidate_target(expected, &values) {
                Ok(value)
                    if value.filesystem.eq_ignore_ascii_case("exFAT") && value.install_eligible =>
                {
                    return Ok(value.clone());
                }
                Ok(_) => {}
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error.to_string()),
        }
        thread::sleep(Duration::from_millis(500));
    }
    bail!(
        "exFAT格式化已结束，但未能重新确认原USB身份和卷状态{}",
        last_error
            .map(|value| format!(": {value}"))
            .unwrap_or_default()
    )
}

fn write_format_request(request: &FormatRequest) -> Result<(PathBuf, String)> {
    let directory = std::env::temp_dir().join("YunshuAI-UsbInstaller/format-requests");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("format-{}.json", Uuid::new_v4()));
    let bytes = serde_json::to_vec(request)?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok((path, hash))
}

fn run_elevated_helper(request_path: &Path, request_hash: &str) -> Result<()> {
    let executable = std::env::current_exe().context("无法识别当前用户安装器")?;
    let parameters = format!(
        "{} {} {}",
        FORMAT_HELPER_ARGUMENT,
        quote_argument(request_path.as_os_str()),
        request_hash
    );
    let verb = wide("runas");
    let file = wide_os(executable.as_os_str());
    let parameters = wide(&parameters);
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: null(),
        nShow: SW_HIDE,
        hInstApp: null_mut(),
        lpIDList: null_mut(),
        lpClass: null(),
        hkeyClass: null_mut(),
        dwHotKey: 0,
        Anonymous: Default::default(),
        hProcess: null_mut(),
    };
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        let code = unsafe { GetLastError() };
        if code == 1223 {
            bail!("用户取消了Windows管理员授权，未格式化U盘");
        }
        return Err(std::io::Error::last_os_error()).context("无法启动提权格式化助手");
    }
    if execute.hProcess.is_null() {
        bail!("格式化助手未返回可等待句柄");
    }
    let wait = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    if wait != WAIT_OBJECT_0 {
        unsafe { CloseHandle(execute.hProcess) };
        bail!("等待格式化完成时发生异常");
    }
    let mut exit_code = 1_u32;
    let got_exit = unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) };
    unsafe { CloseHandle(execute.hProcess) };
    if got_exit == 0 {
        return Err(std::io::Error::last_os_error()).context("无法读取格式化助手退出码");
    }
    if exit_code != 0 {
        bail!("Windows格式化失败，退出码: {exit_code}");
    }
    Ok(())
}

fn run_format_helper(path: &Path, expected_hash: &str) -> Result<()> {
    let bytes = fs::read(path).context("无法读取格式化请求")?;
    let actual_hash = hex::encode(Sha256::digest(&bytes));
    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        bail!("格式化请求已被篡改");
    }
    let request: FormatRequest = serde_json::from_slice(&bytes).context("格式化请求格式无效")?;
    let _ = fs::remove_file(path);
    if request.schema_version != FORMAT_HELPER_SCHEMA {
        bail!("格式化请求版本不受支持");
    }
    let age = Utc::now().timestamp() - request.created_at_unix;
    if !(0..=FORMAT_HELPER_MAX_AGE_SECONDS).contains(&age) {
        bail!("格式化请求已过期");
    }
    validate_drive_root(&request.target_root)?;
    let executable = std::env::current_exe()?;
    let values = list_installer_targets(request.required_bytes, &executable)?;
    let current = revalidate_target(&request.identity, &values)?;
    if !current.root.eq_ignore_ascii_case(&request.target_root)
        || !current.filesystem.eq_ignore_ascii_case("FAT32")
        || !current.format_eligible
    {
        bail!("提权后目标USB身份或格式化条件发生变化");
    }
    run_system_format(&request.target_root, &request.target_label)
}

fn run_system_format(root: &str, current_label: &str) -> Result<()> {
    let system_root = std::env::var_os("SystemRoot").context("无法识别Windows系统目录")?;
    let format_path = Path::new(&system_root).join("System32/format.com");
    if !format_path.is_file() {
        bail!("Windows内置format.com不存在");
    }
    let drive = &root[..2];
    let mut child = Command::new(format_path)
        .arg(drive)
        .args(["/FS:exFAT", "/Q", "/V:YUNSHU_AI", "/X"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("无法启动Windows内置格式化工具")?;
    if let Some(mut input) = child.stdin.take() {
        // format.com 在不同卷状态下可能先询问当前卷标，也可能只等待回车；
        // 始终发送第一行可同时覆盖“空卷标/按回车继续”的情况，再发送明确确认。
        writeln!(input, "{}", current_label.trim())?;
        writeln!(input, "Y")?;
    }
    let deadline = Instant::now() + Duration::from_secs(5 * 60);
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!("Windows格式化失败，退出码: {}", status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Windows格式化超时，已终止，未继续写入工作站");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn validate_drive_root(root: &str) -> Result<()> {
    let bytes = root.as_bytes();
    if bytes.len() != 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\'
    {
        bail!("目标盘符格式无效，拒绝格式化");
    }
    Ok(())
}

fn quote_argument(value: &OsStr) -> String {
    let value = value.to_string_lossy().replace('"', "\\\"");
    format!("\"{value}\"")
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_drive_roots_for_formatting() {
        assert!(validate_drive_root("U:\\").is_ok());
        assert!(validate_drive_root("C:\\Windows").is_err());
        assert!(validate_drive_root("\\\\server\\share").is_err());
    }

    #[test]
    fn quote_argument_wraps_generated_request_path() {
        assert_eq!(
            quote_argument(OsStr::new(r"C:\Temp\request.json")),
            r#""C:\Temp\request.json""#
        );
    }
}
