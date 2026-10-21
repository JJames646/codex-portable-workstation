use crate::{DEFAULT_PRODUCT_DIRECTORY, DELIVERY_SOURCE_DIRECTORY};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileScope {
    Product,
    UsbRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseFile {
    pub source: String,
    pub scope: FileScope,
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDirectory {
    pub source: String,
    pub scope: FileScope,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootFileMapping {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PayloadArchitecture {
    X64,
    Arm64,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCompatibility {
    pub supported: Vec<String>,
    pub risk_trial: Vec<String>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub product_version: String,
    pub build_id: String,
    pub generated_at: String,
    pub product_directory: String,
    pub expanded_bytes: u64,
    pub file_count: u64,
    pub factory_file_count: usize,
    #[serde(default)]
    pub payload_architecture: PayloadArchitecture,
    #[serde(default)]
    pub host_compatibility: HostCompatibility,
    pub payload_sha256: Option<String>,
    pub files: Vec<ReleaseFile>,
    pub root_mappings: Vec<RootFileMapping>,
    #[serde(default)]
    pub directories: Vec<ReleaseDirectory>,
}

fn map_source_path(
    source_relative: &Path,
    delivery_prefix: &Path,
    delivery_root: &Path,
) -> Result<Option<(FileScope, PathBuf)>> {
    let mapped = if source_relative.starts_with(delivery_prefix) {
        source_relative
            .strip_prefix(delivery_prefix)
            .context("交付资料路径映射失败")?
    } else if source_relative.starts_with(delivery_root) {
        source_relative
            .strip_prefix(delivery_root)
            .context("交付资料路径映射失败")?
    } else {
        return Ok(Some((FileScope::Product, source_relative.to_path_buf())));
    };
    if mapped.as_os_str().is_empty() {
        return Ok(None);
    }
    Ok(Some((FileScope::UsbRoot, mapped.to_path_buf())))
}

pub fn build_release_manifest(
    master_root: &Path,
    product_version: &str,
    build_id: &str,
    factory_file_count: usize,
) -> Result<ReleaseManifest> {
    if product_version.trim().is_empty() {
        bail!("产品版本不能为空");
    }
    let delivery_prefix = Path::new(DELIVERY_SOURCE_DIRECTORY);
    let delivery_root = delivery_prefix
        .parent()
        .ok_or_else(|| anyhow::anyhow!("交付资料路径无效"))?;
    let mut files = Vec::new();
    let mut directories = Vec::new();

    for entry in WalkDir::new(master_root).follow_links(false) {
        let entry = entry.context("扫描母盘失败")?;
        if entry.path() == master_root {
            continue;
        }
        if entry.file_type().is_symlink() {
            bail!("母盘包含符号链接或重解析点: {}", entry.path().display());
        }
        let source_relative = entry
            .path()
            .strip_prefix(master_root)
            .context("母盘文件不在母盘根目录内")?;
        validate_relative_path(source_relative)?;

        let Some((scope, relative_path)) =
            map_source_path(source_relative, delivery_prefix, delivery_root)?
        else {
            continue;
        };

        validate_relative_path(&relative_path)?;
        if entry.file_type().is_dir() {
            let mut children = entry.path().read_dir().context("读取母盘目录失败")?;
            if children.next().is_none() {
                directories.push(ReleaseDirectory {
                    source: path_to_manifest(source_relative)?,
                    scope,
                    relative_path: path_to_manifest(&relative_path)?,
                });
            }
            continue;
        }
        let metadata = entry.metadata().context("读取母盘文件属性失败")?;
        files.push(ReleaseFile {
            source: path_to_manifest(source_relative)?,
            scope,
            relative_path: path_to_manifest(&relative_path)?,
            bytes: metadata.len(),
            sha256: sha256_file(entry.path())?,
        });
    }

    files.sort_by(|left, right| left.source.cmp(&right.source));
    directories.sort_by(|left, right| left.source.cmp(&right.source));
    if files.is_empty() {
        bail!("母盘没有可写入文件");
    }
    let mut destinations = HashSet::new();
    for directory in &directories {
        let key = (
            directory.scope,
            directory.relative_path.to_ascii_lowercase(),
        );
        if !destinations.insert(key) {
            bail!(
                "多个母盘目录映射到同一目标路径: {}",
                directory.relative_path
            );
        }
    }
    for file in &files {
        let key = (file.scope, file.relative_path.to_ascii_lowercase());
        if !destinations.insert(key) {
            bail!("多个母盘文件映射到同一目标路径: {}", file.relative_path);
        }
    }
    let expanded_bytes = files.iter().map(|file| file.bytes).sum();
    let root_mappings = files
        .iter()
        .filter(|file| file.scope == FileScope::UsbRoot)
        .map(|file| RootFileMapping {
            source: file.source.clone(),
            destination: file.relative_path.clone(),
        })
        .collect();

    Ok(ReleaseManifest {
        schema_version: 2,
        product_version: product_version.trim().to_owned(),
        build_id: build_id.to_owned(),
        generated_at: Utc::now().to_rfc3339(),
        product_directory: DEFAULT_PRODUCT_DIRECTORY.to_owned(),
        expanded_bytes,
        file_count: files.len() as u64,
        factory_file_count,
        payload_architecture: PayloadArchitecture::X64,
        host_compatibility: HostCompatibility {
            supported: vec!["windows-10-x64".into(), "windows-11-x64".into()],
            risk_trial: vec!["windows-11-arm64".into()],
            blocked: vec!["windows-10-arm64".into(), "windows-x86".into()],
        },
        payload_sha256: None,
        files,
        root_mappings,
        directories,
    })
}

pub(crate) fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("相对路径为空或包含绝对路径: {}", path.display());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("相对路径包含越界成分: {}", path.display()),
        }
    }
    Ok(())
}

pub(crate) fn manifest_path_to_path(value: &str) -> Result<PathBuf> {
    let path = value.split('/').collect::<PathBuf>();
    validate_relative_path(&path)?;
    Ok(path)
}

pub(crate) fn path_to_manifest(path: &Path) -> Result<String> {
    validate_relative_path(path)?;
    let parts = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => bail!("路径包含非法成分: {}", path.display()),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.join("/"))
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("无法打开文件: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn maps_delivery_files_to_usb_root_and_excludes_wrapper() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("backend")).unwrap();
        fs::write(dir.path().join("backend/runtime.exe"), b"runtime").unwrap();
        fs::create_dir_all(dir.path().join("Factory/Seeds/UI")).unwrap();
        fs::create_dir_all(dir.path().join("交付资料/写入U盘根目录")).unwrap();
        fs::write(
            dir.path().join("交付资料/写入U盘根目录/使用说明.url"),
            b"url",
        )
        .unwrap();
        fs::write(dir.path().join("交付资料/环境助手.exe"), b"helper").unwrap();

        let manifest = build_release_manifest(dir.path(), "1.0.0", "build-1", 1626).unwrap();
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.payload_architecture, PayloadArchitecture::X64);
        assert_eq!(manifest.files.len(), 3);
        assert!(manifest.files.iter().any(|file| {
            file.scope == FileScope::Product && file.relative_path == "backend/runtime.exe"
        }));
        assert!(manifest.files.iter().any(|file| {
            file.scope == FileScope::UsbRoot && file.relative_path == "使用说明.url"
        }));
        assert!(manifest.files.iter().any(|file| {
            file.scope == FileScope::UsbRoot && file.relative_path == "环境助手.exe"
        }));
        assert!(manifest.directories.iter().any(|directory| {
            directory.scope == FileScope::Product && directory.relative_path == "Factory/Seeds/UI"
        }));
    }

    #[test]
    fn rejects_duplicate_delivery_destinations() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("交付资料/写入U盘根目录")).unwrap();
        fs::write(dir.path().join("交付资料/使用说明.url"), b"direct").unwrap();
        fs::write(
            dir.path().join("交付资料/写入U盘根目录/使用说明.url"),
            b"nested",
        )
        .unwrap();

        assert!(build_release_manifest(dir.path(), "1.0.0", "build-1", 1626).is_err());
    }

    #[test]
    fn rejects_parent_directory_components() {
        assert!(validate_relative_path(Path::new("../secret")).is_err());
        assert!(manifest_path_to_path("ok/../../secret").is_err());
    }

    #[test]
    fn hashes_on_a_small_windows_sized_thread_stack() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("large.bin");
        fs::write(&path, vec![7_u8; 2 * 1024 * 1024]).unwrap();
        let hash = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || sha256_file(&path))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(hash.len(), 64);
    }
}
