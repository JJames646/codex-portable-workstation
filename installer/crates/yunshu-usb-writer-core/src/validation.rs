use crate::manifest::sha256_file;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const FORBIDDEN_MARKERS: &[&str] = &[
    "data/installation-id",
    "data/initialization.json",
    "data/factory-generation-id",
    "data/entitlement.json",
    "data/vault.key",
    "data/provider-vault.bin",
    "data/runtime/first-use/operation.json",
];

const WRITABLE_AREAS: &[&str] = &["data", "workspace", "logs", "backups", "cache"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterCheck {
    pub code: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorySummary {
    pub schema_version: u32,
    pub runtime_build_id: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterValidationResult {
    pub valid: bool,
    pub root: String,
    pub build_id: Option<String>,
    pub launcher_sha256: Option<String>,
    pub runtime_sha256: Option<String>,
    pub factory: Option<FactorySummary>,
    pub checks: Vec<MasterCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryManifest {
    schema_version: u32,
    runtime_build_id: String,
    files: Vec<FactoryFile>,
}

#[derive(Debug, Deserialize)]
struct FactoryFile {
    source: String,
    bytes: u64,
    sha256: String,
}

pub fn validate_master(root: &Path) -> Result<MasterValidationResult> {
    let root = root
        .canonicalize()
        .with_context(|| format!("无法读取母盘目录: {}", root.display()))?;
    if !root.is_dir() {
        bail!("母盘路径不是文件夹");
    }
    let mut checks = Vec::new();

    let reparse = first_reparse_point(&root)?;
    checks.push(MasterCheck {
        code: "noReparsePoints".into(),
        label: "目录链接与重解析点".into(),
        passed: reparse.is_none(),
        detail: reparse
            .map(|path| format!("发现不允许的链接: {}", display_relative(&root, &path)))
            .unwrap_or_else(|| "未发现目录链接或符号链接".into()),
    });

    let marker = FORBIDDEN_MARKERS
        .iter()
        .map(|relative| (relative, root.join(relative)))
        .find(|(_, path)| path.exists());
    checks.push(MasterCheck {
        code: "noInstallationState".into(),
        label: "安装身份与授权状态".into(),
        passed: marker.is_none(),
        detail: marker
            .map(|(relative, _)| format!("发现禁止状态文件: {relative}"))
            .unwrap_or_else(|| "未发现安装身份、授权或凭据库".into()),
    });

    let writable_file = first_file_in_writable_areas(&root)?;
    checks.push(MasterCheck {
        code: "writableAreasEmpty".into(),
        label: "可写用户目录".into(),
        passed: writable_file.is_none(),
        detail: writable_file
            .map(|path| format!("可写目录中存在文件: {}", display_relative(&root, &path)))
            .unwrap_or_else(|| "data、workspace、logs、backups、cache 均为空".into()),
    });

    let launcher = find_launcher(&root);
    checks.push(MasterCheck {
        code: "launcherPresent".into(),
        label: "工作站启动器".into(),
        passed: launcher.is_some(),
        detail: launcher
            .as_ref()
            .map(|path| format!("已找到: {}", display_relative(&root, path)))
            .unwrap_or_else(|| "母盘根目录缺少工作站启动器".into()),
    });
    let runtime = root.join("backend/workstation-runtime.exe");
    checks.push(MasterCheck {
        code: "runtimePresent".into(),
        label: "监督后端 Runtime".into(),
        passed: runtime.is_file(),
        detail: if runtime.is_file() {
            "backend/workstation-runtime.exe 已找到".into()
        } else {
            "缺少 backend/workstation-runtime.exe".into()
        },
    });

    let factory_result = verify_factory(&root);
    let factory = factory_result.as_ref().ok().cloned();
    checks.push(MasterCheck {
        code: "factoryManifest".into(),
        label: "Factory 出厂种子".into(),
        passed: factory_result.is_ok(),
        detail: match &factory_result {
            Ok(summary) => format!(
                "schema v{}，{} 项文件校验通过",
                summary.schema_version, summary.file_count
            ),
            Err(error) => error.to_string(),
        },
    });

    let launcher_sha256 = launcher.as_ref().and_then(|path| sha256_file(path).ok());
    let runtime_sha256 = runtime
        .is_file()
        .then(|| sha256_file(&runtime).ok())
        .flatten();
    let valid = checks.iter().all(|check| check.passed);
    Ok(MasterValidationResult {
        valid,
        root: root.to_string_lossy().into_owned(),
        build_id: factory.as_ref().map(|value| value.runtime_build_id.clone()),
        launcher_sha256,
        runtime_sha256,
        factory,
        checks,
    })
}

fn find_launcher(root: &Path) -> Option<PathBuf> {
    let exact = root.join("云枢AI便携工作站.exe");
    if exact.is_file() {
        return Some(exact);
    }
    fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("exe")
                })
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("工作站"))
        })
}

