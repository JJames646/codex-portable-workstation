use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use yunshu_workstation_runtime::model::Engine;
use yunshu_workstation_runtime::paths::WorkstationPaths;

const MAX_TUTORIAL_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TUTORIAL_ASSET_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialEntry {
    pub id: String,
    pub engine: Engine,
    pub title: String,
    pub file_name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialDocument {
    pub entry: TutorialEntry,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialAsset {
    pub media_type: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialSearchHit {
    pub entry: TutorialEntry,
    pub excerpt: String,
}

pub fn list(paths: &WorkstationPaths, engine: Engine) -> Result<Vec<TutorialEntry>> {
    let directory = tutorial_directory(paths, engine);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("无法读取教程目录 {}", directory.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        })
        .map(|entry| build_entry(engine, entry.path()))
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(entries)
}

pub fn read(paths: &WorkstationPaths, engine: Engine, file_name: &str) -> Result<TutorialDocument> {
    validate_file_name(file_name)?;
    let directory = tutorial_directory(paths, engine);
    let path = directory.join(file_name);
    ensure_within_directory(&directory, &path)?;
    let metadata = fs::metadata(&path)
        .with_context(|| format!("教程不存在或尚未通过发布审查：{}", path.display()))?;
    if metadata.len() > MAX_TUTORIAL_BYTES {
        bail!("教程文件超过5MB读取限制");
    }
    let markdown =
        fs::read_to_string(&path).with_context(|| format!("无法读取教程 {}", path.display()))?;
    Ok(TutorialDocument {
        entry: build_entry(engine, path)?,
        markdown,
    })
}

pub fn search(
    paths: &WorkstationPaths,
    engine: Engine,
    query: &str,
) -> Result<Vec<TutorialSearchHit>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    for entry in list(paths, engine)? {
        let document = read(paths, engine, &entry.file_name)?;
        let normalized = document.markdown.to_lowercase();
        if entry.title.to_lowercase().contains(&query) || normalized.contains(&query) {
            let excerpt = excerpt_around(&document.markdown, &query, 180);
            hits.push(TutorialSearchHit { entry, excerpt });
        }
        if hits.len() >= 100 {
            break;
        }
    }
    Ok(hits)
}

pub fn read_asset(
    paths: &WorkstationPaths,
    engine: Engine,
    asset_path: &str,
) -> Result<TutorialAsset> {
    let directory = tutorial_directory(paths, engine);
    let relative = validate_asset_path(asset_path)?;
    let path = directory.join(relative);
    ensure_within_directory(&directory, &path)?;
    let metadata =
        fs::metadata(&path).with_context(|| format!("教程图片不存在：{}", path.display()))?;
    if !metadata.is_file() {
        bail!("教程资源不是文件");
    }
    if metadata.len() > MAX_TUTORIAL_ASSET_BYTES {
        bail!("教程图片超过10MB读取限制");
    }
    let media_type = tutorial_asset_media_type(&path)?;
    let bytes = fs::read(&path).with_context(|| format!("无法读取教程图片 {}", path.display()))?;
    Ok(TutorialAsset {
        data_url: format!("data:{media_type};base64,{}", BASE64.encode(bytes)),
        media_type: media_type.to_string(),
    })
}

fn tutorial_directory(paths: &WorkstationPaths, engine: Engine) -> PathBuf {
    let slug = match engine {
        Engine::Codex => "codex",
        Engine::Claude => "claude-code",
        Engine::OpenClaw => "openclaw",
    };
    paths.root.join("docs").join("tutorials").join(slug)
}

fn build_entry(engine: Engine, path: PathBuf) -> Result<TutorialEntry> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("教程文件名不是有效UTF-8")?
        .to_string();
    let bytes = fs::metadata(&path)
        .with_context(|| format!("无法读取教程信息 {}", path.display()))?
        .len();
    let markdown =
        fs::read_to_string(&path).with_context(|| format!("无法读取教程 {}", path.display()))?;
    let title = markdown
        .lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("未命名教程")
                .to_string()
        });
    Ok(TutorialEntry {
        id: format!("{}:{file_name}", engine.slug()),
        engine,
        title,
        file_name,
        bytes,
    })
}

fn validate_file_name(file_name: &str) -> Result<()> {
    let path = Path::new(file_name);
    if file_name.is_empty()
        || file_name.len() > 255
        || path.components().count() != 1
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
        })
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        bail!("教程文件名不安全");
    }
    Ok(())
}

fn validate_asset_path(value: &str) -> Result<PathBuf> {
    let normalized = value
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim()
        .replace('\\', "/");
    let path = PathBuf::from(&normalized);
    if normalized.is_empty()
        || normalized.len() > 1024
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("教程图片路径不安全");
    }
    tutorial_asset_media_type(&path)?;
    Ok(path)
}

fn tutorial_asset_media_type(path: &Path) -> Result<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Ok("image/png"),
        Some("jpg" | "jpeg") => Ok("image/jpeg"),
        Some("webp") => Ok("image/webp"),
        Some("gif") => Ok("image/gif"),
        _ => bail!("教程图片只支持PNG、JPG、WebP或GIF"),
    }
}

fn ensure_within_directory(directory: &Path, path: &Path) -> Result<()> {
    if !directory.is_dir() {
        bail!("当前智能体没有已通过发布审查的教程");
    }
    let canonical_directory = directory
        .canonicalize()
        .with_context(|| format!("无法解析教程目录 {}", directory.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("教程不存在 {}", path.display()))?;
    if !canonical_path.starts_with(canonical_directory) {
        bail!("教程路径越出正式目录");
    }
    Ok(())
}

fn excerpt_around(markdown: &str, lower_query: &str, max_chars: usize) -> String {
    let matching_line = markdown
        .lines()
        .find(|line| line.to_lowercase().contains(lower_query))
        .unwrap_or(markdown);
    matching_line
        .chars()
        .take(max_chars)
        .collect::<String>()
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutorial_reader_rejects_directory_traversal() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("工作站"));
        let result = read(&paths, Engine::Codex, "../secret.md");
        assert!(result.is_err());
    }

    #[test]
    fn tutorial_excerpt_is_safe_for_chinese_text() {
        let markdown = "第一段内容\n这里包含便携工作站中文关键字\n最后一段";
        let excerpt = excerpt_around(markdown, "工作站", 20);
        assert!(excerpt.contains("工作站"));
    }

    #[test]
    fn tutorial_asset_is_confined_and_returned_as_data_url() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("工作站"));
        let directory = paths.root.join("docs/tutorials/codex/images");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("example.png"), b"png-bytes").unwrap();

        let asset = read_asset(&paths, Engine::Codex, "images/example.png").unwrap();
        assert_eq!(asset.media_type, "image/png");
        assert!(asset.data_url.starts_with("data:image/png;base64,"));
        assert!(read_asset(&paths, Engine::Codex, "../secret.png").is_err());
    }
}
