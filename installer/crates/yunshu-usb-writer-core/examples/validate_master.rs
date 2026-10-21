use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use yunshu_usb_writer_core::{build_release_manifest, validate_master};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let master = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("用法: validate_master <母盘目录> <产品版本> [清单文件]"))?;
    let version = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| anyhow::anyhow!("缺少产品版本"))?;
    let output = args.next();
    let validation = validate_master(Path::new(&master))?;
    if !validation.valid {
        println!("{}", serde_json::to_string_pretty(&validation)?);
        anyhow::bail!("母盘验证未通过");
    }
    let manifest = build_release_manifest(
        Path::new(&master),
        &version,
        validation.build_id.as_deref().unwrap_or_default(),
        validation
            .factory
            .as_ref()
            .map(|value| value.file_count)
            .unwrap_or(0),
    )?;
    if let Some(output) = output {
        let output = Path::new(&output);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "passed",
                "manifest": output,
                "schemaVersion": manifest.schema_version,
                "productVersion": manifest.product_version,
                "buildId": manifest.build_id,
                "fileCount": manifest.file_count,
                "expandedBytes": manifest.expanded_bytes,
                "factoryFileCount": manifest.factory_file_count,
                "rootMappingCount": manifest.root_mappings.len(),
            }))?
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
    }
    Ok(())
}
