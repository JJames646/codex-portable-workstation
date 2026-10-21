use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use yunshu_usb_writer_core::load_or_create_signing_key;

fn main() -> anyhow::Result<()> {
    let private_key = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("用法: provision_signing_key <私钥文件> <公钥文件>"))?;
    let public_key = std::env::args_os()
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("缺少公钥文件"))?;
    let key = load_or_create_signing_key(Path::new(&private_key))?;
    let public_key = Path::new(&public_key);
    if let Some(parent) = public_key.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(public_key)?;
    output.write_all(&key.verifying_key().to_bytes())?;
    output.sync_all()?;
    println!("{}", hex::encode(key.verifying_key().to_bytes()));
    Ok(())
}
