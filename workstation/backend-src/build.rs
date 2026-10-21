use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let mut files = Vec::new();
    collect_rust_files(Path::new("src"), &mut files);
    files.push(PathBuf::from("Cargo.toml"));
    files.sort();

    // 使用稳定、无依赖的 FNV-1a 生成源码指纹。启动器静态链接本 crate，外部
    // workstation-runtime.exe 也从同一源码构建；两者指纹不一致时拒绝混装启动。
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for path in files {
        hash_bytes(&mut hash, path.to_string_lossy().as_bytes());
        if let Ok(bytes) = fs::read(&path) {
            hash_bytes(&mut hash, &bytes);
        }
    }
    println!("cargo:rustc-env=YUNSHU_RUNTIME_BUILD_ID={hash:016x}");
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}
