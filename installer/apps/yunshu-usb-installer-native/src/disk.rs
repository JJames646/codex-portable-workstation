use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::{OsStr, c_void};
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_WRITE_PROTECT, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    BusTypeUsb, CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW,
    GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING, PARTITION_SYSTEM_GUID,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DRIVE_LAYOUT_INFORMATION_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_DISK_IS_WRITABLE,
    IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX,
    PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_DEVICE_ID_DESCRIPTOR, STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY,
    StorageDeviceIdProperty, StorageDeviceProperty,
};
use windows_sys::Win32::System::SystemServices::FILE_READ_ONLY_VOLUME;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTargetDiskInfo {
    pub root: String,
    pub label: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub disk_number: u32,
    pub partition_number: u32,
    pub model: String,
    pub serial: String,
    pub stable_device_id_hash: String,
    pub bus_type: String,
    pub read_only: bool,
    pub system_disk: bool,
    pub boot_disk: bool,
    pub installer_disk: bool,
    pub volume_count: u32,
    pub layout_verified: bool,
    pub install_eligible: bool,
    pub format_eligible: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskIdentity {
    pub disk_number: u32,
    pub partition_number: u32,
    pub model: String,
    pub serial: String,
    pub stable_device_id_hash: String,
    pub total_bytes: u64,
    pub volume_count: u32,
}

impl NativeTargetDiskInfo {
    pub fn identity(&self) -> DiskIdentity {
        DiskIdentity {
            disk_number: self.disk_number,
            partition_number: self.partition_number,
            model: self.model.clone(),
            serial: self.serial.clone(),
            stable_device_id_hash: self.stable_device_id_hash.clone(),
            total_bytes: self.total_bytes,
            volume_count: self.volume_count,
        }
    }

    fn finalize(mut self, required_bytes: u64) -> Self {
        let filesystem = self.filesystem.to_ascii_uppercase();
        let base_reason = if !self.bus_type.eq_ignore_ascii_case("USB") {
            Some("只允许选择USB移动磁盘".to_owned())
        } else if self.system_disk {
            Some("Windows系统盘不可写入".to_owned())
        } else if !self.layout_verified {
            Some("无法确认USB分区布局，禁止写入或格式化".to_owned())
        } else if self.installer_disk {
            Some("安装器所在物理磁盘不能作为目标".to_owned())
        } else if self.read_only {
            Some("目标磁盘处于只读状态".to_owned())
        } else {
            None
        };

        self.install_eligible = base_reason.is_none()
            && matches!(filesystem.as_str(), "NTFS" | "EXFAT")
            && !self.stable_device_id_hash.is_empty()
            && self.free_bytes >= required_bytes;
        self.format_eligible = base_reason.is_none()
            && filesystem == "FAT32"
            && self.layout_verified
            && self.volume_count == 1
            && !self.stable_device_id_hash.is_empty()
            && self.total_bytes >= required_bytes;
        self.reason = if let Some(reason) = base_reason {
            Some(reason)
        } else if filesystem == "FAT32" {
            if self.volume_count != 1 {
                Some("多卷USB不允许格式化".into())
            } else if self.stable_device_id_hash.is_empty() {
                Some("无法获取USB稳定设备身份，禁止格式化".into())
            } else if self.total_bytes < required_bytes {
                Some("目标磁盘容量不足".into())
            } else if self.boot_disk {
                Some("此U盘为FAT32且带启动标记，格式化为exFAT后原数据和启动功能都会消失".into())
            } else {
                Some("此U盘为FAT32，需要格式化为exFAT后才能安装".into())
            }
        } else if !matches!(filesystem.as_str(), "NTFS" | "EXFAT") {
            Some("仅支持NTFS、exFAT，或将单卷FAT32格式化为exFAT".into())
        } else if self.stable_device_id_hash.is_empty() {
            Some("无法获取USB稳定设备身份，禁止写入".into())
        } else if self.free_bytes < required_bytes {
            Some("目标磁盘剩余空间不足".into())
        } else {
            None
        };
        self
    }
}

pub fn list_installer_targets(
    required_bytes: u64,
    installer_path: &Path,
) -> Result<Vec<NativeTargetDiskInfo>> {
    let system_root = std::env::var_os("SystemRoot").context("无法识别Windows系统目录")?;
    let system_disk = disk_number_for_path(Path::new(&system_root))
        .context("无法确认Windows系统所在物理磁盘，已停止USB扫描")?;
    let installer_disk = disk_number_for_path(installer_path)
        .context("无法确认安装器所在物理磁盘，已停止USB扫描")?;
    let roots = logical_roots();
    let mut probes = Vec::new();
    let mut visible_counts = HashMap::<u32, u32>::new();
    for root in roots {
        if let Ok(probe) = probe_volume(&root) {
            *visible_counts.entry(probe.disk_number).or_default() += 1;
            probes.push(probe);
        }
    }

    let mut targets = Vec::new();
    for probe in probes {
        if probe.bus_type != BusTypeUsb {
            continue;
        }
        let visible_count = *visible_counts.get(&probe.disk_number).unwrap_or(&1);
        let (layout_verified, volume_count, boot_disk) =
            match physical_disk_layout(probe.disk_number) {
                Ok(layout) => (
                    true,
                    layout.partition_count.max(visible_count),
                    layout.has_boot_partition,
                ),
                Err(_) => (false, visible_count, false),
            };
        targets.push(
            NativeTargetDiskInfo {
                root: probe.root,
                label: probe.label,
                filesystem: probe.filesystem,
                total_bytes: probe.total_bytes,
                free_bytes: probe.free_bytes,
                disk_number: probe.disk_number,
                partition_number: probe.partition_number,
                model: probe.model,
                serial: probe.serial,
                stable_device_id_hash: probe.stable_device_id_hash,
                bus_type: "USB".into(),
                read_only: probe.read_only,
                system_disk: system_disk == probe.disk_number,
                boot_disk,
                installer_disk: installer_disk == probe.disk_number,
                volume_count,
                layout_verified,
                install_eligible: false,
                format_eligible: false,
                reason: None,
            }
            .finalize(required_bytes),
        );
    }
    targets.sort_by(|left, right| left.root.cmp(&right.root));
    Ok(targets)
}

pub fn revalidate_target<'a>(
    expected: &DiskIdentity,
    values: &'a [NativeTargetDiskInfo],
) -> Result<&'a NativeTargetDiskInfo> {
    let mut matches = values
        .iter()
        .filter(|value| identity_matches(expected, &value.identity()));
    let value = matches
        .next()
        .context("原目标USB已消失或物理身份发生变化")?;
    if matches.next().is_some() {
        bail!("检测到多个相同目标，拒绝继续写入");
    }
    Ok(value)
}

