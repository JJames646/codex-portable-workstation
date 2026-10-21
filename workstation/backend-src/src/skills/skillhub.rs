use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::paths::WorkstationPaths;
use crate::windows_process::replace_file_atomic;

use super::metadata::write_json_atomic;
use super::skillhub_signature::{
    SkillHubPlatformKeys, SkillHubSignatureEnvelope, VerifiedSkillHubSignature, verify_package,
};

const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_API_BASE: &str = "https://api.skillhub.cn";
const DEFAULT_WEB_BASE: &str = "https://skillhub.cn";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillHubConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub api_base: String,
    pub web_base: String,
    pub catalog_ttl_seconds: u64,
    pub categories_ttl_seconds: u64,
    pub max_cache_bytes: u64,
    pub allow_unsigned: bool,
}

impl Default for SkillHubConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            api_base: DEFAULT_API_BASE.to_string(),
            web_base: DEFAULT_WEB_BASE.to_string(),
            catalog_ttl_seconds: 6 * 60 * 60,
            categories_ttl_seconds: 24 * 60 * 60,
            max_cache_bytes: 2 * 1024 * 1024 * 1024,
            allow_unsigned: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedJson {
    schema_version: u32,
    fetched_at_unix_ms: u128,
    request_url: String,
    data: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillHubQueryResult {
    pub source: String,
    pub cache_status: String,
    pub fetched_at_unix_ms: u128,
    pub data: Value,
}

#[derive(Debug, Clone)]
pub struct SkillHubCoordinate {
    pub namespace: String,
    pub slug: String,
    pub version: Option<String>,
}

impl SkillHubCoordinate {
    pub fn canonical(&self, version: &str) -> String {
        format!("skillhub:@{}/{}@{version}", self.namespace, self.slug)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedSkillHubPackage {
    pub path: PathBuf,
    pub slug: String,
    pub namespace: String,
    pub version: String,
    pub coordinate: String,
    pub display_name: String,
    pub description_zh: String,
    pub package_sha256: String,
    pub content_hash: String,
    pub signature_key_id: String,
    pub signature_verified: bool,
    pub target_directory: String,
}

pub fn ensure_default_config(paths: &WorkstationPaths) -> Result<()> {
    let path = config_path(paths);
    if path.is_file() {
        return Ok(());
    }
    write_json_atomic(&path, &SkillHubConfig::default())
}

pub fn read_config(paths: &WorkstationPaths) -> Result<SkillHubConfig> {
    let path = config_path(paths);
    let config = if path.is_file() {
        let bytes = fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?
    } else {
        SkillHubConfig::default()
    };
    validate_config(&config)?;
    Ok(config)
}

pub async fn categories(paths: &WorkstationPaths, offline: bool) -> Result<SkillHubQueryResult> {
    let config = read_config(paths)?;
    let url = endpoint_url(&config, &["api", "v1", "categories"])?;
    query_json(
        paths,
        &config,
        url,
        "categories",
        config.categories_ttl_seconds,
        offline,
    )
    .await
}

pub async fn recommended(paths: &WorkstationPaths, offline: bool) -> Result<SkillHubQueryResult> {
    let config = read_config(paths)?;
    let url = endpoint_url(&config, &["api", "v1", "showcase", "recommended"])?;
    query_json(
        paths,
        &config,
        url,
        "recommended",
        config.catalog_ttl_seconds,
        offline,
    )
    .await
}

pub async fn search(
    paths: &WorkstationPaths,
    query: Option<&str>,
    category: Option<&str>,
    page: u32,
    page_size: u32,
    sort_by: &str,
    offline: bool,
) -> Result<SkillHubQueryResult> {
    if page == 0 {
        bail!("SkillHub page必须大于0");
    }
    if !(1..=100).contains(&page_size) {
        bail!("SkillHub pageSize必须在1到100之间");
    }
    if !matches!(
        sort_by,
        "downloads" | "installs" | "stars" | "score" | "updated_at"
    ) {
        bail!("不支持的SkillHub排序字段：{sort_by}");
    }
    let config = read_config(paths)?;
    let mut url = endpoint_url(&config, &["api", "skills"])?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("page", &page.to_string())
            .append_pair("pageSize", &page_size.to_string())
            .append_pair("sortBy", sort_by);
        if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            pairs.append_pair("q", query.trim());
        }
        if let Some(category) = category.filter(|value| !value.trim().is_empty()) {
            pairs.append_pair("category", category.trim());
        }
    }
    query_json(
        paths,
        &config,
        url,
        "search",
        config.catalog_ttl_seconds,
        offline,
    )
    .await
}

pub async fn detail(
    paths: &WorkstationPaths,
    reference: &str,
    offline: bool,
) -> Result<SkillHubQueryResult> {
    let coordinate = parse_reference(reference)?;
    let config = read_config(paths)?;
    let mut url = endpoint_url(&config, &["api", "v1", "skills", &coordinate.slug])?;
    url.query_pairs_mut()
        .append_pair("namespace", &coordinate.namespace);
    query_json(
        paths,
        &config,
        url,
        "details",
        config.catalog_ttl_seconds,
        offline,
    )
    .await
}

pub async fn download_package(
    paths: &WorkstationPaths,
    reference: &str,
) -> Result<DownloadedSkillHubPackage> {
    download_package_with_mode(paths, reference, false).await
}

pub async fn download_package_with_mode(
    paths: &WorkstationPaths,
    reference: &str,
    offline: bool,
) -> Result<DownloadedSkillHubPackage> {
    paths.prepare_layout()?;
    ensure_default_config(paths)?;
    let config = read_config(paths)?;
    if !config.enabled {
        bail!("SkillHub来源已禁用");
    }
    if config.allow_unsigned {
        bail!("当前版本禁止启用allowUnsigned；SkillHub包必须通过官方签名验证");
    }
    let coordinate = parse_reference(reference)?;
    let detail = detail(paths, reference, offline).await?;
    let version = coordinate
        .version
        .clone()
        .or_else(|| {
            detail
                .data
                .pointer("/latestVersion/version")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| anyhow!("SkillHub详情没有返回可安装版本"))?;
    let display_name = detail
        .data
        .pointer("/skill/displayName")
        .and_then(Value::as_str)
        .unwrap_or(&coordinate.slug)
        .to_string();
    let description_zh = detail
        .data
        .pointer("/skill/summary_zh")
        .or_else(|| detail.data.pointer("/skill/summary"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let signature = fetch_signature(paths, &config, &coordinate, &version, offline).await?;
    let keys = fetch_platform_keys(paths, &config, offline).await?;
    let destination = package_cache_path(paths, &coordinate, &version);
    if destination.is_file() {
        match verify_package(&destination, &coordinate.slug, &version, &signature, &keys) {
            Ok(verified) => {
                return build_download_result(
                    &destination,
                    &coordinate,
                    &version,
                    &display_name,
                    &description_zh,
                    verified,
                );
            }
            Err(_) => {
                fs::remove_file(&destination).with_context(|| {
                    format!("无法清理校验失败的SkillHub缓存 {}", destination.display())
                })?;
            }
        }
    }
    if offline {
        bail!("SkillHub离线包缓存不存在：{}", destination.display());
    }

    download_verified_package(
        &config,
        &coordinate,
        &version,
        &destination,
        &signature,
        &keys,
    )
    .await?;
    let verified = verify_package(&destination, &coordinate.slug, &version, &signature, &keys)?;
    prune_package_cache(paths, config.max_cache_bytes, Some(&destination))?;
    build_download_result(
        &destination,
        &coordinate,
        &version,
        &display_name,
        &description_zh,
        verified,
    )
}

pub fn parse_reference(reference: &str) -> Result<SkillHubCoordinate> {
    let value = reference
        .strip_prefix("skillhub:")
        .ok_or_else(|| anyhow!("SkillHub来源格式应为 skillhub:@namespace/slug[@version]"))?;
    let value = value
        .strip_prefix('@')
        .ok_or_else(|| anyhow!("SkillHub来源必须包含@namespace：{reference}"))?;
    let (namespace, skill_and_version) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("SkillHub来源缺少 /：{reference}"))?;
    let (slug, version) = match skill_and_version.rsplit_once('@') {
        Some((slug, version)) if !slug.is_empty() && !version.is_empty() => {
            (slug, Some(version.to_string()))
        }
        _ => (skill_and_version, None),
    };
    validate_coordinate_component("namespace", namespace)?;
    validate_coordinate_component("slug", slug)?;
    if let Some(version) = &version {
        validate_coordinate_component("version", version)?;
    }
    Ok(SkillHubCoordinate {
        namespace: namespace.to_string(),
        slug: slug.to_string(),
        version,
    })
}

fn validate_config(config: &SkillHubConfig) -> Result<()> {
    if config.schema_version != 1 {
        bail!(
            "不支持的SkillHub配置schemaVersion：{}",
            config.schema_version
        );
    }
    let api = Url::parse(&config.api_base).context("SkillHub apiBase无效")?;
    if api.scheme() != "https" || api.host_str() != Some("api.skillhub.cn") {
        bail!("SkillHub apiBase只允许官方地址 https://api.skillhub.cn");
    }
    let web = Url::parse(&config.web_base).context("SkillHub webBase无效")?;
    if web.scheme() != "https" || !matches!(web.host_str(), Some("skillhub.cn" | "www.skillhub.cn"))
    {
        bail!("SkillHub webBase只允许官方skillhub.cn地址");
    }
    if config.max_cache_bytes < MAX_PACKAGE_BYTES {
        bail!("SkillHub maxCacheBytes不能小于256MB");
    }
    Ok(())
}

fn validate_coordinate_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("SkillHub {label}包含不安全字符：{value}");
    }
    Ok(())
}

async fn query_json(
    paths: &WorkstationPaths,
    config: &SkillHubConfig,
    url: Url,
    group: &str,
    ttl_seconds: u64,
    offline: bool,
) -> Result<SkillHubQueryResult> {
    paths.prepare_layout()?;
    let cache_path = json_cache_path(paths, group, url.as_str());
    if let Some(cached) = read_json_cache(&cache_path)? {
        let age_ms = now_unix_ms().saturating_sub(cached.fetched_at_unix_ms);
        if offline || age_ms <= u128::from(ttl_seconds) * 1000 {
            return Ok(SkillHubQueryResult {
                source: "skillhub".to_string(),
                cache_status: if offline {
                    "offline-cache".to_string()
                } else {
                    "fresh-cache".to_string()
                },
                fetched_at_unix_ms: cached.fetched_at_unix_ms,
                data: cached.data,
            });
        }
    }
    if offline {
        bail!("SkillHub离线缓存不存在：{}", cache_path.display());
    }

    let online = fetch_json(config, &url).await;
    match online {
        Ok(data) => {
            let cached = CachedJson {
                schema_version: 1,
                fetched_at_unix_ms: now_unix_ms(),
                request_url: url.to_string(),
                data,
            };
            write_json_atomic(&cache_path, &cached)?;
            Ok(SkillHubQueryResult {
                source: "skillhub".to_string(),
                cache_status: "online".to_string(),
                fetched_at_unix_ms: cached.fetched_at_unix_ms,
                data: cached.data,
            })
        }
        Err(error) => {
            if let Some(cached) = read_json_cache(&cache_path)? {
                Ok(SkillHubQueryResult {
                    source: "skillhub".to_string(),
                    cache_status: "stale-cache".to_string(),
                    fetched_at_unix_ms: cached.fetched_at_unix_ms,
                    data: cached.data,
                })
            } else {
                Err(error)
            }
        }
    }
}

async fn fetch_signature(
    paths: &WorkstationPaths,
    config: &SkillHubConfig,
    coordinate: &SkillHubCoordinate,
    version: &str,
    offline: bool,
) -> Result<SkillHubSignatureEnvelope> {
    let mut url = endpoint_url(
        config,
        &[
            "api",
            "v1",
            "open",
            "skills",
            &coordinate.slug,
            "versions",
            version,
            "signature",
        ],
    )?;
    url.query_pairs_mut()
        .append_pair("namespace", &coordinate.namespace);
    let result = query_json(
        paths,
        config,
        url,
        "signatures",
        config.catalog_ttl_seconds,
        offline,
    )
    .await?;
    serde_json::from_value(result.data).context("SkillHub签名响应结构无效")
}

async fn fetch_platform_keys(
    paths: &WorkstationPaths,
    config: &SkillHubConfig,
    offline: bool,
) -> Result<SkillHubPlatformKeys> {
    let url = endpoint_url(config, &["api", "v1", "open", "platform", "keys"])?;
    let result = query_json(
        paths,
        config,
        url,
        "platform-keys",
        config.categories_ttl_seconds,
        offline,
    )
    .await?;
    serde_json::from_value(result.data).context("SkillHub平台公钥响应结构无效")
}

async fn download_verified_package(
    config: &SkillHubConfig,
    coordinate: &SkillHubCoordinate,
    version: &str,
    destination: &Path,
    signature: &SkillHubSignatureEnvelope,
    keys: &SkillHubPlatformKeys,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut url = endpoint_url(config, &["api", "v1", "download"])?;
    url.query_pairs_mut()
        .append_pair("slug", &coordinate.slug)
        .append_pair("version", version)
        .append_pair("namespace", &coordinate.namespace);
    let response = get_with_controlled_redirects(http_client()?, url).await?;
    if !response.status().is_success() {
        bail!("SkillHub下载返回HTTP {}", response.status());
    }
    if let Some(length) = response.content_length()
        && length > MAX_PACKAGE_BYTES
    {
        bail!("SkillHub ZIP超过256MB限制");
    }

    let temporary = destination.with_extension("zip.part");
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = async {
        let mut output = File::create(&temporary)
            .with_context(|| format!("无法创建下载临时文件 {}", temporary.display()))?;
        let mut received = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("SkillHub ZIP下载中断")?;
            received = received.saturating_add(chunk.len() as u64);
            if received > MAX_PACKAGE_BYTES {
                bail!("SkillHub ZIP超过256MB限制");
            }
            output.write_all(&chunk)?;
        }
        output.flush()?;
        output.sync_all()?;
        verify_package(&temporary, &coordinate.slug, version, signature, keys)?;
        replace_file_atomic(&temporary, destination)
    }
    .await;
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

async fn get_with_controlled_redirects(
    client: reqwest::Client,
    mut url: Url,
) -> Result<reqwest::Response> {
    for _ in 0..=3 {
        validate_download_url(&url)?;
        let response = client
            .get(url.clone())
            .send()
            .await
            .with_context(|| format!("无法下载SkillHub包：{url}"))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| anyhow!("SkillHub重定向缺少Location"))?
                .to_str()
                .context("SkillHub重定向Location不是UTF-8")?;
            url = url.join(location).context("SkillHub重定向URL无效")?;
            continue;
        }
        return Ok(response);
    }
    bail!("SkillHub下载重定向次数超过3次")
}

