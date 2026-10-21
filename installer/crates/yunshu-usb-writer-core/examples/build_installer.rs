use std::path::Path;
use yunshu_usb_writer_core::{
    build_release_manifest, build_single_exe_installer, load_or_create_signing_key,
    validate_installer_template, validate_master,
};

fn main() -> anyhow::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 5 {
        anyhow::bail!(
            "用法: build_installer <母盘目录> <安装器模板EXE> <输出EXE> <私钥文件> <版本>"
        );
    }
    let master = Path::new(&args[0]);
    let validation = validate_master(master)?;
    if !validation.valid {
        anyhow::bail!("母盘验证未通过");
    }
    let version = args[4]
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("版本不是有效文本"))?;
    let manifest = build_release_manifest(
        master,
        version,
        validation.build_id.as_deref().unwrap_or_default(),
        validation
            .factory
            .as_ref()
            .map(|value| value.file_count)
            .unwrap_or(0),
    )?;
    let key = load_or_create_signing_key(Path::new(&args[3]))?;
    validate_installer_template(Path::new(&args[1]), &key.verifying_key().to_bytes())?;
    let verified = build_single_exe_installer(
        master,
        &manifest,
        Path::new(&args[1]),
        Path::new(&args[2]),
        &key,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": "completed",
            "output": verified.path,
            "buildId": verified.manifest.build_id,
            "fileCount": verified.manifest.file_count,
            "expandedBytes": verified.manifest.expanded_bytes,
            "footerSchemaVersion": verified.footer.schema_version,
            "manifestSchemaVersion": verified.manifest.schema_version,
            "payloadArchitecture": verified.manifest.payload_architecture,
        }))?
    );
    Ok(())
}
