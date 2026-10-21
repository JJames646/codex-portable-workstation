use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use yunshu_usb_writer_core::{
    InstallRequest, build_release_manifest, install_from_master, install_from_verified_installer,
    validate_master, verify_single_exe_installer_deep_with_public_key,
};

fn main() -> anyhow::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 7 {
        anyhow::bail!(
            "用法: compare_write_modes <母盘> <安装包> <公钥> <直接写入目录> <安装包写入目录> <报告> <版本>"
        );
    }
    let master = Path::new(&args[0]);
    let installer_path = Path::new(&args[1]);
    let public_key: [u8; 32] = fs::read(&args[2])?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥文件长度必须为32字节"))?;
    let direct_target = PathBuf::from(&args[3]);
    let package_target = PathBuf::from(&args[4]);
    let report_path = Path::new(&args[5]);
    let version = args[6]
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("版本不是有效文本"))?;

    for target in [&direct_target, &package_target] {
        if target.exists() {
            anyhow::bail!("测试目标已存在，拒绝覆盖: {}", target.display());
        }
        fs::create_dir_all(target)?;
    }

    let validation = validate_master(master)?;
    if !validation.valid {
        anyhow::bail!("母盘验证未通过");
    }
    let direct_manifest = build_release_manifest(
        master,
        version,
        validation.build_id.as_deref().unwrap_or_default(),
        validation
            .factory
            .as_ref()
            .map(|value| value.file_count)
            .unwrap_or(0),
    )?;
    let installer = verify_single_exe_installer_deep_with_public_key(installer_path, &public_key)?;
    if direct_manifest.build_id != installer.manifest.build_id
        || direct_manifest.product_directory != installer.manifest.product_directory
        || direct_manifest.expanded_bytes != installer.manifest.expanded_bytes
        || direct_manifest.file_count != installer.manifest.file_count
        || direct_manifest.files != installer.manifest.files
        || direct_manifest.root_mappings != installer.manifest.root_mappings
    {
        anyhow::bail!("直接写入清单与用户安装包清单不一致");
    }

    let direct_status = install_from_master(
        master,
        &direct_manifest,
        &InstallRequest::new(direct_target.clone()),
        |_| {},
        || false,
    )?;
    let package_status = install_from_verified_installer(
        &installer,
        &InstallRequest::new(package_target.clone()),
        |_| {},
        || false,
    )?;

    let report = serde_json::json!({
        "schemaVersion": 1,
        "status": "passed",
        "buildId": direct_manifest.build_id,
        "fileCount": direct_manifest.file_count,
        "expandedBytes": direct_manifest.expanded_bytes,
        "rootMappingCount": direct_manifest.root_mappings.len(),
        "manifestFilesIdentical": true,
        "directWritePhase": direct_status.phase,
        "installerWritePhase": package_status.phase,
        "directTarget": direct_target,
        "installerTarget": package_target,
    });
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(report_path)?;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    output.sync_all()?;
    println!("passed");
    Ok(())
}
