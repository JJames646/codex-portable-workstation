#![cfg_attr(windows, windows_subsystem = "windows")]

#[used]
static NATIVE_TEMPLATE_MARKER: [u8; 26] = *b"YUNSHU_NATIVE_INSTALLER_V1";

#[cfg(windows)]
mod folder_ui;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod presentation;

#[cfg(windows)]
fn main() {
    if let Err(error) = folder_ui::run() {
        folder_ui::show_fatal_error(&error.to_string());
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("云枢AI一键写入U盘仅支持Windows");
}
