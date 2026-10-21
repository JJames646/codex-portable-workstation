use crate::manifest::{
    FileScope, ReleaseDirectory, ReleaseFile, ReleaseManifest, manifest_path_to_path, sha256_file,
};
use crate::package::{VerifiedInstaller, open_payload_archive};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const LAUNCH_SHORTCUT_NAME: &str = "立即启动.lnk";
const WORKSTATION_LAUNCHER_NAME: &str = "云枢AI便携工作站.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WritePhase {
    Preflight,
    Staging,
    Writing,
    Verifying,
    Committing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    pub target_root: PathBuf,
    pub operation_id: String,
}

impl InstallRequest {
    pub fn new(target_root: PathBuf) -> Self {
        Self {
            target_root,
            operation_id: Uuid::new_v4().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOperation {
    pub operation_id: String,
    pub target_root: String,
    pub staging_directory: String,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteJobStatus {
    pub operation: WriteOperation,
    pub phase: WritePhase,
    pub processed_files: u64,
    pub total_files: u64,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub message: String,
    pub error: Option<String>,
}

pub fn install_from_master<P, C>(
    master_root: &Path,
    manifest: &ReleaseManifest,
    request: &InstallRequest,
    mut progress: P,
    cancelled: C,
) -> Result<WriteJobStatus>
where
    P: FnMut(&WriteJobStatus),
    C: Fn() -> bool,
{
    run_install(
        manifest,
        request,
        &mut progress,
        &cancelled,
        &mut || Ok(()),
        |item, destination| copy_master_file(master_root, item, destination),
    )
}

pub fn install_from_verified_installer<P, C>(
    installer: &VerifiedInstaller,
    request: &InstallRequest,
    mut progress: P,
    cancelled: C,
) -> Result<WriteJobStatus>
where
    P: FnMut(&WriteJobStatus),
    C: Fn() -> bool,
{
    let mut archive = open_payload_archive(installer)?;
    run_install(
        &installer.manifest,
        request,
        &mut progress,
        &cancelled,
        &mut || Ok(()),
        |item, destination| {
            let mut source = archive
                .by_name(&item.source)
                .with_context(|| format!("安装包缺少文件: {}", item.source))?;
            write_stream(&mut source, destination, item.bytes)
        },
    )
}

pub fn install_from_verified_installer_checked<P, C, G>(
    installer: &VerifiedInstaller,
    request: &InstallRequest,
    mut progress: P,
    cancelled: C,
    mut target_guard: G,
) -> Result<WriteJobStatus>
where
    P: FnMut(&WriteJobStatus),
    C: Fn() -> bool,
    G: FnMut() -> Result<()>,
{
    let mut archive = open_payload_archive(installer)?;
    run_install(
        &installer.manifest,
        request,
        &mut progress,
        &cancelled,
        &mut target_guard,
        |item, destination| {
            let mut source = archive
                .by_name(&item.source)
                .with_context(|| format!("安装包缺少文件: {}", item.source))?;
            write_stream(&mut source, destination, item.bytes)
        },
    )
}

fn run_install<F, P, C, G>(
    manifest: &ReleaseManifest,
    request: &InstallRequest,
    progress: &mut P,
    cancelled: &C,
    target_guard: &mut G,
    mut materialize: F,
) -> Result<WriteJobStatus>
where
    F: FnMut(&ReleaseFile, &Path) -> Result<()>,
    P: FnMut(&WriteJobStatus),
    C: Fn() -> bool,
    G: FnMut() -> Result<()>,
{
    validate_operation_id(&request.operation_id)?;
    target_guard()?;
    let target = request
        .target_root
        .canonicalize()
        .with_context(|| format!("无法读取目标盘: {}", request.target_root.display()))?;
    if !target.is_dir() {
        bail!("目标路径不是磁盘根目录");
    }
    let staging = target.join(format!(".yunshu-write-{}.partial", request.operation_id));
    let operation = WriteOperation {
        operation_id: request.operation_id.clone(),
        target_root: target.to_string_lossy().into_owned(),
        staging_directory: staging.to_string_lossy().into_owned(),
        started_at: Utc::now().to_rfc3339(),
    };
    let mut status = WriteJobStatus {
        operation,
        phase: WritePhase::Preflight,
        processed_files: 0,
        total_files: manifest.file_count,
        processed_bytes: 0,
        total_bytes: manifest.expanded_bytes,
        message: "正在检查目标U盘".into(),
        error: None,
    };
    progress(&status);
    preflight_target(&target, &staging, manifest)?;
    if cancelled() {
        status.phase = WritePhase::Cancelled;
        status.message = "写入已取消".into();
        progress(&status);
        return Ok(status);
    }

    status.phase = WritePhase::Staging;
    status.message = "正在创建安全暂存区".into();
    progress(&status);
    fs::create_dir(&staging)?;
    fs::write(
        staging.join("operation-id"),
        request.operation_id.as_bytes(),
    )?;
    let product_directory = manifest_path_to_path(&manifest.product_directory)?;
    let product_stage = staging.join("product").join(&product_directory);
    let root_stage = staging.join("root");
    fs::create_dir_all(&product_stage)?;
    fs::create_dir_all(&root_stage)?;
    for item in &manifest.directories {
        let destination = staged_directory_path(&product_stage, &root_stage, item)?;
        fs::create_dir_all(&destination)
            .with_context(|| format!("无法创建母盘目录: {}", item.relative_path))?;
    }

    let install_result = (|| -> Result<()> {
        status.phase = WritePhase::Writing;
        status.message = "正在写入文件".into();
        progress(&status);
        for item in &manifest.files {
            if cancelled() {
                bail!("__CANCELLED__");
            }
            let destination = staged_path(&product_stage, &root_stage, item)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            materialize(item, &destination)?;
            status.processed_files += 1;
            status.processed_bytes += item.bytes;
            progress(&status);
        }

        status.phase = WritePhase::Verifying;
        status.message = "正在逐文件校验SHA-256".into();
        progress(&status);
        for item in &manifest.files {
            let path = staged_path(&product_stage, &root_stage, item)?;
            verify_staged_file(item, &path)?;
        }

        if cancelled() {
            bail!("__CANCELLED__");
        }
        status.phase = WritePhase::Committing;
        status.message = "正在提交正式目录".into();
        progress(&status);
        target_guard()?;
        commit_staging(&target, &staging, manifest)?;
        Ok(())
    })();

    match install_result {
        Ok(()) => {
            status.phase = WritePhase::Completed;
            status.message = "U盘制作完成".into();
            progress(&status);
            Ok(status)
        }
        Err(error) if error.to_string() == "__CANCELLED__" => {
            cleanup_owned_staging(&staging, &request.operation_id)?;
            status.phase = WritePhase::Cancelled;
            status.message = "写入已取消，暂存区已清理".into();
            progress(&status);
            Ok(status)
        }
        Err(error) => {
            status.phase = WritePhase::Failed;
            status.message = "写入失败，未发布半成品目录".into();
            status.error = Some(error.to_string());
            progress(&status);
            Err(error)
        }
    }
}

fn preflight_target(target: &Path, staging: &Path, manifest: &ReleaseManifest) -> Result<()> {
    if staging.exists() {
        bail!("存在同operation-id的未完成暂存目录");
    }
    let product_directory = manifest_path_to_path(&manifest.product_directory)?;
    reject_existing_reparse_components(target, &product_directory)?;
    let product = target.join(&product_directory);
    if product.exists() {
        bail!("目标盘已存在同名工作站目录，拒绝覆盖");
    }
    launch_shortcut_target(manifest)?;
    let shortcut = Path::new(LAUNCH_SHORTCUT_NAME);
    if manifest.files.iter().any(|item| {
        item.scope == FileScope::UsbRoot
            && item
                .relative_path
                .eq_ignore_ascii_case(LAUNCH_SHORTCUT_NAME)
    }) {
        bail!("母盘交付资料已包含“立即启动”，与写入器固定入口冲突");
    }
    reject_existing_reparse_components(target, shortcut)?;
    if target.join(shortcut).exists() {
        bail!("U盘根目录文件发生冲突: {LAUNCH_SHORTCUT_NAME}");
    }
    for item in manifest
        .files
        .iter()
        .filter(|item| item.scope == FileScope::UsbRoot)
    {
        let relative = manifest_path_to_path(&item.relative_path)?;
        reject_existing_reparse_components(target, &relative)?;
        let destination = target.join(relative);
        if destination.exists() {
            bail!("U盘根目录文件发生冲突: {}", item.relative_path);
        }
    }
    for item in manifest
        .directories
        .iter()
        .filter(|item| item.scope == FileScope::UsbRoot)
    {
        let relative = manifest_path_to_path(&item.relative_path)?;
        if relative
            .to_string_lossy()
            .eq_ignore_ascii_case(LAUNCH_SHORTCUT_NAME)
        {
            bail!("母盘目录与写入器固定的“立即启动”入口冲突");
        }
        reject_existing_reparse_components(target, &relative)?;
        let destination = target.join(&relative);
        if destination.exists() && !fs::symlink_metadata(&destination)?.is_dir() {
            bail!("U盘根目录映射的目标不是目录: {}", item.relative_path);
        }
    }
    Ok(())
}

fn staged_path(product: &Path, root: &Path, item: &ReleaseFile) -> Result<PathBuf> {
    let relative = manifest_path_to_path(&item.relative_path)?;
    Ok(match item.scope {
        FileScope::Product => product.join(relative),
        FileScope::UsbRoot => root.join(relative),
    })
}

fn staged_directory_path(product: &Path, root: &Path, item: &ReleaseDirectory) -> Result<PathBuf> {
    let relative = manifest_path_to_path(&item.relative_path)?;
    Ok(match item.scope {
        FileScope::Product => product.join(relative),
        FileScope::UsbRoot => root.join(relative),
    })
}

fn copy_master_file(master: &Path, item: &ReleaseFile, destination: &Path) -> Result<()> {
    let source = master.join(manifest_path_to_path(&item.source)?);
    let mut reader = BufReader::new(File::open(&source)?);
    write_stream(&mut reader, destination, item.bytes)
}

fn write_stream(reader: &mut impl Read, destination: &Path, expected_bytes: u64) -> Result<()> {
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let mut writer = BufWriter::new(file);
    let copied = std::io::copy(reader, &mut writer)?;
    if copied != expected_bytes {
        bail!("写入长度不匹配: {}", destination.display());
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn verify_staged_file(item: &ReleaseFile, path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != item.bytes {
        bail!("暂存文件长度不匹配: {}", item.relative_path);
    }
    if !sha256_file(path)?.eq_ignore_ascii_case(&item.sha256) {
        bail!("暂存文件SHA-256不匹配: {}", item.relative_path);
    }
    Ok(())
}

fn commit_staging(target: &Path, staging: &Path, manifest: &ReleaseManifest) -> Result<()> {
    let product_directory = manifest_path_to_path(&manifest.product_directory)?;
    let staged_product = staging.join("product").join(&product_directory);
    ensure_safe_parent_directories(target, &product_directory)?;
    reject_existing_reparse_components(target, &product_directory)?;
    let final_product = target.join(&product_directory);
    if final_product.exists() {
        bail!("目标盘已存在同名工作站目录，拒绝覆盖");
    }
    fs::rename(&staged_product, &final_product)?;
    let mut committed_roots = Vec::new();
    let mut committed_directories = Vec::new();
    let result = (|| -> Result<()> {
        for item in manifest
            .directories
            .iter()
            .filter(|item| item.scope == FileScope::UsbRoot)
        {
            let relative = manifest_path_to_path(&item.relative_path)?;
            ensure_target_directory_tree(&target, &relative, &mut committed_directories)?;
        }
        for item in manifest
            .files
            .iter()
            .filter(|item| item.scope == FileScope::UsbRoot)
        {
            let relative = manifest_path_to_path(&item.relative_path)?;
            let source = staging.join("root").join(&relative);
            ensure_safe_parent_directories(target, &relative)?;
            reject_existing_reparse_components(target, &relative)?;
            let destination = target.join(&relative);
            if destination.exists() {
                bail!("U盘根目录文件发生冲突: {}", item.relative_path);
            }
            fs::rename(source, &destination)?;
            committed_roots.push(destination);
        }
        #[cfg(windows)]
        {
            let shortcut_relative = Path::new(LAUNCH_SHORTCUT_NAME);
            let staged_shortcut = staging.join("root").join(shortcut_relative);
            let final_shortcut = target.join(shortcut_relative);
            let launcher = target.join(launch_shortcut_target(manifest)?);
            let working_directory = launcher
                .parent()
                .context("无法确定工作站快捷方式的工作目录")?;
            create_launch_shortcut(
                &staged_shortcut,
                &final_shortcut,
                &launcher,
                working_directory,
            )?;
            fs::rename(&staged_shortcut, &final_shortcut)?;
            committed_roots.push(final_shortcut);
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in committed_roots.into_iter().rev() {
            let _ = fs::remove_file(path);
        }
        for path in committed_directories.into_iter().rev() {
            let _ = fs::remove_dir(path);
        }
        let _ = fs::rename(&final_product, &staged_product);
        return Err(error);
    }
    fs::remove_dir_all(staging)?;
    Ok(())
}

fn launch_shortcut_target(manifest: &ReleaseManifest) -> Result<PathBuf> {
    let launcher = Path::new(WORKSTATION_LAUNCHER_NAME);
    let exists = manifest.files.iter().any(|item| {
        item.scope == FileScope::Product
            && manifest_path_to_path(&item.relative_path).is_ok_and(|relative| relative == launcher)
    });
    if !exists {
        bail!("母盘缺少工作站主程序: {WORKSTATION_LAUNCHER_NAME}");
    }
    Ok(manifest_path_to_path(&manifest.product_directory)?.join(launcher))
}

#[cfg(windows)]
fn create_launch_shortcut(
    staged_shortcut: &Path,
    final_shortcut: &Path,
    launcher: &Path,
    working_directory: &Path,
) -> Result<()> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};

    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .context("无法初始化Windows快捷方式服务")?;
        let _apartment = ComApartment;
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .context("无法创建Windows快捷方式")?;
        let launcher = shell_path_utf16(launcher);
        let working_directory = shell_path_utf16(working_directory);
        let final_shortcut = shell_path_utf16(final_shortcut);
        let staged_shortcut = shell_path_utf16(staged_shortcut);
        let description = "立即启动云枢AI便携工作站"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        shell_link
            .SetPath(PCWSTR(launcher.as_ptr()))
            .context("无法设置工作站快捷方式目标")?;
        shell_link
            .SetWorkingDirectory(PCWSTR(working_directory.as_ptr()))
            .context("无法设置工作站快捷方式的工作目录")?;
        shell_link
            .SetDescription(PCWSTR(description.as_ptr()))
            .context("无法设置工作站快捷方式说明")?;
        shell_link
            .SetRelativePath(PCWSTR(final_shortcut.as_ptr()), 0)
            .context("无法设置可移动快捷方式路径")?;
        let persist: IPersistFile = shell_link.cast().context("无法保存Windows快捷方式")?;
        persist
            .Save(PCWSTR(staged_shortcut.as_ptr()), true)
            .context("无法写入“立即启动”快捷方式")?;
    }
    if fs::metadata(staged_shortcut)?.len() == 0 {
        bail!("“立即启动”快捷方式为空文件");
    }
    Ok(())
}

#[cfg(windows)]
fn shell_path_utf16(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    const VERBATIM_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const VERBATIM_UNC_PREFIX: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let original = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut compatible = if let Some(remainder) = original.strip_prefix(VERBATIM_UNC_PREFIX) {
        [b'\\' as u16, b'\\' as u16]
            .into_iter()
            .chain(remainder.iter().copied())
            .collect::<Vec<_>>()
    } else if let Some(remainder) = original.strip_prefix(VERBATIM_PREFIX) {
        remainder.to_vec()
    } else {
        original
    };
    compatible.push(0);
    compatible
}

fn reject_existing_reparse_components(root: &Path, relative: &Path) -> Result<()> {
    ensure_safe_existing_directory(root, root)?;
    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            bail!("目标相对路径包含越界成分: {}", relative.display());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata_is_reparse(&metadata) {
                    bail!(
                        "目标盘既有路径包含符号链接或重解析点: {}",
                        relative.display()
                    );
                }
                if components.peek().is_some() && !metadata.is_dir() {
                    bail!("目标盘既有父路径不是目录: {}", relative.display());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法检查目标盘既有路径: {}", current.display()));
            }
        }
    }
    Ok(())
}

fn ensure_safe_parent_directories(root: &Path, relative: &Path) -> Result<()> {
    ensure_safe_existing_directory(root, root)?;
    let mut current = root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let Component::Normal(name) = component else {
            bail!("目标相对路径包含越界成分: {}", relative.display());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(_) => ensure_safe_existing_directory(&current, relative)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).with_context(|| {
                    format!("无法创建U盘根目录映射的父目录: {}", current.display())
                })?;
                ensure_safe_existing_directory(&current, relative)?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法检查目标盘父路径: {}", current.display()));
            }
        }
    }
    Ok(())
}

