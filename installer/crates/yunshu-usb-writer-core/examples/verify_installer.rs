use std::path::Path;
use yunshu_usb_writer_core::verify_single_exe_installer_deep_with_public_key;

fn main() -> anyhow::Result<()> {
    let installer = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("用法: verify_installer <安装包EXE> <公钥文件>"))?;
    let public_key = std::env::args_os()
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("缺少公钥文件"))?;
    let output = std::env::args_os().nth(3);
    let public_key: [u8; 32] = std::fs::read(&public_key)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥文件长度必须为32字节"))?;
    let verified =
        verify_single_exe_installer_deep_with_public_key(Path::new(&installer), &public_key)?;
    let summary = serde_json::json!({
        "status": "verified",
        "path": verified.path,
        "footerSchemaVersion": verified.footer.schema_version,
        "manifestSchemaVersion": verified.manifest.schema_version,
        "payloadArchitecture": verified.manifest.payload_architecture,
        "productVersion": verified.manifest.product_version,
        "buildId": verified.manifest.build_id,
        "fileCount": verified.manifest.file_count,
        "expandedBytes": verified.manifest.expanded_bytes,
        "payloadBytes": verified.footer.payload_length,
    });
    if let Some(output) = output {
        let output = Path::new(&output);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&summary)?)?;
        file.sync_all()?;
        println!("verified");
    } else {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    }
    Ok(())
}