fn identity_matches(expected: &DiskIdentity, current: &DiskIdentity) -> bool {
    let size_delta = expected.total_bytes.abs_diff(current.total_bytes);
    let size_tolerance = (expected.total_bytes / 100).max(64 * 1024 * 1024);
    expected.disk_number == current.disk_number
        && expected.partition_number == current.partition_number
        && expected.model.eq_ignore_ascii_case(&current.model)
        && !expected.stable_device_id_hash.is_empty()
        && expected
            .stable_device_id_hash
            .eq_ignore_ascii_case(&current.stable_device_id_hash)
        && size_delta <= size_tolerance
        && expected.volume_count == current.volume_count
}

struct VolumeProbe {
    root: String,
    label: String,
    filesystem: String,
    total_bytes: u64,
    free_bytes: u64,
    disk_number: u32,
    partition_number: u32,
    model: String,
    serial: String,
    stable_device_id_hash: String,
    bus_type: i32,
    read_only: bool,
}

fn logical_roots() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26)
        .filter(|index| mask & (1 << index) != 0)
        .map(|index| format!("{}:\\", (b'A' + index as u8) as char))
        .collect()
}

fn probe_volume(root: &str) -> Result<VolumeProbe> {
    let handle = open_volume(root)?;
    let number = device_number(handle.0)?;
    let descriptor = device_descriptor(handle.0)?;
    let stable_device_id_hash = device_identifier_hash(handle.0, &descriptor)?;
    let (label, filesystem, volume_read_only) = volume_information(root)?;
    let (free_bytes, total_bytes) = disk_space(root)?;
    let writable = device_is_writable(handle.0)?;
    Ok(VolumeProbe {
        root: root.to_owned(),
        label,
        filesystem,
        total_bytes,
        free_bytes,
        disk_number: number.DeviceNumber,
        partition_number: number.PartitionNumber,
        model: descriptor.model,
        serial: descriptor.serial,
        stable_device_id_hash,
        bus_type: descriptor.bus_type,
        read_only: volume_read_only || !writable,
    })
}

