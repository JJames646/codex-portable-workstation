use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::model::SkillTarget;
use crate::paths::WorkstationPaths;

use super::metadata::write_json_atomic;

const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_PACKAGE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogConfig {
    pub schema_version: u32,
    pub sources: Vec<CatalogSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSource {
    pub id: String,
    pub url: String,
    pub public_key_base64: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedCatalogEnvelope {
    pub schema_version: u32,
    pub catalog: CatalogDocument,
    pub signature_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDocument {
    pub id: String,
    pub generated_at: String,
    pub skills: Vec<CatalogSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkill {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub download_url: String,
    pub sha256: String,
    pub targets: Vec<SkillTarget>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub published_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRefreshResult {
    pub id: String,
    pub url: String,
    pub verified: bool,
    pub skill_count: usize,
    pub cache_path: PathBuf,
}

pub async fn refresh_all(paths: &WorkstationPaths) -> Result<Vec<CatalogRefreshResult>> {
    paths.prepare_layout()?;
    let config = read_config(paths)?;
    let client = http_client()?;
    let mut results = Vec::new();
    for source in config.sources.into_iter().filter(|source| source.enabled) {
        validate_network_url(&source.url)?;
        let response = client
            .get(&source.url)
            .send()
            .await
            .with_context(|| format!("无法下载Catalog {}", source.id))?;
        if !response.status().is_success() {
            bail!("Catalog {} 返回HTTP {}", source.id, response.status());
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_CATALOG_BYTES {
            bail!("Catalog {} 超过4MB限制", source.id);
        }
        let envelope: SignedCatalogEnvelope = serde_json::from_slice(&bytes)
            .with_context(|| format!("Catalog {} JSON无效", source.id))?;
        verify_envelope(&source, &envelope)?;
        if envelope.catalog.id != source.id {
            bail!(
                "Catalog ID不匹配：配置={}，签名内容={}",
                source.id,
                envelope.catalog.id
            );
        }
        let cache_path = catalog_cache_path(paths, &source.id);
        write_json_atomic(&cache_path, &envelope)?;
        results.push(CatalogRefreshResult {
            id: source.id,
            url: source.url,
            verified: true,
            skill_count: envelope.catalog.skills.len(),
            cache_path,
        });
    }
    Ok(results)
}

pub async fn download_package(
    paths: &WorkstationPaths,
    reference: &str,
) -> Result<(PathBuf, CatalogSkill, String)> {
    let (catalog_id, slug, requested_version) = parse_reference(reference)?;
    let config = read_config(paths)?;
    let source = config
        .sources
        .iter()
        .find(|source| source.id == catalog_id && source.enabled)
        .ok_or_else(|| anyhow!("Catalog未配置或未启用：{catalog_id}"))?;
    let envelope = read_cached_catalog(paths, source)?;
    let item = envelope
        .catalog
        .skills
        .into_iter()
        .find(|item| {
            item.slug == slug
                && requested_version
                    .as_deref()
                    .is_none_or(|version| item.version == version)
        })
        .ok_or_else(|| anyhow!("Catalog中没有找到Skill：{reference}"))?;
    validate_network_url(&item.download_url)?;

    fs::create_dir_all(&paths.skills_download_cache)?;
    let file_name = format!(
        "{}-{}-{}.zip",
        safe_component(catalog_id),
        safe_component(&item.slug),
        safe_component(&item.version)
    );
    let destination = paths.skills_download_cache.join(file_name);
    if destination.is_file() && hash_bytes(&fs::read(&destination)?) == item.sha256.to_uppercase() {
        return Ok((destination, item, catalog_id.to_string()));
    }

    let response = http_client()?
        .get(&item.download_url)
        .send()
        .await
        .with_context(|| format!("无法下载Skill {}", item.slug))?;
    if !response.status().is_success() {
        bail!("Skill下载返回HTTP {}", response.status());
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        bail!("Skill ZIP超过256MB限制");
    }
    let actual_hash = hash_bytes(&bytes);
    if !actual_hash.eq_ignore_ascii_case(&item.sha256) {
        bail!(
            "Skill ZIP哈希不匹配：expected={} actual={actual_hash}",
            item.sha256
        );
    }
    let temporary = destination.with_extension("zip.tmp");
    fs::write(&temporary, &bytes)?;
    crate::windows_process::replace_file_atomic(&temporary, &destination)?;
    Ok((destination, item, catalog_id.to_string()))
}

pub fn list_cached(paths: &WorkstationPaths) -> Result<Vec<CatalogDocument>> {
    let config = read_config(paths)?;
    let mut catalogs = Vec::new();
    for source in config.sources.iter().filter(|source| source.enabled) {
        let cache = catalog_cache_path(paths, &source.id);
        if cache.is_file() {
            catalogs.push(read_cached_catalog(paths, source)?.catalog);
        }
    }
    Ok(catalogs)
}

pub fn ensure_default_config(paths: &WorkstationPaths) -> Result<()> {
    let path = config_path(paths);
    if path.is_file() {
        return Ok(());
    }
    write_json_atomic(
        &path,
        &CatalogConfig {
            schema_version: 1,
            sources: Vec::new(),
        },
    )
}

fn read_config(paths: &WorkstationPaths) -> Result<CatalogConfig> {
    let path = config_path(paths);
    if !path.is_file() {
        return Ok(CatalogConfig {
            schema_version: 1,
            sources: Vec::new(),
        });
    }
    let bytes = fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))
}

fn read_cached_catalog(
    paths: &WorkstationPaths,
    source: &CatalogSource,
) -> Result<SignedCatalogEnvelope> {
    let path = catalog_cache_path(paths, &source.id);
    let bytes = fs::read(&path)
        .with_context(|| format!("Catalog缓存不存在，请先refresh：{}", source.id))?;
    let envelope: SignedCatalogEnvelope = serde_json::from_slice(&bytes)?;
    verify_envelope(source, &envelope)?;
    Ok(envelope)
}

fn verify_envelope(source: &CatalogSource, envelope: &SignedCatalogEnvelope) -> Result<()> {
    if envelope.schema_version != 1 {
        bail!("不支持的Catalog schemaVersion：{}", envelope.schema_version);
    }
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&source.public_key_base64)
        .context("Catalog公钥不是有效Base64")?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow!("Catalog Ed25519公钥必须是32字节"))?;
    let verifying_key = VerifyingKey::from_bytes(&key_array).context("Catalog Ed25519公钥无效")?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(&envelope.signature_base64)
        .context("Catalog签名不是有效Base64")?;
    let signature = Signature::from_slice(&signature_bytes).context("Catalog签名长度无效")?;
    let payload = serde_json::to_vec(&envelope.catalog)?;
    verifying_key
        .verify(&payload, &signature)
        .context("Catalog Ed25519签名验证失败")
}

fn parse_reference(reference: &str) -> Result<(&str, &str, Option<String>)> {
    let value = reference
        .strip_prefix("catalog:")
        .ok_or_else(|| anyhow!("Catalog来源格式应为 catalog:<id>/<slug>[@version]"))?;
    let (catalog_id, skill) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("Catalog来源缺少 /：{reference}"))?;
    let (slug, version) = skill
        .split_once('@')
        .map_or((skill, None), |(slug, version)| {
            (slug, Some(version.to_string()))
        });
    if catalog_id.is_empty() || slug.is_empty() {
        bail!("Catalog来源格式无效：{reference}");
    }
    Ok((catalog_id, slug, version))
}

fn validate_network_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("无效URL：{value}"))?;
    let allowed = match url.scheme() {
        "https" => true,
        "http" => url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1")),
        _ => false,
    };
    if !allowed {
        bail!("Catalog/Skill URL只允许HTTPS或本机HTTP：{value}");
    }
    Ok(url)
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .context("无法创建Catalog HTTP客户端")
}