fn ensure_target_directory_tree(
    root: &Path,
    relative: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<()> {
    ensure_safe_existing_directory(root, relative)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("目标相对路径包含越界成分: {}", relative.display());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata_is_reparse(&metadata) {
                    bail!(
                        "目标盘既有路径包含符号链接或重解析点: {}",
                        relative.display()
                    );
                }
                if !metadata.is_dir() {
                    bail!("目标盘既有路径不是目录: {}", relative.display());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .with_context(|| format!("无法创建U盘目录: {}", current.display()))?;
                created.push(current.clone());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法检查目标盘目录: {}", current.display()));
            }
        }
    }
    Ok(())
}

fn ensure_safe_existing_directory(path: &Path, display_relative: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("无法读取目标目录属性: {}", path.display()))?;
    if metadata_is_reparse(&metadata) {
        bail!(
            "目标盘既有路径包含符号链接或重解析点: {}",
            display_relative.display()
        );
    }
    if !metadata.is_dir() {
        bail!("目标盘既有父路径不是目录: {}", display_relative.display());
    }
    Ok(())
}

fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn cleanup_owned_staging(staging: &Path, operation_id: &str) -> Result<bool> {
    if !staging.is_dir() {
        return Ok(false);
    }
    let marker = fs::read_to_string(staging.join("operation-id"))?;
    if marker != operation_id {
        bail!("暂存目录operation-id不匹配，拒绝清理");
    }
    fs::remove_dir_all(staging)?;
    Ok(true)
}