fn disk_number_for_path(path: &Path) -> Result<u32> {
    let root = volume_root(path).context("路径不包含Windows盘符")?;
    let handle = open_volume(&root)?;
    Ok(device_number(handle.0)?.DeviceNumber)
}

fn volume_root(path: &Path) -> Option<String> {
    let value = path.to_string_lossy();
    let bytes = value.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':')
        .then(|| format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()))
}

fn open_volume(root: &str) -> Result<OwnedHandle> {
    let letter = root
        .as_bytes()
        .first()
        .copied()
        .filter(|value| value.is_ascii_alphabetic())
        .context("目标盘符无效")? as char;
    let path = format!(r"\\.\{}:", letter.to_ascii_uppercase());
    open_device(&path)
}

fn device_identifier_hash(handle: HANDLE, descriptor: &DeviceDescriptor) -> Result<String> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceIdProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const c_void,
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    let mut hasher = Sha256::new();
    if ok != 0 && returned >= size_of::<STORAGE_DEVICE_ID_DESCRIPTOR>() as u32 {
        let descriptor_header = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr() as *const STORAGE_DEVICE_ID_DESCRIPTOR)
        };
        let size = usize::try_from(descriptor_header.Size)
            .unwrap_or(0)
            .min(returned as usize)
            .min(buffer.len());
        if descriptor_header.NumberOfIdentifiers > 0 && size > 12 {
            hasher.update(&buffer[12..size]);
            return Ok(hex::encode(hasher.finalize()));
        }
    }
    if descriptor.serial.trim().is_empty() {
        return Ok(String::new());
    }
    hasher.update(descriptor.model.to_ascii_lowercase().as_bytes());
    hasher.update([0]);
    hasher.update(descriptor.serial.to_ascii_lowercase().as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn open_physical(disk_number: u32) -> Result<OwnedHandle> {
    open_device(&format!(r"\\.\PhysicalDrive{disk_number}"))
}

fn open_device(path: &str) -> Result<OwnedHandle> {
    let wide = wide(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("无法打开磁盘设备: {path}"));
    }
    Ok(OwnedHandle(handle))
}

fn device_number(handle: HANDLE) -> Result<STORAGE_DEVICE_NUMBER> {
    let mut value = STORAGE_DEVICE_NUMBER::default();
    ioctl_out(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER, &mut value)?;
    Ok(value)
}

struct DeviceDescriptor {
    model: String,
    serial: String,
    bus_type: i32,
}

fn device_descriptor(handle: HANDLE) -> Result<DeviceDescriptor> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buffer = vec![0_u8; 4096];
    let mut returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const c_void,
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("无法读取USB设备描述符");
    }
    if returned < size_of::<STORAGE_DEVICE_DESCRIPTOR>() as u32 {
        bail!("USB设备描述符被截断");
    }
    let descriptor =
        unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    let vendor = descriptor_string(&buffer, descriptor.VendorIdOffset);
    let product = descriptor_string(&buffer, descriptor.ProductIdOffset);
    let model = format!("{vendor} {product}").trim().to_owned();
    Ok(DeviceDescriptor {
        model: if model.is_empty() {
            "USB 移动磁盘".into()
        } else {
            model
        },
        serial: descriptor_string(&buffer, descriptor.SerialNumberOffset),
        bus_type: descriptor.BusType,
    })
}

fn descriptor_string(buffer: &[u8], offset: u32) -> String {
    if offset == 0 || offset as usize >= buffer.len() {
        return String::new();
    }
    let bytes = &buffer[offset as usize..];
    let length = bytes.iter().position(|value| *value == 0).unwrap_or(0);
    String::from_utf8_lossy(&bytes[..length]).trim().to_owned()
}

fn volume_information(root: &str) -> Result<(String, String, bool)> {
    let root = wide(root);
    let mut label = [0_u16; 260];
    let mut filesystem = [0_u16; 64];
    let mut flags = 0_u32;
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            null_mut(),
            null_mut(),
            &mut flags,
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("无法读取U盘卷信息");
    }
    Ok((
        from_wide_z(&label),
        from_wide_z(&filesystem),
        flags & FILE_READ_ONLY_VOLUME != 0,
    ))
}

