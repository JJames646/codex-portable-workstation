use std::path::Path;
use yunshu_usb_writer_core::list_usb_targets;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let required_bytes = args
        .next()
        .and_then(|value| value.into_string().ok())
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let excluded = args.next();
    let targets = list_usb_targets(required_bytes, excluded.as_deref().map(Path::new))?;
    println!("{}", serde_json::to_string_pretty(&targets)?);
    Ok(())
}
