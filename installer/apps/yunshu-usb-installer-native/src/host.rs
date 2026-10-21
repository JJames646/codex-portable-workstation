use anyhow::{Result, bail};
use serde::Serialize;
use std::mem::size_of;
use windows_sys::Win32::System::SystemInformation::{
    GetNativeSystemInfo, OSVERSIONINFOW, PROCESSOR_ARCHITECTURE_AMD64,
    PROCESSOR_ARCHITECTURE_ARM64, SYSTEM_INFO,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatibilityLevel {
    Supported,
    RiskTrial,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub architecture: String,
    pub windows_build: u32,
    pub compatibility: CompatibilityLevel,
    pub compatibility_id: String,
    pub reason: String,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

pub fn detect_host() -> Result<HostInfo> {
    let mut system = SYSTEM_INFO::default();
    unsafe { GetNativeSystemInfo(&mut system) };
    let architecture = unsafe { system.Anonymous.Anonymous.wProcessorArchitecture };

    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    let status = unsafe { RtlGetVersion(&mut version) };
    if status < 0 {
        bail!("Windows版本检测失败: NTSTATUS 0x{:08x}", status as u32);
    }

    Ok(classify_host(architecture, version.dwBuildNumber))
}

fn classify_host(architecture: u16, build: u32) -> HostInfo {
    match architecture {
        PROCESSOR_ARCHITECTURE_AMD64 => HostInfo {
            architecture: "x64".into(),
            windows_build: build,
            compatibility: CompatibilityLevel::Supported,
            compatibility_id: if build >= 22000 {
                "windows-11-x64"
            } else {
                "windows-10-x64"
            }
            .into(),
            reason: "当前x64工作站载荷正式支持此系统".into(),
        },
        PROCESSOR_ARCHITECTURE_ARM64 if build >= 22000 => HostInfo {
            architecture: "arm64".into(),
            windows_build: build,
            compatibility: CompatibilityLevel::RiskTrial,
            compatibility_id: "windows-11-arm64".into(),
            reason: "Windows 11 ARM64只允许x64兼容风险试装，不代表三智能体已完整验证".into(),
        },
        PROCESSOR_ARCHITECTURE_ARM64 => HostInfo {
            architecture: "arm64".into(),
            windows_build: build,
            compatibility: CompatibilityLevel::Blocked,
            compatibility_id: "windows-10-arm64".into(),
            reason: "Windows 10 ARM64暂不支持当前x64工作站载荷".into(),
        },
        _ => HostInfo {
            architecture: "x86-or-unknown".into(),
            windows_build: build,
            compatibility: CompatibilityLevel::Blocked,
            compatibility_id: "windows-x86".into(),
            reason: "32位或未识别的Windows架构不支持当前x64工作站载荷".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_supported_and_risk_hosts() {
        assert_eq!(
            classify_host(PROCESSOR_ARCHITECTURE_AMD64, 19045).compatibility,
            CompatibilityLevel::Supported
        );
        assert_eq!(
            classify_host(PROCESSOR_ARCHITECTURE_ARM64, 22621).compatibility,
            CompatibilityLevel::RiskTrial
        );
        assert_eq!(
            classify_host(PROCESSOR_ARCHITECTURE_ARM64, 19045).compatibility,
            CompatibilityLevel::Blocked
        );
    }
}