fn disk_space(root: &str) -> Result<(u64, u64)> {
    let root = wide(root);
    let mut available = 0_u64;
    let mut total = 0_u64;
    let mut free = 0_u64;
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, &mut free) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("无法读取U盘容量");
    }
    Ok((available, total))
}

struct PhysicalDiskLayout {
    partition_count: u32,
    has_boot_partition: bool,
}

fn physical_disk_layout(disk_number: u32) -> Result<PhysicalDiskLayout> {
    let handle = open_physical(disk_number)?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            null(),
            0,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("无法读取USB分区布局");
    }
    if returned < size_of::<DRIVE_LAYOUT_INFORMATION_EX>() as u32 {
        bail!("USB分区布局被截断");
    }
    let layout =
        unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const DRIVE_LAYOUT_INFORMATION_EX) };
    let entry_offset = offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
    let entries_bytes = usize::try_from(layout.PartitionCount)
        .context("USB分区数量无效")?
        .checked_mul(size_of::<PARTITION_INFORMATION_EX>())
        .context("USB分区布局范围溢出")?;
    let required = entry_offset
        .checked_add(entries_bytes)
        .context("USB分区布局范围溢出")?;
    if (returned as usize) < required || required > buffer.len() {
        bail!("USB分区布局条目被截断");
    }
    let mut has_boot_partition = false;
    for index in 0..layout.PartitionCount as usize {
        let offset = entry_offset + index * size_of::<PARTITION_INFORMATION_EX>();
        let partition = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const PARTITION_INFORMATION_EX)
        };
        if partition_has_boot_marker(&partition) {
            has_boot_partition = true;
            break;
        }
    }
    Ok(PhysicalDiskLayout {
        partition_count: layout.PartitionCount,
        has_boot_partition,
    })
}

fn partition_has_boot_marker(partition: &PARTITION_INFORMATION_EX) -> bool {
    if partition.PartitionLength <= 0 {
        return false;
    }
    match partition.PartitionStyle {
        PARTITION_STYLE_MBR => unsafe { partition.Anonymous.Mbr.BootIndicator },
        PARTITION_STYLE_GPT => unsafe {
            let partition_type = partition.Anonymous.Gpt.PartitionType;
            partition_type.data1 == PARTITION_SYSTEM_GUID.data1
                && partition_type.data2 == PARTITION_SYSTEM_GUID.data2
                && partition_type.data3 == PARTITION_SYSTEM_GUID.data3
                && partition_type.data4 == PARTITION_SYSTEM_GUID.data4
        },
        _ => false,
    }
}

fn device_is_writable(handle: HANDLE) -> Result<bool> {
    let mut returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_IS_WRITABLE,
            null(),
            0,
            null_mut(),
            0,
            &mut returned,
            null_mut(),
        )
    };
    interpret_writable_query(ok != 0, unsafe { GetLastError() })
}

fn interpret_writable_query(succeeded: bool, error: u32) -> Result<bool> {
    if succeeded {
        return Ok(true);
    }
    if error == ERROR_WRITE_PROTECT {
        return Ok(false);
    }
    Err(std::io::Error::from_raw_os_error(error as i32)).context("无法确认USB物理磁盘可写状态")
}