fn first_reparse_point(root: &Path) -> Result<Option<PathBuf>> {
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.context("检查目录链接失败")?;
        if entry.path() == root {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
            return Ok(Some(entry.path().to_path_buf()));
        }
    }
    Ok(None)
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn first_file_in_writable_areas(root: &Path) -> Result<Option<PathBuf>> {
    for relative in WRITABLE_AREAS {
        let area = root.join(relative);
        if !area.exists() {
            continue;
        }
        for entry in WalkDir::new(&area).follow_links(false) {
            let entry = entry.context("检查可写用户目录失败")?;
            if entry.file_type().is_file() || entry.file_type().is_symlink() {
                return Ok(Some(entry.path().to_path_buf()));
            }
        }
    }
    Ok(None)
}

fn verify_factory(root: &Path) -> Result<FactorySummary> {
    let factory_root = root.join("Factory");
    let manifest_path = factory_root.join("manifest.json");
    let raw = fs::read_to_string(&manifest_path).context("缺少或无法读取 Factory/manifest.json")?;
    let manifest: FactoryManifest = serde_json::from_str(&raw).context("Factory 清单格式错误")?;
    if manifest.schema_version != 2 {
        bail!(
            "Factory schemaVersion 应为2，当前为{}",
            manifest.schema_version
        );
    }
    if manifest.files.len() != 1626 {
        bail!("Factory 文件应为1626项，当前为{}项", manifest.files.len());
    }
    for file in &manifest.files {
        let relative = Path::new(&file.source);
        crate::manifest::validate_relative_path(relative)?;
        let path = factory_root.join(relative);
        let metadata =
            fs::metadata(&path).with_context(|| format!("Factory 缺少文件: {}", file.source))?;
        if metadata.len() != file.bytes {
            bail!("Factory 文件长度不匹配: {}", file.source);
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(&file.sha256) {
            bail!("Factory 文件哈希不匹配: {}", file.source);
        }
    }
    Ok(FactorySummary {
        schema_version: manifest.schema_version,
        runtime_build_id: manifest.runtime_build_id,
        file_count: manifest.files.len(),
    })
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    fn seed_factory(root: &Path) {
        let factory = root.join("Factory");
        fs::create_dir_all(factory.join("Seeds/Test")).unwrap();
        let mut files = Vec::new();
        for index in 0..1626 {
            let relative = format!("Seeds/Test/{index}.txt");
            let contents = format!("seed-{index}").into_bytes();
            fs::write(factory.join(&relative), &contents).unwrap();
            files.push(serde_json::json!({
                "source": relative,
                "bytes": contents.len(),
                "sha256": hex::encode(Sha256::digest(&contents)),
            }));
        }
        fs::write(
            factory.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 2,
                "runtimeBuildId": "fixture-build",
                "codexSha256": "fixture",
                "componentHashes": {},
                "roots": [],
                "files": files,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn seed_master(root: &Path) {
        fs::create_dir_all(root.join("backend")).unwrap();
        for area in WRITABLE_AREAS {
            fs::create_dir_all(root.join(area)).unwrap();
        }
        fs::write(root.join("云枢AI便携工作站.exe"), b"launcher").unwrap();
        fs::write(root.join("backend/workstation-runtime.exe"), b"runtime").unwrap();
        seed_factory(root);
    }

    #[test]
    fn accepts_clean_fixture_and_rejects_user_state() {
        let dir = tempdir().unwrap();
        seed_master(dir.path());
        let clean = validate_master(dir.path()).unwrap();
        assert!(clean.valid);
        assert_eq!(clean.factory.unwrap().file_count, 1626);

        fs::write(dir.path().join("data/installation-id"), b"private").unwrap();
        let dirty = validate_master(dir.path()).unwrap();
        assert!(!dirty.valid);
        assert!(
            dirty
                .checks
                .iter()
                .any(|check| check.code == "noInstallationState" && !check.passed)
        );
    }
}
