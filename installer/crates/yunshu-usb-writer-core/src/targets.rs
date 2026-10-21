#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDiskInfo {
    pub root: String,
    pub label: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub disk_number: u32,
    pub model: String,
    pub serial: String,
    pub bus_type: String,
    pub read_only: bool,
    pub system_disk: bool,
    pub boot_partition: bool,
    pub eligible: bool,
    pub reason: Option<String>,
}

impl TargetDiskInfo {
    #[cfg(any(windows, test))]
    fn finalize(mut self, required_bytes: u64, excluded_root: Option<&Path>) -> Self {
        let excluded = excluded_root
            .and_then(volume_root)
            .is_some_and(|root| root.eq_ignore_ascii_case(&self.root));
        let reason = if !self.bus_type.eq_ignore_ascii_case("USB") {
            Some("只允许选择USB移动磁盘".to_owned())
        } else if self.system_disk || self.boot_partition {
            Some("Windows系统盘或启动分区不可写入".to_owned())
        } else if self.read_only {
            Some("目标磁盘处于只读状态".to_owned())
        } else if !matches!(
            self.filesystem.to_ascii_uppercase().as_str(),
            "NTFS" | "EXFAT"
        ) {
            Some(if self.filesystem.eq_ignore_ascii_case("FAT32") {
                "FAT32不支持超过4GB的工作站文件".to_owned()
            } else {
                "仅支持NTFS或exFAT".to_owned()
            })
        } else if self.free_bytes < required_bytes {
            Some("目标磁盘剩余空间不足".to_owned())
        } else if excluded {
            Some("安装器所在磁盘不能作为写入目标".to_owned())
        } else {
            None
        };
        self.eligible = reason.is_none();
        self.reason = reason;
        self
    }
}

#[cfg(windows)]
pub fn list_usb_targets(
    required_bytes: u64,
    excluded_root: Option<&Path>,
) -> Result<Vec<TargetDiskInfo>> {
    use std::process::Command;
    let script = r#"
$ErrorActionPreference='Stop'
$items = foreach ($volume in Get-Volume | Where-Object DriveLetter) {
  $partition = Get-Partition -DriveLetter $volume.DriveLetter -ErrorAction SilentlyContinue
  if (-not $partition) { continue }
  $disk = Get-Disk -Number $partition.DiskNumber -ErrorAction SilentlyContinue
  if (-not $disk -or $disk.BusType -ne 'USB') { continue }
  [pscustomobject]@{
    root = ([string]$volume.DriveLetter + ':\')
    label = [string]$volume.FileSystemLabel
    filesystem = [string]$volume.FileSystem
    totalBytes = [uint64]$volume.Size
    freeBytes = [uint64]$volume.SizeRemaining
    diskNumber = [uint32]$disk.Number
    model = [string]$disk.FriendlyName
    serial = [string]$disk.SerialNumber
    busType = [string]$disk.BusType
    readOnly = [bool]$disk.IsReadOnly
    systemDisk = [bool]$disk.IsSystem
    bootPartition = [bool]$partition.IsBoot
    eligible = $false
    reason = $null
  }
}
ConvertTo-Json -InputObject @($items) -Compress
"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .context("无法调用Windows磁盘检测")?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows磁盘检测失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8(output.stdout).context("磁盘检测输出不是UTF-8")?;
    let values: Vec<TargetDiskInfo> =
        serde_json::from_str(raw.trim()).context("磁盘检测输出格式错误")?;
    Ok(values
        .into_iter()
        .map(|value| value.finalize(required_bytes, excluded_root))
        .collect())
}

#[cfg(not(windows))]
pub fn list_usb_targets(
    _required_bytes: u64,
    _excluded_root: Option<&Path>,
) -> Result<Vec<TargetDiskInfo>> {
    Ok(Vec::new())
}

#[cfg(any(windows, test))]
fn volume_root(path: &Path) -> Option<String> {
    let value = path.to_string_lossy();
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        Some(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(filesystem: &str) -> TargetDiskInfo {
        TargetDiskInfo {
            root: "U:\\".into(),
            label: "TEST".into(),
            filesystem: filesystem.into(),
            total_bytes: 100,
            free_bytes: 80,
            disk_number: 4,
            model: "USB".into(),
            serial: "SERIAL".into(),
            bus_type: "USB".into(),
            read_only: false,
            system_disk: false,
            boot_partition: false,
            eligible: false,
            reason: None,
        }
    }

    #[test]
    fn accepts_ntfs_and_rejects_fat32_and_source_volume() {
        assert!(disk("NTFS").finalize(20, None).eligible);
        assert!(!disk("FAT32").finalize(20, None).eligible);
        assert!(
            !disk("exFAT")
                .finalize(20, Some(Path::new("U:\\package.exe")))
                .eligible
        );
    }

    #[test]
    fn rejects_non_usb_system_read_only_and_small_targets() {
        let mut non_usb = disk("NTFS");
        non_usb.bus_type = "SATA".into();
        assert!(!non_usb.finalize(20, None).eligible);

        let mut system = disk("NTFS");
        system.system_disk = true;
        assert!(!system.finalize(20, None).eligible);

        let mut read_only = disk("exFAT");
        read_only.read_only = true;
        assert!(!read_only.finalize(20, None).eligible);

        assert!(!disk("NTFS").finalize(81, None).eligible);
    }
}