fn ioctl_out<T>(handle: HANDLE, code: u32, value: &mut T) -> Result<()> {
    let mut returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            code,
            null(),
            0,
            value as *mut T as *mut c_void,
            size_of::<T>() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("Windows磁盘查询失败");
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

fn from_wide_z(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|item| *item == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(filesystem: &str) -> NativeTargetDiskInfo {
        NativeTargetDiskInfo {
            root: "U:\\".into(),
            label: "TEST".into(),
            filesystem: filesystem.into(),
            total_bytes: 100,
            free_bytes: 80,
            disk_number: 7,
            partition_number: 1,
            model: "USB".into(),
            serial: "SERIAL".into(),
            stable_device_id_hash: "stable-id".into(),
            bus_type: "USB".into(),
            read_only: false,
            system_disk: false,
            boot_disk: false,
            installer_disk: false,
            volume_count: 1,
            layout_verified: true,
            install_eligible: false,
            format_eligible: false,
            reason: None,
        }
    }

    #[test]
    fn separates_direct_install_from_fat32_formatting() {
        let ntfs = target("NTFS").finalize(20);
        assert!(ntfs.install_eligible);
        assert!(!ntfs.format_eligible);

        let fat32 = target("FAT32").finalize(20);
        assert!(!fat32.install_eligible);
        assert!(fat32.format_eligible);
    }

    #[test]
    fn blocks_multi_volume_and_protected_disks() {
        let mut multi = target("FAT32");
        multi.volume_count = 2;
        assert!(!multi.finalize(20).format_eligible);

        let mut installer = target("exFAT");
        installer.installer_disk = true;
        assert!(!installer.finalize(20).install_eligible);

        let mut system = target("exFAT");
        system.system_disk = true;
        assert!(!system.finalize(20).install_eligible);

        let mut read_only = target("FAT32");
        read_only.read_only = true;
        assert!(!read_only.finalize(20).format_eligible);

        let mut unknown_layout = target("NTFS");
        unknown_layout.layout_verified = false;
        let unknown_layout = unknown_layout.finalize(20);
        assert!(!unknown_layout.install_eligible);
        assert!(!unknown_layout.format_eligible);
        assert_eq!(
            unknown_layout.reason.as_deref(),
            Some("无法确认USB分区布局，禁止写入或格式化")
        );

        let mut unknown_format_layout = target("FAT32");
        unknown_format_layout.layout_verified = false;
        assert!(!unknown_format_layout.finalize(20).format_eligible);
    }

    #[test]
    fn boot_markers_do_not_masquerade_as_the_current_windows_system_disk() {
        let mut exfat = target("exFAT");
        exfat.boot_disk = true;
        let exfat = exfat.finalize(20);
        assert!(exfat.install_eligible);
        assert!(!exfat.format_eligible);

        let mut fat32 = target("FAT32");
        fat32.boot_disk = true;
        let fat32 = fat32.finalize(20);
        assert!(!fat32.install_eligible);
        assert!(fat32.format_eligible);
        assert_eq!(
            fat32.reason.as_deref(),
            Some("此U盘为FAT32且带启动标记，格式化为exFAT后原数据和启动功能都会消失")
        );

        let mut system = target("exFAT");
        system.system_disk = true;
        system.boot_disk = true;
        assert!(!system.finalize(20).install_eligible);
    }

    #[test]
    fn detects_mbr_active_and_gpt_efi_boot_partitions() {
        use windows_sys::Win32::System::Ioctl::{
            PARTITION_INFORMATION_EX_0, PARTITION_INFORMATION_GPT, PARTITION_INFORMATION_MBR,
        };

        let mbr = PARTITION_INFORMATION_EX {
            PartitionStyle: PARTITION_STYLE_MBR,
            PartitionLength: 1,
            Anonymous: PARTITION_INFORMATION_EX_0 {
                Mbr: PARTITION_INFORMATION_MBR {
                    BootIndicator: true,
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        assert!(partition_has_boot_marker(&mbr));

        let gpt = PARTITION_INFORMATION_EX {
            PartitionStyle: PARTITION_STYLE_GPT,
            PartitionLength: 1,
            Anonymous: PARTITION_INFORMATION_EX_0 {
                Gpt: PARTITION_INFORMATION_GPT {
                    PartitionType: PARTITION_SYSTEM_GUID,
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        assert!(partition_has_boot_marker(&gpt));

        let ordinary_gpt = PARTITION_INFORMATION_EX {
            PartitionStyle: PARTITION_STYLE_GPT,
            PartitionLength: 1,
            ..Default::default()
        };
        assert!(!partition_has_boot_marker(&ordinary_gpt));
    }

    #[test]
    fn writable_query_fails_closed_on_unknown_errors() {
        assert!(interpret_writable_query(true, 0).unwrap());
        assert!(!interpret_writable_query(false, ERROR_WRITE_PROTECT).unwrap());
        assert!(interpret_writable_query(false, 5).is_err());
    }

    #[test]
    fn revalidation_rejects_changed_identity() {
        let value = target("NTFS").finalize(20);
        let mut changed = value.clone();
        changed.stable_device_id_hash = "other-stable-id".into();
        assert!(revalidate_target(&value.identity(), &[changed]).is_err());
    }
}