fn config_path(paths: &WorkstationPaths) -> PathBuf {
    paths.skills_config.join("catalogs.json")
}

fn catalog_cache_path(paths: &WorkstationPaths, id: &str) -> PathBuf {
    paths
        .skills_catalog_cache
        .join(format!("{}.json", safe_component(id)))
}

fn safe_component(value: &str) -> String {
    let filtered = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(80)
        .collect::<String>();
    if filtered.is_empty() {
        "catalog".to_string()
    } else {
        filtered
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn signed_catalog_verifies_and_tampering_fails() {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let source = CatalogSource {
            id: "yunshu".to_string(),
            url: "https://example.com/catalog.json".to_string(),
            public_key_base64: STANDARD.encode(signing.verifying_key().to_bytes()),
            enabled: true,
        };
        let catalog = CatalogDocument {
            id: "yunshu".to_string(),
            generated_at: "2026-07-29T00:00:00Z".to_string(),
            skills: Vec::new(),
        };
        let signature = signing.sign(&serde_json::to_vec(&catalog).unwrap());
        let mut envelope = SignedCatalogEnvelope {
            schema_version: 1,
            catalog,
            signature_base64: STANDARD.encode(signature.to_bytes()),
        };
        verify_envelope(&source, &envelope).unwrap();
        envelope.catalog.generated_at = "tampered".to_string();
        assert!(verify_envelope(&source, &envelope).is_err());
    }
}
