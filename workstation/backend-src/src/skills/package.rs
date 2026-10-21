use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::metadata::{DependencySummary, SkillDescriptor};

const MAX_ARCHIVE_FILES: usize = 4096;
const MAX_SINGLE_FILE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_UNCOMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct StagedSource {
    pub staging_root: PathBuf,
    pub skills: Vec<SkillDescriptor>,
}

pub fn inspect_source(source: &Path) -> Result<Vec<SkillDescriptor>> {
    ensure_regular_source_file(source)?;
    match source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("zip") => inspect_zip(source),
        Some("md") => inspect_markdown_file(source),
        _ => bail!(
            "Skill来源只支持 SKILL.md、Markdown文件或ZIP：{}",
            source.display()
        ),
    }
}

pub fn stage_source(source: &Path, staging_root: &Path) -> Result<StagedSource> {
    stage_source_with_metadata_policy(source, staging_root, false)
}

pub fn stage_skillhub_source(source: &Path, staging_root: &Path) -> Result<StagedSource> {
    stage_source_with_metadata_policy(source, staging_root, true)
}

fn stage_source_with_metadata_policy(
    source: &Path,
    staging_root: &Path,
    replace_upstream_metadata: bool,
) -> Result<StagedSource> {
    ensure_regular_source_file(source)?;
    if staging_root.exists() {
        fs::remove_dir_all(staging_root)
            .with_context(|| format!("无法清理旧staging {}", staging_root.display()))?;
    }
    fs::create_dir_all(staging_root)
        .with_context(|| format!("无法创建staging {}", staging_root.display()))?;

    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    let mut descriptors = match extension.as_deref() {
        Some("zip") => {
            let descriptors = inspect_zip(source)?;
            extract_zip_safely(source, staging_root)?;
            descriptors
        }
        Some("md") => {
            let descriptors = inspect_markdown_file(source)?;
            let descriptor = descriptors
                .first()
                .ok_or_else(|| anyhow!("Markdown没有生成Skill描述"))?;
            let target = staging_root.join(&descriptor.slug);
            fs::create_dir_all(&target)?;
            fs::copy(source, target.join("SKILL.md"))
                .with_context(|| format!("无法复制 {}", source.display()))?;
            descriptors
        }
        _ => bail!("不支持的Skill来源：{}", source.display()),
    };

    for descriptor in &mut descriptors {
        let skill_root = staging_root.join(path_from_slash(&descriptor.relative_root));
        if !skill_root.join("SKILL.md").is_file() {
            bail!("staging中的Skill缺少SKILL.md：{}", skill_root.display());
        }
        for reserved in [".skill-source", "_meta.json"] {
            let reserved_path = skill_root.join(reserved);
            if reserved_path.is_file() && replace_upstream_metadata {
                fs::remove_file(&reserved_path).with_context(|| {
                    format!("无法替换SkillHub上游元数据 {}", reserved_path.display())
                })?;
            } else if reserved_path.exists() {
                bail!("Skill包占用了工作站保留元数据文件：{reserved}");
            }
        }
        descriptor.package_sha256 = hash_directory(&skill_root)?;
        descriptor.dependencies = detect_dependencies(&skill_root);
    }
    Ok(StagedSource {
        staging_root: staging_root.to_path_buf(),
        skills: descriptors,
    })
}

pub fn hash_directory(root: &Path) -> Result<String> {
    if !root.is_dir() {
        bail!("Skill目录不存在：{}", root.display());
    }
    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path) in files {
        if relative == ".skill-source" || relative == "_meta.json" {
            continue;
        }
        digest.update(relative.as_bytes());
        digest.update([0]);
        let mut file =
            File::open(&path).with_context(|| format!("无法读取Skill文件 {}", path.display()))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        digest.update([0xff]);
    }
    Ok(to_hex(&digest.finalize()))
}

pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("Skill源目录不存在：{}", source.display());
    }
    if destination.exists() {
        bail!("Skill目标临时目录已经存在：{}", destination.display());
    }
    fs::create_dir_all(destination)
        .with_context(|| format!("无法创建 {}", destination.display()))?;
    copy_tree_contents(source, destination)
}

fn copy_tree_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("无法读取目录 {}", source.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("Skill包包含符号链接：{}", entry.path().display());
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree_contents(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "无法复制 {} -> {}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        } else {
            bail!("Skill包包含不支持的文件类型：{}", entry.path().display());
        }
    }
    Ok(())
}

fn inspect_markdown_file(source: &Path) -> Result<Vec<SkillDescriptor>> {
    let text =
        fs::read_to_string(source).with_context(|| format!("无法读取 {}", source.display()))?;
    let metadata = parse_skill_metadata(&text);
    let fallback = if source
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
    {
        source
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    } else {
        source
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    };
    let slug = safe_slug(
        metadata
            .get("slug")
            .or_else(|| metadata.get("name"))
            .map(String::as_str)
            .unwrap_or(fallback),
    );
    let source_hash = hash_file(source)?;
    Ok(vec![SkillDescriptor {
        name: metadata
            .get("name")
            .cloned()
            .unwrap_or_else(|| slug.clone()),
        description: metadata.get("description").cloned().unwrap_or_default(),
        version: metadata
            .get("version")
            .cloned()
            .unwrap_or_else(|| "1.0.0".to_string()),
        relative_root: slash_path(Path::new(&slug)),
        package_sha256: source_hash,
        dependencies: DependencySummary::default(),
        slug,
    }])
}

fn inspect_zip(source: &Path) -> Result<Vec<SkillDescriptor>> {
    let file = File::open(source).with_context(|| format!("无法打开 {}", source.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("无法解析ZIP {}", source.display()))?;
    validate_zip(&mut archive)?;

    let package_hash = hash_file(source)?;
    let mut skill_files = Vec::new();
    let mut entry_names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("ZIP包含越界路径：{}", entry.name()))?;
        let normalized = slash_path(&enclosed);
        entry_names.insert(normalized.clone());
        if enclosed
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
        {
            skill_files.push((index, enclosed));
        }
    }
    if skill_files.is_empty() {
        bail!("ZIP中没有找到SKILL.md");
    }

    let fallback_zip_name = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let mut descriptors = Vec::new();
    let mut used_slugs = HashSet::new();
    for (index, skill_file) in skill_files {
        let mut entry = archive.by_index(index)?;
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .with_context(|| format!("SKILL.md不是有效UTF-8：{}", entry.name()))?;
        let root = skill_file.parent().unwrap_or_else(|| Path::new(""));
        let metadata = parse_skill_metadata(&text);
        let fallback = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_zip_name);
        let slug = safe_slug(
            metadata
                .get("slug")
                .or_else(|| metadata.get("name"))
                .map(String::as_str)
                .unwrap_or(fallback),
        );
        if !used_slugs.insert(slug.clone()) {
            bail!("ZIP内存在重复Skill slug：{slug}");
        }
        let root_prefix = if root.as_os_str().is_empty() {
            String::new()
        } else {
            format!("{}/", slash_path(root))
        };
        let has = |file: &str| entry_names.contains(&format!("{root_prefix}{file}"));
        descriptors.push(SkillDescriptor {
            name: metadata
                .get("name")
                .cloned()
                .unwrap_or_else(|| slug.clone()),
            description: metadata.get("description").cloned().unwrap_or_default(),
            version: metadata
                .get("version")
                .cloned()
                .unwrap_or_else(|| "1.0.0".to_string()),
            relative_root: slash_path(root),
            package_sha256: package_hash.clone(),
            dependencies: DependencySummary {
                requirements_txt: has("requirements.txt"),
                pyproject_toml: has("pyproject.toml"),
                package_json: has("package.json"),
                package_lock: has("package-lock.json"),
                pnpm_lock: has("pnpm-lock.yaml"),
                yarn_lock: has("yarn.lock"),
            },
            slug,
        });
    }
    Ok(descriptors)
}

