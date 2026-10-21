use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use yunshu_usb_writer_core::{
    build_release_manifest, build_single_exe_installer, load_signing_key,
    verify_single_exe_installer_deep_with_public_key,
};

fn main() -> anyhow::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 5 {
        anyhow::bail!(
            "用法: large_installer_smoke <安装器模板> <私钥> <公钥> <测试目录> <报告文件>"
        );
    }
    let template = Path::new(&args[0]);
    let signing_key = load_signing_key(Path::new(&args[1]))?;
    let public_key: [u8; 32] = fs::read(&args[2])?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥文件长度必须为32字节"))?;
    let root = Path::new(&args[3]);
    let report_path = Path::new(&args[4]);
    if root.exists() {
        anyhow::bail!("大文件测试目录已存在，拒绝覆盖");
    }
    fs::create_dir_all(root)?;
    let master = root.join("master");
    fs::create_dir_all(&master)?;
    fs::write(master.join("payload.txt"), b"stage11-large-file-smoke")?;

    let stub = root.join("large-template.exe");
    fs::copy(template, &stub)?;
    OpenOptions::new()
        .write(true)
        .open(&stub)?
        .set_len(4_u64 * 1024 * 1024 * 1024 + 64 * 1024 * 1024)?;
    let manifest = build_release_manifest(&master, "large-file-smoke", "large-file-smoke", 0)?;
    let installer = root.join("large-installer.exe");
    let verified = build_single_exe_installer(&master, &manifest, &stub, &installer, &signing_key)?;
    verify_single_exe_installer_deep_with_public_key(&installer, &public_key)?;

    let copied = root.join("large-installer-copy.exe");
    fs::copy(&installer, &copied)?;
    verify_single_exe_installer_deep_with_public_key(&copied, &public_key)?;
    let installer_bytes = fs::metadata(&installer)?.len();
    let copied_bytes = fs::metadata(&copied)?.len();
    let report = serde_json::json!({
        "schemaVersion": 1,
        "status": "passed",
        "installerBytes": installer_bytes,
        "copiedBytes": copied_bytes,
        "largerThan4GiB": installer_bytes > 4_u64 * 1024 * 1024 * 1024,
        "footerSchemaVersion": verified.footer.schema_version,
        "strictEd25519Verified": true,
        "copiedInstallerVerified": true,
        "largeFilesRemovedAfterVerification": true,
    });
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut report_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(report_path)?;
    report_file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    report_file.sync_all()?;

    fs::remove_file(&copied)?;
    fs::remove_file(&installer)?;
    fs::remove_file(&stub)?;
    println!("passed");
    Ok(())
}