fn validate_download_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        bail!("SkillHub下载只允许HTTPS");
    }
    let host = url.host_str().unwrap_or_default();
    if host == "api.skillhub.cn"
        || (host.ends_with(".myqcloud.com") && host.contains(".cos."))
        || (host.ends_with(".myqcloud.com") && host.starts_with("skillhub-"))
    {
        return Ok(());
    }
    bail!("SkillHub下载重定向到未授权域名：{host}")
}

async fn fetch_json(config: &SkillHubConfig, url: &Url) -> Result<Value> {
    if !config.enabled {
        bail!("SkillHub来源已禁用");
    }
    if url.host_str() != Some("api.skillhub.cn") || url.scheme() != "https" {
        bail!("SkillHub JSON请求不是官方HTTPS地址");
    }
    let response = http_client()?
        .get(url.clone())
        .send()
        .await
        .with_context(|| format!("无法访问SkillHub：{url}"))?;
    if response.status() == StatusCode::NOT_FOUND {
        bail!("SkillHub没有找到对应资源：{url}");
    }
    if !response.status().is_success() {
        bail!("SkillHub返回HTTP {}：{url}", response.status());
    }
    if let Some(length) = response.content_length()
        && length > MAX_JSON_BYTES as u64
    {
        bail!("SkillHub JSON超过8MB限制");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_JSON_BYTES {
        bail!("SkillHub JSON超过8MB限制");
    }
    serde_json::from_slice(&bytes).context("SkillHub返回的JSON无效")
}

fn build_download_result(
    path: &Path,
    coordinate: &SkillHubCoordinate,
    version: &str,
    display_name: &str,
    description_zh: &str,
    verified: VerifiedSkillHubSignature,
) -> Result<DownloadedSkillHubPackage> {
    Ok(DownloadedSkillHubPackage {
        path: path.to_path_buf(),
        slug: coordinate.slug.clone(),
        namespace: coordinate.namespace.clone(),
        version: version.to_string(),
        coordinate: coordinate.canonical(version),
        display_name: display_name.to_string(),
        description_zh: description_zh.to_string(),
        package_sha256: file_sha256(path)?,
        content_hash: verified.content_hash,
        signature_key_id: verified.key_id,
        signature_verified: verified.verified,
        target_directory: namespaced_target_directory(&coordinate.slug, &coordinate.namespace),
    })
}

fn namespaced_target_directory(slug: &str, namespace: &str) -> String {
    let digest = Sha256::digest(namespace.as_bytes());
    let suffix = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{slug}--{suffix}")
}

fn endpoint_url(config: &SkillHubConfig, segments: &[&str]) -> Result<Url> {
    let mut url = Url::parse(&config.api_base)?;
    {
        let mut target = url
            .path_segments_mut()
            .map_err(|_| anyhow!("SkillHub apiBase不能作为路径基础"))?;
        target.clear();
        target.extend(segments);
    }
    Ok(url)
}

fn config_path(paths: &WorkstationPaths) -> PathBuf {
    paths.skills_config.join("skillhub.json")
}

fn json_cache_path(paths: &WorkstationPaths, group: &str, request_url: &str) -> PathBuf {
    let digest = Sha256::digest(request_url.as_bytes());
    let key = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    paths
        .skills_skillhub_cache
        .join(group)
        .join(format!("{key}.json"))
}

fn package_cache_path(
    paths: &WorkstationPaths,
    coordinate: &SkillHubCoordinate,
    version: &str,
) -> PathBuf {
    paths
        .skills_skillhub_cache
        .join("packages")
        .join(&coordinate.namespace)
        .join(&coordinate.slug)
        .join(format!("{version}.zip"))
}

fn read_json_cache(path: &Path) -> Result<Option<CachedJson>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("无法读取缓存 {}", path.display()))?;
    let cached: CachedJson = serde_json::from_slice(&bytes)
        .with_context(|| format!("缓存JSON无效 {}", path.display()))?;
    if cached.schema_version != 1 {
        return Ok(None);
    }
    Ok(Some(cached))
}

