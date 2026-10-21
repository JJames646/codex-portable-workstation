use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

const MAX_ARCHIVE_FILES: usize = 4096;
const MAX_SINGLE_FILE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_UNCOMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHubPlatformKeys {
    pub keys: Vec<SkillHubPlatformKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHubPlatformKey {
    pub key_id: String,
    pub algorithm: String,
    pub public_key_raw_b64: String,
    pub status: String,
    pub issuer: String,
    pub activated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHubSignatureEnvelope {
    pub signed: bool,
    pub payload: String,
    pub signature: String,
    pub content_hash: String,
    pub key_id: String,
    pub hash_version: u32,
    pub signed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHubSignaturePayload {
    pub content_hash: String,
    pub file_count: usize,
    pub issuer: String,
    pub skill_slug: String,
    pub skill_version: String,
    pub v: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedSkillHubSignature {
    pub content_hash: String,
    pub file_count: usize,
    pub key_id: String,
    pub signed_at: u64,
    pub verified: bool,
}

pub fn verify_package(
    package: &Path,
    expected_slug: &str,
    expected_version: &str,
    envelope: &SkillHubSignatureEnvelope,
    platform_keys: &SkillHubPlatformKeys,
) -> Result<VerifiedSkillHubSignature> {
    if !envelope.signed {
        bail!("SkillHub包没有平台签名，已按安全策略拒绝");
    }
    if envelope.hash_version != 1 {
        bail!(
            "不支持的SkillHub content hash版本：{}",
            envelope.hash_version
        );
    }
    let key = platform_keys
        .keys
        .iter()
        .find(|key| key.key_id == envelope.key_id)
        .ok_or_else(|| anyhow!("SkillHub平台公钥中没有key_id={}", envelope.key_id))?;
    if !key.algorithm.eq_ignore_ascii_case("ed25519") {
        bail!("SkillHub平台公钥算法不是Ed25519：{}", key.algorithm);
    }
    if key.status.eq_ignore_ascii_case("revoked") {
        bail!("SkillHub签名公钥已被撤销：{}", key.key_id);
    }
    if key.issuer != "skillhub.cn" {
        bail!("SkillHub公钥issuer异常：{}", key.issuer);
    }

    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&key.public_key_raw_b64)
        .context("SkillHub平台公钥不是有效Base64")?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow!("SkillHub Ed25519公钥必须是32字节"))?;
    let verifying_key = VerifyingKey::from_bytes(&key_array).context("SkillHub Ed25519公钥无效")?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(&envelope.signature)
        .context("SkillHub签名不是有效Base64")?;
    let signature =
        Signature::from_slice(&signature_bytes).context("SkillHub签名长度不是64字节")?;
    verifying_key
        .verify(envelope.payload.as_bytes(), &signature)
        .context("SkillHub Ed25519签名验证失败")?;

    let payload: SkillHubSignaturePayload =
        serde_json::from_str(&envelope.payload).context("SkillHub签名payload不是有效JSON")?;
    if payload.v != 1 {
        bail!("不支持的SkillHub签名payload版本：{}", payload.v);
    }
    if payload.issuer != "skillhub.cn" {
        bail!("SkillHub签名issuer异常：{}", payload.issuer);
    }
    if payload.skill_slug != expected_slug || payload.skill_version != expected_version {
        bail!(
            "SkillHub签名坐标不匹配：expected={}@{} actual={}@{}",
            expected_slug,
            expected_version,
            payload.skill_slug,
            payload.skill_version
        );
    }
    if !payload
        .content_hash
        .eq_ignore_ascii_case(&envelope.content_hash)
    {
        bail!("SkillHub签名外层content_hash与payload不一致");
    }

    let (actual_hash, actual_count) = compute_package_content_hash(package)?;
    if actual_count != payload.file_count {
        bail!(
            "SkillHub包文件数量不匹配：expected={} actual={actual_count}",
            payload.file_count
        );
    }
    if !actual_hash.eq_ignore_ascii_case(&payload.content_hash) {
        bail!(
            "SkillHub包内容哈希不匹配：expected={} actual={actual_hash}",
            payload.content_hash
        );
    }
    Ok(VerifiedSkillHubSignature {
        content_hash: actual_hash,
        file_count: actual_count,
        key_id: envelope.key_id.clone(),
        signed_at: envelope.signed_at,
        verified: true,
    })
}

pub fn compute_package_content_hash(package: &Path) -> Result<(String, usize)> {
    let file = File::open(package)
        .with_context(|| format!("无法打开SkillHub ZIP {}", package.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("无法解析SkillHub ZIP {}", package.display()))?;
    if archive.len() > MAX_ARCHIVE_FILES {
        bail!("SkillHub ZIP文件数量超过限制：{}", archive.len());
    }

    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("SkillHub ZIP包含越界路径：{}", entry.name()))?;
        let relative = enclosed.to_string_lossy().replace('\\', "/");
        if official_hash_ignores(&relative) {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("SkillHub ZIP包含符号链接：{relative}");
        }
        if entry.size() > MAX_SINGLE_FILE_SIZE {
            bail!("SkillHub ZIP单文件超过64MB限制：{relative}");
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > MAX_TOTAL_UNCOMPRESSED_SIZE {
            bail!("SkillHub ZIP解压总大小超过256MB限制");
        }
        if !seen.insert(relative.clone()) {
            bail!("SkillHub ZIP包含重复路径：{relative}");
        }
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        entries.push((relative, lower_hex(&digest.finalize())));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut manifest = Sha256::new();
    for (relative, hash) in &entries {
        manifest.update(relative.as_bytes());
        manifest.update(b":");
        manifest.update(hash.as_bytes());
        manifest.update(b"\n");
    }
    Ok((lower_hex(&manifest.finalize()), entries.len()))
}

fn official_hash_ignores(relative: &str) -> bool {
    let mut components = relative.split('/');
    components.any(|component| {
        component == "__MACOSX"
            || component == ".DS_Store"
            || component == "Thumbs.db"
            || component == "_meta.json"
            || component.starts_with("._")
    })
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use base64::engine::general_purpose::STANDARD;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn package_hash_excludes_skillhub_metadata_and_signature_verifies() {
        let directory = tempdir().unwrap();
        let package = directory.path().join("signed.zip");
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("SKILL.md", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"---\nname: signed-test\n---\n").unwrap();
        writer
            .start_file("_meta.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"{\"ignored\":true}").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        std::fs::write(&package, bytes).unwrap();

        let (content_hash, file_count) = compute_package_content_hash(&package).unwrap();
        assert_eq!(file_count, 1);
        let signing = SigningKey::from_bytes(&[9_u8; 32]);
        let payload = serde_json::json!({
            "content_hash": content_hash,
            "file_count": file_count,
            "issuer": "skillhub.cn",
            "skill_slug": "signed-test",
            "skill_version": "1.0.0",
            "v": 1
        })
        .to_string();
        let envelope = SkillHubSignatureEnvelope {
            signed: true,
            signature: STANDARD.encode(signing.sign(payload.as_bytes()).to_bytes()),
            payload,
            content_hash,
            key_id: "test-key".to_string(),
            hash_version: 1,
            signed_at: 1,
        };
        let keys = SkillHubPlatformKeys {
            keys: vec![SkillHubPlatformKey {
                key_id: "test-key".to_string(),
                algorithm: "Ed25519".to_string(),
                public_key_raw_b64: STANDARD.encode(signing.verifying_key().to_bytes()),
                status: "active".to_string(),
                issuer: "skillhub.cn".to_string(),
                activated_at: 1,
            }],
        };
        assert!(
            verify_package(&package, "signed-test", "1.0.0", &envelope, &keys)
                .unwrap()
                .verified
        );

        let tampered_package = directory.path().join("tampered.zip");
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("SKILL.md", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(b"---\nname: signed-test\n---\ntampered\n")
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        std::fs::write(&tampered_package, bytes).unwrap();
        assert!(
            verify_package(&tampered_package, "signed-test", "1.0.0", &envelope, &keys).is_err()
        );
    }
}