fn validate_operation_id(value: &str) -> Result<()> {
    Uuid::parse_str(value).context("operation-id格式无效")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ReleaseDirectory, RootFileMapping, build_release_manifest, sha256_file};
    use crate::{build_single_exe_installer, verify_single_exe_installer};
    use ed25519_dalek::SigningKey;
    use tempfile::tempdir;

    fn manifest(master: &Path) -> ReleaseManifest {
        let product = master.join("app/file.txt");
        let launcher = master.join(WORKSTATION_LAUNCHER_NAME);
        let factory = master.join("Factory/Seeds/Codex/manifest.json");
        let root = master.join("交付资料/写入U盘根目录/使用说明.url");
        ReleaseManifest {
            schema_version: 2,
            product_version: "test".into(),
            build_id: "build".into(),
            generated_at: "now".into(),
            product_directory: "云枢AI便携工作站".into(),
            expanded_bytes: 22,
            file_count: 4,
            factory_file_count: 1626,
            payload_architecture: crate::manifest::PayloadArchitecture::X64,
            host_compatibility: crate::manifest::HostCompatibility {
                supported: vec!["windows-10-x64".into(), "windows-11-x64".into()],
                risk_trial: vec!["windows-11-arm64".into()],
                blocked: vec!["windows-10-arm64".into(), "windows-x86".into()],
            },
            payload_sha256: None,
            files: vec![
                ReleaseFile {
                    source: "app/file.txt".into(),
                    scope: FileScope::Product,
                    relative_path: "app/file.txt".into(),
                    bytes: 7,
                    sha256: sha256_file(&product).unwrap(),
                },
                ReleaseFile {
                    source: WORKSTATION_LAUNCHER_NAME.into(),
                    scope: FileScope::Product,
                    relative_path: WORKSTATION_LAUNCHER_NAME.into(),
                    bytes: 8,
                    sha256: sha256_file(&launcher).unwrap(),
                },
                ReleaseFile {
                    source: "Factory/Seeds/Codex/manifest.json".into(),
                    scope: FileScope::Product,
                    relative_path: "Factory/Seeds/Codex/manifest.json".into(),
                    bytes: 4,
                    sha256: sha256_file(&factory).unwrap(),
                },
                ReleaseFile {
                    source: "交付资料/写入U盘根目录/使用说明.url".into(),
                    scope: FileScope::UsbRoot,
                    relative_path: "使用说明.url".into(),
                    bytes: 3,
                    sha256: sha256_file(&root).unwrap(),
                },
            ],
            root_mappings: vec![RootFileMapping {
                source: "交付资料/写入U盘根目录/使用说明.url".into(),
                destination: "使用说明.url".into(),
            }],
            directories: vec![
                ReleaseDirectory {
                    source: "Factory/Seeds/UI".into(),
                    scope: FileScope::Product,
                    relative_path: "Factory/Seeds/UI".into(),
                },
                ReleaseDirectory {
                    source: "交付资料/写入U盘根目录/教程".into(),
                    scope: FileScope::UsbRoot,
                    relative_path: "教程".into(),
                },
            ],
        }
    }

    fn seed(master: &Path) {
        fs::create_dir_all(master.join("app")).unwrap();
        fs::write(master.join("app/file.txt"), b"payload").unwrap();
        fs::write(master.join(WORKSTATION_LAUNCHER_NAME), b"launcher").unwrap();
        fs::create_dir_all(master.join("Factory/Seeds/Codex")).unwrap();
        fs::write(master.join("Factory/Seeds/Codex/manifest.json"), b"seed").unwrap();
        fs::create_dir_all(master.join("Factory/Seeds/UI")).unwrap();
        fs::create_dir_all(master.join("交付资料/写入U盘根目录")).unwrap();
        fs::write(master.join("交付资料/写入U盘根目录/使用说明.url"), b"url").unwrap();
        fs::create_dir_all(master.join("交付资料/写入U盘根目录/教程")).unwrap();
    }

    #[test]
    fn commits_product_and_root_mapping_without_touching_sentinel() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        seed(master.path());
        fs::write(target.path().join("keep.txt"), b"keep").unwrap();
        let result = install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        )
        .unwrap();
        assert_eq!(result.phase, WritePhase::Completed);
        assert_eq!(fs::read(target.path().join("keep.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(target.path().join("云枢AI便携工作站/app/file.txt")).unwrap(),
            b"payload"
        );
        assert_eq!(
            fs::read(target.path().join("使用说明.url")).unwrap(),
            b"url"
        );
        assert!(
            target
                .path()
                .join("云枢AI便携工作站/Factory/Seeds/UI")
                .is_dir()
        );
        assert!(target.path().join("教程").is_dir());
    }

    #[test]
    fn installs_into_an_arbitrary_nested_directory() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        let nested = target.path().join("资料").join("云枢工作站");
        seed(master.path());
        fs::create_dir_all(&nested).unwrap();

        let result = install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(nested.clone()),
            |_| {},
            || false,
        )
        .unwrap();

        assert_eq!(result.phase, WritePhase::Completed);
        assert!(nested.join("云枢AI便携工作站/app/file.txt").is_file());
        assert!(!target.path().join("云枢AI便携工作站").exists());
    }

    #[test]
    fn verified_installer_preserves_manifest_directories() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        let output = tempdir().unwrap();
        seed(master.path());

        let manifest = build_release_manifest(master.path(), "1.0.0", "build-1", 1626).unwrap();
        assert!(
            manifest
                .directories
                .iter()
                .any(|directory| directory.relative_path == "Factory/Seeds/UI")
        );
        let stub = output.path().join("installer-stub.exe");
        fs::write(&stub, b"stub").unwrap();
        let installer_path = output.path().join("installer.exe");
        let key = SigningKey::from_bytes(&[9_u8; 32]);
        build_single_exe_installer(master.path(), &manifest, &stub, &installer_path, &key).unwrap();
        let verified = verify_single_exe_installer(&installer_path).unwrap();

        let result = install_from_verified_installer(
            &verified,
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        )
        .unwrap();

        assert_eq!(result.phase, WritePhase::Completed);
        assert!(
            target
                .path()
                .join("云枢AI便携工作站/Factory/Seeds/UI")
                .is_dir()
        );
        assert!(target.path().join("教程").is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn creates_launch_shortcut_for_workstation_launcher() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        seed(master.path());

        install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        )
        .unwrap();

        let shortcut = target.path().join(LAUNCH_SHORTCUT_NAME);
        assert!(fs::metadata(&shortcut).unwrap().len() > 0);
    }

    #[test]
    fn rejects_root_conflict_before_writing() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        seed(master.path());
        fs::write(target.path().join("使用说明.url"), b"existing").unwrap();
        let result = install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read(target.path().join("使用说明.url")).unwrap(),
            b"existing"
        );
        assert!(!target.path().join("云枢AI便携工作站").exists());
    }

    #[test]
    fn rejects_launch_shortcut_conflict_before_writing() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        seed(master.path());
        fs::write(target.path().join("立即启动.lnk"), b"existing").unwrap();

        let result = install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read(target.path().join("立即启动.lnk")).unwrap(),
            b"existing"
        );
        assert!(!target.path().join("云枢AI便携工作站").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_symlink_parent_for_usb_root_mapping() {
        use std::os::unix::fs::symlink;

        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        let outside = tempdir().unwrap();
        seed(master.path());
        symlink(outside.path(), target.path().join("docs")).unwrap();
        let mut release = manifest(master.path());
        release.files[1].relative_path = "docs/使用说明.url".into();
        release.root_mappings[0].destination = "docs/使用说明.url".into();

        let result = install_from_master(
            master.path(),
            &release,
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        );

        assert!(result.is_err());
        assert!(!outside.path().join("使用说明.url").exists());
        assert!(!target.path().join("云枢AI便携工作站").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_symlink_parent_for_product_directory() {
        use std::os::unix::fs::symlink;

        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        let outside = tempdir().unwrap();
        seed(master.path());
        symlink(outside.path(), target.path().join("products")).unwrap();
        let mut release = manifest(master.path());
        release.product_directory = "products/云枢AI便携工作站".into();

        let result = install_from_master(
            master.path(),
            &release,
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || false,
        );

        assert!(result.is_err());
        assert!(!outside.path().join("云枢AI便携工作站").exists());
    }

    #[test]
    fn cancellation_removes_only_owned_staging() {
        let master = tempdir().unwrap();
        let target = tempdir().unwrap();
        seed(master.path());
        fs::write(target.path().join("keep.txt"), b"keep").unwrap();
        let result = install_from_master(
            master.path(),
            &manifest(master.path()),
            &InstallRequest::new(target.path().to_path_buf()),
            |_| {},
            || true,
        )
        .unwrap();
        assert_eq!(result.phase, WritePhase::Cancelled);
        assert_eq!(fs::read(target.path().join("keep.txt")).unwrap(), b"keep");
    }
}