fn validate_zip<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>) -> Result<()> {
    if archive.len() > MAX_ARCHIVE_FILES {
        bail!(
            "ZIP文件数量超过限制：{} > {MAX_ARCHIVE_FILES}",
            archive.len()
        );
    }
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("ZIP包含越界路径：{}", entry.name()))?;
        validate_relative_path(&path)?;
        if entry.size() > MAX_SINGLE_FILE_SIZE {
            bail!("ZIP单文件超过64MB限制：{}", entry.name());
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > MAX_TOTAL_UNCOMPRESSED_SIZE {
            bail!("ZIP解压总大小超过256MB限制");
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("ZIP包含符号链接：{}", entry.name());
        }
    }
    Ok(())
}

fn extract_zip_safely(source: &Path, destination: &Path) -> Result<()> {
    let file = File::open(source)?;
    let mut archive = ZipArchive::new(file)?;
    validate_zip(&mut archive)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("ZIP包含越界路径：{}", entry.name()))?;
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output =
            File::create(&target).with_context(|| format!("无法创建 {}", target.display()))?;
        std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
    }
    Ok(())
}

fn detect_dependencies(root: &Path) -> DependencySummary {
    DependencySummary {
        requirements_txt: root.join("requirements.txt").is_file(),
        pyproject_toml: root.join("pyproject.toml").is_file(),
        package_json: root.join("package.json").is_file(),
        package_lock: root.join("package-lock.json").is_file(),
        pnpm_lock: root.join("pnpm-lock.yaml").is_file(),
        yarn_lock: root.join("yarn.lock").is_file(),
    }
}

fn collect_regular_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("Skill目录包含符号链接：{}", entry.path().display());
        }
        if file_type.is_dir() {
            collect_regular_files(root, &entry.path(), output)?;
        } else if file_type.is_file() {
            let relative = entry.path().strip_prefix(root)?.to_path_buf();
            output.push((slash_path(&relative), entry.path()));
        } else {
            bail!("Skill目录包含不支持的文件类型：{}", entry.path().display());
        }
    }
    Ok(())
}

fn parse_skill_metadata(text: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return metadata;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if matches!(key.as_str(), "name" | "slug" | "description" | "version") {
            metadata.insert(
                key,
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    metadata
}

fn safe_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for character in value.trim().chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                slug.push(lower);
            }
            previous_separator = false;
        } else if matches!(character, '-' | '_' | ' ' | '.') && !previous_separator {
            slug.push('-');
            previous_separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3",
    ];
    if slug.is_empty() || reserved.iter().any(|item| item.eq_ignore_ascii_case(slug)) {
        "portable-skill".to_string()
    } else {
        slug.chars().take(80).collect()
    }
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        bail!("Skill包包含绝对路径：{}", path.display());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => bail!("Skill包包含不安全路径：{}", path.display()),
        }
    }
    Ok(())
}

fn ensure_regular_source_file(source: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Skill来源不存在：{}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("Skill来源必须是普通文件：{}", source.display());
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(to_hex(&digest.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn path_from_slash(value: &str) -> PathBuf {
    value.split('/').filter(|part| !part.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    #[test]
    fn markdown_skill_is_staged_with_portable_slug() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("知识 助手.md");
        fs::write(
            &source,
            "---\nname: 知识助手\nversion: 1.2.0\ndescription: 测试\n---\n正文\n",
        )
        .unwrap();
        let stage = stage_source(&source, &temporary.path().join("stage")).unwrap();
        assert_eq!(stage.skills[0].slug, "知识助手");
        assert!(
            stage
                .staging_root
                .join("知识助手")
                .join("SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn zip_traversal_is_rejected() {
        let temporary = tempdir().unwrap();
        let zip_path = temporary.path().join("unsafe.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("../SKILL.md", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"---\nname: unsafe\n---").unwrap();
        writer.finish().unwrap();

        assert!(inspect_zip(&zip_path).is_err());
    }
}