fn prune_package_cache(
    paths: &WorkstationPaths,
    max_bytes: u64,
    keep: Option<&Path>,
) -> Result<()> {
    let root = paths.skills_skillhub_cache.join("packages");
    if !root.is_dir() {
        return Ok(());
    }
    let mut files = Vec::new();
    collect_cache_files(&root, &mut files)?;
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total <= max_bytes {
        return Ok(());
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if keep.is_some_and(|keep| keep == path) {
            continue;
        }
        fs::remove_file(&path)
            .with_context(|| format!("无法清理SkillHub缓存 {}", path.display()))?;
        total = total.saturating_sub(size);
        if total <= max_bytes {
            break;
        }
    }
    Ok(())
}

fn collect_cache_files(root: &Path, output: &mut Vec<(PathBuf, u64, SystemTime)>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_cache_files(&entry.path(), output)?;
        } else if file_type.is_file() {
            let metadata = entry.metadata()?;
            output.push((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
            ));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect())
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(60))
        .user_agent("YunshuPortableWorkstation/phase3")
        .build()
        .context("无法创建SkillHub HTTP客户端")
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_skillhub_coordinate() {
        let coordinate =
            parse_reference("skillhub:@root/find-skills@1.0.0").expect("coordinate should parse");
        assert_eq!(coordinate.namespace, "root");
        assert_eq!(coordinate.slug, "find-skills");
        assert_eq!(coordinate.version.as_deref(), Some("1.0.0"));
        assert_eq!(
            coordinate.canonical("1.0.0"),
            "skillhub:@root/find-skills@1.0.0"
        );
    }

    #[test]
    fn target_directory_is_stable_and_namespace_aware() {
        assert_eq!(
            namespaced_target_directory("same", "owner-a"),
            namespaced_target_directory("same", "owner-a")
        );
        assert_ne!(
            namespaced_target_directory("same", "owner-a"),
            namespaced_target_directory("same", "owner-b")
        );
    }

    #[test]
    fn rejects_non_official_skillhub_config() {
        let config = SkillHubConfig {
            api_base: "https://example.com".to_string(),
            ..SkillHubConfig::default()
        };
        assert!(validate_config(&config).is_err());
    }
}
