use std::collections::BTreeMap;
use std::fs;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, anyhow, bail};
use getrandom::fill;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::paths::WorkstationPaths;
use crate::windows_process::replace_file_atomic;

const VAULT_MAGIC: &[u8; 4] = b"YSV1";
const VAULT_AAD: &[u8] = b"yunshu-dev2-provider-vault-v1";
const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultPayload {
    schema_version: u32,
    secrets: BTreeMap<String, String>,
}

pub struct PortableVault {
    key: Zeroizing<Vec<u8>>,
    payload: VaultPayload,
}

impl PortableVault {
    pub fn open(paths: &WorkstationPaths) -> Result<Self> {
        fs::create_dir_all(&paths.secrets)
            .with_context(|| format!("无法创建凭据目录 {}", paths.secrets.display()))?;
        let key = load_or_create_key(paths)?;
        let payload = if paths.vault_file().is_file() {
            decrypt_payload(paths, &key)?
        } else {
            VaultPayload {
                schema_version: 1,
                secrets: BTreeMap::new(),
            }
        };
        Ok(Self { key, payload })
    }

    pub fn contains(&self, reference: &str) -> bool {
        self.payload.secrets.contains_key(reference)
    }

    pub fn get(&self, reference: &str) -> Result<Zeroizing<String>> {
        self.payload
            .secrets
            .get(reference)
            .cloned()
            .map(Zeroizing::new)
            .ok_or_else(|| anyhow!("凭据库中缺少引用：{reference}"))
    }

    pub fn set(&mut self, reference: impl Into<String>, secret: Zeroizing<String>) -> Result<()> {
        let reference = reference.into();
        if reference.trim().is_empty() {
            bail!("凭据引用不能为空");
        }
        if secret.trim().is_empty() {
            bail!("API Key不能为空");
        }
        self.payload.secrets.insert(reference, secret.to_string());
        Ok(())
    }

    pub fn remove(&mut self, reference: &str) -> bool {
        if let Some(mut secret) = self.payload.secrets.remove(reference) {
            secret.zeroize();
            true
        } else {
            false
        }
    }

    pub fn retain_only(&mut self, references: &[&str]) -> bool {
        let mut changed = false;
        self.payload.secrets.retain(|reference, secret| {
            let keep = references.iter().any(|candidate| candidate == reference);
            if !keep {
                secret.zeroize();
                changed = true;
            }
            keep
        });
        changed
    }

    pub fn save(&self, paths: &WorkstationPaths) -> Result<()> {
        let plaintext = Zeroizing::new(serde_json::to_vec(&self.payload)?);
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| anyhow!("凭据库密钥长度不正确"))?;
        let mut nonce_bytes = [0_u8; NONCE_LENGTH];
        fill(&mut nonce_bytes).map_err(|error| anyhow!("无法生成凭据库随机数：{error}"))?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &plaintext,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|_| anyhow!("无法加密凭据库"))?;

        let mut bytes = Vec::with_capacity(VAULT_MAGIC.len() + NONCE_LENGTH + ciphertext.len());
        bytes.extend_from_slice(VAULT_MAGIC);
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext);
        write_atomic(&paths.vault_file(), &bytes)
    }
}

fn load_or_create_key(paths: &WorkstationPaths) -> Result<Zeroizing<Vec<u8>>> {
    let key_path = paths.vault_key_file();
    if key_path.is_file() {
        let key = fs::read(&key_path)
            .with_context(|| format!("无法读取凭据库密钥 {}", key_path.display()))?;
        if key.len() != KEY_LENGTH {
            bail!(
                "凭据库密钥长度异常：应为{KEY_LENGTH}字节，实际{}字节",
                key.len()
            );
        }
        return Ok(Zeroizing::new(key));
    }

    let mut key = vec![0_u8; KEY_LENGTH];
    fill(&mut key).map_err(|error| anyhow!("无法生成凭据库密钥：{error}"))?;
    write_atomic(&key_path, &key)?;
    Ok(Zeroizing::new(key))
}

fn decrypt_payload(paths: &WorkstationPaths, key: &[u8]) -> Result<VaultPayload> {
    let bytes = Zeroizing::new(
        fs::read(paths.vault_file())
            .with_context(|| format!("无法读取凭据库 {}", paths.vault_file().display()))?,
    );
    if bytes.len() < VAULT_MAGIC.len() + NONCE_LENGTH || &bytes[..VAULT_MAGIC.len()] != VAULT_MAGIC
    {
        bail!("凭据库格式或版本不正确");
    }
    let nonce_start = VAULT_MAGIC.len();
    let cipher_start = nonce_start + NONCE_LENGTH;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("凭据库密钥长度不正确"))?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&bytes[nonce_start..cipher_start]),
                Payload {
                    msg: &bytes[cipher_start..],
                    aad: VAULT_AAD,
                },
            )
            .map_err(|_| anyhow!("凭据库无法解密，密钥可能不匹配或文件已损坏"))?,
    );
    serde_json::from_slice(&plaintext).context("凭据库内容无法解析")
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入临时文件 {}", temporary.display()))?;
    replace_file_atomic(&temporary, path)
}

impl Drop for PortableVault {
    fn drop(&mut self) {
        for secret in self.payload.secrets.values_mut() {
            secret.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn vault_encrypts_and_round_trips_without_plaintext() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 dev2"));
        paths.prepare_layout().unwrap();

        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set(
                "provider-codex",
                Zeroizing::new("sk-private-test-value".to_string()),
            )
            .unwrap();
        vault.save(&paths).unwrap();
        drop(vault);

        let bytes = fs::read(paths.vault_file()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-private-test-value"));
        let reopened = PortableVault::open(&paths).unwrap();
        assert_eq!(
            reopened.get("provider-codex").unwrap().as_str(),
            "sk-private-test-value"
        );
    }

    #[test]
    fn vault_removes_secret_and_persists_removal() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        paths.prepare_layout().unwrap();

        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set("temporary-proxy", Zeroizing::new("secret".to_string()))
            .unwrap();
        vault.save(&paths).unwrap();
        assert!(vault.remove("temporary-proxy"));
        vault.save(&paths).unwrap();

        assert!(
            !PortableVault::open(&paths)
                .unwrap()
                .contains("temporary-proxy")
        );
    }

    #[test]
    fn vault_retain_only_preserves_license_private_key() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        paths.prepare_layout().unwrap();

        let mut vault = PortableVault::open(&paths).unwrap();
        vault
            .set(
                "license-installation-private-key",
                Zeroizing::new("license-key".to_string()),
            )
            .unwrap();
        vault
            .set("provider-codex", Zeroizing::new("provider-key".to_string()))
            .unwrap();
        assert!(vault.retain_only(&["license-installation-private-key"]));
        vault.save(&paths).unwrap();

        let reopened = PortableVault::open(&paths).unwrap();
        assert!(reopened.contains("license-installation-private-key"));
        assert!(!reopened.contains("provider-codex"));
    }
}
