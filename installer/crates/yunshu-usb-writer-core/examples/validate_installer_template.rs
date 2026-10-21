use std::path::Path;
use yunshu_usb_writer_core::validate_installer_template;

fn main() -> anyhow::Result<()> {
    let template = std::env::args_os().nth(1).ok_or_else(|| {
        anyhow::anyhow!("用法: validate_installer_template <模板EXE> <公钥文件> [报告]")
    })?;
    let public_key = std::env::args_os()
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("缺少公钥文件"))?;
    let output = std::env::args_os().nth(3);
    let public_key: [u8; 32] = std::fs::read(&public_key)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥文件长度必须为32字节"))?;
    let validation = validate_installer_template(Path::new(&template), &public_key)?;
    let report = serde_json::json!({
        "schemaVersion": 1,
        "status": "passed",
        "template": Path::new(&template)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        "validation": validation,
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
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&report)?)?;
        file.sync_all()?;
        println!("passed");
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}
