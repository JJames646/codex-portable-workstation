use crate::footer::{FOOTER_SIZE, InstallerFooterV1};
use crate::manifest::{ReleaseManifest, sha256_file};
use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use getrandom::fill as fill_random;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Clone)]
pub struct VerifiedInstaller {
    pub path: PathBuf,
    pub footer: InstallerFooterV1,
    pub manifest: ReleaseManifest,
}

pub fn load_signing_key(path: &Path) -> Result<SigningKey> {
    let bytes = fs::read(path).with_context(|| format!("无法读取签名私钥: {}", path.display()))?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("签名私钥长度必须为32字节"))?;
    Ok(SigningKey::from_bytes(&key))
}

pub fn load_or_create_signing_key(path: &Path) -> Result<SigningKey> {
    if path.exists() {
        return load_signing_key(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut key = [0_u8; 32];
    fill_random(&mut key).map_err(|error| anyhow::anyhow!("生成签名私钥失败: {error}"))?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("无法创建签名私钥: {}", path.display()))?;
    file.write_all(&key)?;
    file.sync_all()?;
    Ok(SigningKey::from_bytes(&key))
}

pub fn build_single_exe_installer(
    master_root: &Path,
    manifest: &ReleaseManifest,
    installer_stub: &Path,
    output: &Path,
    signing_key: &SigningKey,
) -> Result<VerifiedInstaller> {
    if output.exists() {
        bail!("输出安装包已存在，拒绝覆盖: {}", output.display());
    }
    let parent = output.parent().context("输出路径缺少父目录")?;
    fs::create_dir_all(parent)?;
    let payload = parent.join(format!(
        ".{}.payload.partial",
        output.file_name().unwrap_or_default().to_string_lossy()
    ));
    if payload.exists() {
        bail!("存在未完成的载荷临时文件: {}", payload.display());
    }

    let result = (|| -> Result<VerifiedInstaller> {
        build_zip_payload(master_root, manifest, &payload)?;
        let payload_sha256 = sha256_file(&payload)?;
        let mut signed_manifest = manifest.clone();
        signed_manifest.payload_sha256 = Some(payload_sha256.clone());
        let manifest_bytes = serde_json::to_vec(&signed_manifest)?;
        let manifest_hash = Sha256::digest(&manifest_bytes);
        let signature = signing_key.sign(&manifest_bytes);

        let mut writer = BufWriter::new(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(output)
                .with_context(|| format!("无法创建安装包: {}", output.display()))?,
        );
        let mut stub_reader =
            BufReader::new(File::open(installer_stub).with_context(|| {
                format!("无法读取用户安装器模板: {}", installer_stub.display())
            })?);
        let stub_length = std::io::copy(&mut stub_reader, &mut writer)?;

        let mut payload_reader = BufReader::new(File::open(&payload)?);
        let payload_length = std::io::copy(&mut payload_reader, &mut writer)?;
        let manifest_offset = stub_length
            .checked_add(payload_length)
            .context("安装包尺寸溢出")?;
        writer.write_all(&manifest_bytes)?;
        let signature_offset = manifest_offset
            .checked_add(manifest_bytes.len() as u64)
            .context("安装包尺寸溢出")?;
        writer.write_all(&signature.to_bytes())?;

        let footer = InstallerFooterV1 {
            schema_version: 1,
            payload_offset: stub_length,
            payload_length,
            manifest_offset,
            manifest_length: manifest_bytes.len() as u64,
            signature_offset,
            signature_length: 64,
            payload_sha256: decode_hash(&payload_sha256)?,
            manifest_sha256: manifest_hash.into(),
            public_key: signing_key.verifying_key().to_bytes(),
        };
        writer.write_all(&footer.encode())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        verify_single_exe_installer_deep_with_public_key(
            output,
            &signing_key.verifying_key().to_bytes(),
        )
    })();

    let _ = fs::remove_file(&payload);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

fn build_zip_payload(master_root: &Path, manifest: &ReleaseManifest, output: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true)
        .unix_permissions(0o644);
    let mut buffer = vec![0_u8; 1024 * 1024];
    for item in &manifest.files {
        let relative = crate::manifest::manifest_path_to_path(&item.source)?;
        let path = master_root.join(relative);
        zip.start_file(&item.source, options)?;
        let mut source = BufReader::new(File::open(&path)?);
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            zip.write_all(&buffer[..read])?;
        }
    }
    let mut output = zip.finish()?;
    output.flush()?;
    output.get_ref().sync_all()?;
    Ok(())
}

pub fn verify_single_exe_installer(path: &Path) -> Result<VerifiedInstaller> {
    verify_single_exe_installer_inner(path, None, false)
}

pub fn verify_single_exe_installer_with_public_key(
    path: &Path,
    expected_public_key: &[u8; 32],
) -> Result<VerifiedInstaller> {
    verify_single_exe_installer_inner(path, Some(expected_public_key), false)
}

pub fn verify_single_exe_installer_deep(path: &Path) -> Result<VerifiedInstaller> {
    verify_single_exe_installer_inner(path, None, true)
}

pub fn verify_single_exe_installer_deep_with_public_key(
    path: &Path,
    expected_public_key: &[u8; 32],
) -> Result<VerifiedInstaller> {
    verify_single_exe_installer_inner(path, Some(expected_public_key), true)
}

pub fn public_key_from_hex(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value.trim()).context("可信公钥不是有效的十六进制")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("可信公钥长度必须为32字节"))
}

fn verify_single_exe_installer_inner(
    path: &Path,
    expected_public_key: Option<&[u8; 32]>,
    deep: bool,
) -> Result<VerifiedInstaller> {
    let mut file =
        File::open(path).with_context(|| format!("无法读取安装包: {}", path.display()))?;
    let file_length = file.metadata()?.len();
    if file_length < FOOTER_SIZE as u64 {
        bail!("安装包被截断，缺少尾部索引");
    }
    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
    let mut footer_bytes = [0_u8; FOOTER_SIZE];
    file.read_exact(&mut footer_bytes)?;
    let footer = InstallerFooterV1::decode(&footer_bytes)?;
    footer.validate_bounds(file_length)?;
    if expected_public_key.is_some_and(|expected| expected != &footer.public_key) {
        bail!("安装包签名公钥与安装器内置可信公钥不一致");
    }

    let payload_hash = hash_file_range(&mut file, footer.payload_offset, footer.payload_length)?;
    if payload_hash != footer.payload_sha256 {
        bail!("安装包载荷SHA-256校验失败");
    }
    let manifest_bytes = read_file_range(
        &mut file,
        footer.manifest_offset,
        footer.manifest_length,
        64 * 1024 * 1024,
    )?;
    let actual_manifest_hash: [u8; 32] = Sha256::digest(&manifest_bytes).into();
    if actual_manifest_hash != footer.manifest_sha256 {
        bail!("安装包清单SHA-256校验失败");
    }
    let signature_bytes = read_file_range(
        &mut file,
        footer.signature_offset,
        u64::from(footer.signature_length),
        64,
    )?;
    let signature = Signature::from_slice(&signature_bytes).context("签名格式无效")?;
    let verifying_key = VerifyingKey::from_bytes(&footer.public_key).context("签名公钥无效")?;
    verifying_key
        .verify(&manifest_bytes, &signature)
        .context("安装包Ed25519签名校验失败")?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&manifest_bytes).context("安装包清单格式无效")?;
    if !matches!(manifest.schema_version, 1 | 2) {
        bail!(
            "不支持的发布清单 schemaVersion: {}",
            manifest.schema_version
        );
    }
    if manifest.payload_sha256.as_deref() != Some(hex::encode(payload_hash).as_str()) {
        bail!("安装包清单中的载荷哈希不一致");
    }

    // 在验签之后再解析ZIP目录，确保损坏或篡改载荷不会进入安装流程。
    let slice = BoundedFile::new(
        File::open(path)?,
        footer.payload_offset,
        footer.payload_length,
    )?;
    let mut archive = ZipArchive::new(slice).context("安装包ZIP64载荷损坏")?;
    verify_archive_entries(&mut archive, &manifest, deep)?;

    Ok(VerifiedInstaller {
        path: path.to_path_buf(),
        footer,
        manifest,
    })
}

fn verify_archive_entries(
    archive: &mut ZipArchive<BoundedFile>,
    manifest: &ReleaseManifest,
    deep: bool,
) -> Result<()> {
    if archive.len() != manifest.files.len() {
        bail!("安装包ZIP文件数量与清单不一致");
    }
    let mut expected = HashMap::with_capacity(manifest.files.len());
    for item in &manifest.files {
        if expected.insert(item.source.as_str(), item).is_some() {
            bail!("安装包清单包含重复来源路径: {}", item.source);
        }
    }

    let mut seen = HashSet::with_capacity(archive.len());
    let mut buffer = vec![0_u8; 1024 * 1024];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("无法读取ZIP条目 #{index}"))?;
        if entry.is_dir() {
            bail!("安装包ZIP包含未在清单中定义的目录条目: {}", entry.name());
        }
        let name = std::str::from_utf8(entry.name_raw())
            .context("ZIP条目名不是UTF-8")?
            .to_owned();
        if !seen.insert(name.clone()) {
            bail!("安装包ZIP包含重复条目: {name}");
        }
        let item = expected
            .get(name.as_str())
            .ok_or_else(|| anyhow::anyhow!("安装包ZIP包含额外条目: {name}"))?;
        if entry.size() != item.bytes {
            bail!("安装包ZIP条目长度与清单不一致: {name}");
        }

        if deep {
            let mut hasher = Sha256::new();
            loop {
                let read = entry
                    .read(&mut buffer)
                    .with_context(|| format!("无法解压并校验ZIP条目: {name}"))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(&item.sha256) {
                bail!("安装包ZIP条目SHA-256与清单不一致: {name}");
            }
        }
    }

    if seen.len() != expected.len() {
        let missing = expected
            .keys()
            .find(|name| !seen.contains(**name))
            .copied()
            .unwrap_or("未知条目");
        bail!("安装包ZIP缺少清单条目: {missing}");
    }
    Ok(())
}

pub(crate) fn open_payload_archive(
    installer: &VerifiedInstaller,
) -> Result<ZipArchive<BoundedFile>> {
    let file = File::open(&installer.path)?;
    let slice = BoundedFile::new(
        file,
        installer.footer.payload_offset,
        installer.footer.payload_length,
    )?;
    Ok(ZipArchive::new(slice)?)
}

fn decode_hash(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("SHA-256长度无效"))
}

fn hash_file_range(file: &mut File, offset: u64, length: u64) -> Result<[u8; 32]> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = length;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
        let read = file.read(&mut buffer[..requested])?;
        if read == 0 {
            bail!("安装包在哈希校验时意外结束");
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(hasher.finalize().into())
}

fn read_file_range(file: &mut File, offset: u64, length: u64, limit: u64) -> Result<Vec<u8>> {
    if length > limit {
        bail!("安装包索引区长度超过允许范围");
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; length as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(crate) struct BoundedFile {
    file: File,
    start: u64,
    length: u64,
    position: u64,
}

impl BoundedFile {
    fn new(file: File, start: u64, length: u64) -> Result<Self> {
        let end = start.checked_add(length).context("载荷范围溢出")?;
        if end > file.metadata()?.len() {
            bail!("载荷范围超出安装包长度");
        }
        Ok(Self {
            file,
            start,
            length,
            position: 0,
        })
    }
}

impl Read for BoundedFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position >= self.length {
            return Ok(0);
        }
        let remaining = self.length - self.position;
        let requested = buffer.len().min(remaining as usize);
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let read = self.file.read(&mut buffer[..requested])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for BoundedFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let target = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(value) => i128::from(self.position) + i128::from(value),
            SeekFrom::End(value) => i128::from(self.length) + i128::from(value),
        };
        if target < 0 || target > i128::from(self.length) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ZIP读取超出载荷边界",
            ));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileScope, ReleaseFile};
    use tempfile::tempdir;

    fn fixture_manifest(source: &Path) -> ReleaseManifest {
        let hash = sha256_file(source).unwrap();
        ReleaseManifest {
            schema_version: 2,
            product_version: "test".into(),
            build_id: "build".into(),
            generated_at: "now".into(),
            product_directory: "云枢AI便携工作站".into(),
            expanded_bytes: 7,
            file_count: 1,
            factory_file_count: 1626,
            payload_architecture: crate::manifest::PayloadArchitecture::X64,
            host_compatibility: crate::manifest::HostCompatibility {
                supported: vec!["windows-10-x64".into(), "windows-11-x64".into()],
                risk_trial: vec!["windows-11-arm64".into()],
                blocked: vec!["windows-10-arm64".into(), "windows-x86".into()],
            },
            payload_sha256: None,
            files: vec![ReleaseFile {
                source: "file.txt".into(),
                scope: FileScope::Product,
                relative_path: "file.txt".into(),
                bytes: 7,
                sha256: hash,
            }],
            root_mappings: vec![],
            directories: vec![],
        }
    }

    fn write_signed_test_installer(
        directory: &Path,
        manifest: &ReleaseManifest,
        entries: &[(&str, &[u8])],
        key: &SigningKey,
        name: &str,
    ) -> PathBuf {
        let payload = directory.join(format!("{name}.zip"));
        let mut zip = ZipWriter::new(File::create(&payload).unwrap());
        let options = SimpleFileOptions::default().large_file(true);
        for (entry_name, contents) in entries {
            zip.start_file(*entry_name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap().sync_all().unwrap();

        let payload_hash = sha256_file(&payload).unwrap();
        let mut signed_manifest = manifest.clone();
        signed_manifest.payload_sha256 = Some(payload_hash.clone());
        let manifest_bytes = serde_json::to_vec(&signed_manifest).unwrap();
        let signature = key.sign(&manifest_bytes);
        let output = directory.join(format!("{name}.exe"));
        let mut writer = BufWriter::new(File::create(&output).unwrap());
        writer.write_all(b"MZ-test-stub").unwrap();
        let payload_offset = 12_u64;
        let payload_bytes = fs::read(&payload).unwrap();
        writer.write_all(&payload_bytes).unwrap();
        let manifest_offset = payload_offset + payload_bytes.len() as u64;
        writer.write_all(&manifest_bytes).unwrap();
        let signature_offset = manifest_offset + manifest_bytes.len() as u64;
        writer.write_all(&signature.to_bytes()).unwrap();
        writer
            .write_all(
                &InstallerFooterV1 {
                    schema_version: 1,
                    payload_offset,
                    payload_length: payload_bytes.len() as u64,
                    manifest_offset,
                    manifest_length: manifest_bytes.len() as u64,
                    signature_offset,
                    signature_length: 64,
                    payload_sha256: decode_hash(&payload_hash).unwrap(),
                    manifest_sha256: Sha256::digest(&manifest_bytes).into(),
                    public_key: key.verifying_key().to_bytes(),
                }
                .encode(),
            )
            .unwrap();
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        output
    }

    #[test]
    fn builds_verifies_and_rejects_tampering() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"payload").unwrap();
        fs::write(dir.path().join("stub.exe"), b"MZ-test-stub").unwrap();
        let manifest = fixture_manifest(&dir.path().join("file.txt"));
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let output = dir.path().join("installer.exe");
        build_single_exe_installer(
            dir.path(),
            &manifest,
            &dir.path().join("stub.exe"),
            &output,
            &key,
        )
        .unwrap();
        assert_eq!(
            verify_single_exe_installer(&output)
                .unwrap()
                .manifest
                .file_count,
            1
        );
        assert!(
            verify_single_exe_installer_with_public_key(&output, &key.verifying_key().to_bytes())
                .is_ok()
        );
        assert!(
            verify_single_exe_installer_deep_with_public_key(
                &output,
                &key.verifying_key().to_bytes(),
            )
            .is_ok()
        );
        assert!(verify_single_exe_installer_with_public_key(&output, &[8_u8; 32]).is_err());

        let mut bytes = fs::read(&output).unwrap();
        bytes[16] ^= 0x7f;
        fs::write(&output, bytes).unwrap();
        assert!(verify_single_exe_installer(&output).is_err());
    }

    #[test]
    fn rejects_truncated_installer() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short.exe");
        fs::write(&path, b"MZ").unwrap();
        assert!(verify_single_exe_installer(&path).is_err());
    }

    #[test]
    fn rejects_payload_file_changed_after_manifest_was_built() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("file.txt");
        fs::write(&source, b"payload").unwrap();
        fs::write(dir.path().join("stub.exe"), b"MZ-test-stub").unwrap();
        let manifest = fixture_manifest(&source);
        fs::write(&source, b"changed").unwrap();
        let output = dir.path().join("installer.exe");
        assert!(
            build_single_exe_installer(
                dir.path(),
                &manifest,
                &dir.path().join("stub.exe"),
                &output,
                &SigningKey::from_bytes(&[10_u8; 32]),
            )
            .is_err()
        );
        assert!(!output.exists());
    }

    #[test]
    fn rejects_missing_extra_and_duplicate_zip_entries() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("file.txt");
        fs::write(&source, b"payload").unwrap();
        let manifest = fixture_manifest(&source);
        let key = SigningKey::from_bytes(&[11_u8; 32]);

        let missing = write_signed_test_installer(dir.path(), &manifest, &[], &key, "missing");
        assert!(verify_single_exe_installer(&missing).is_err());

        let extra = write_signed_test_installer(
            dir.path(),
            &manifest,
            &[("file.txt", b"payload"), ("extra.txt", b"extra")],
            &key,
            "extra",
        );
        assert!(verify_single_exe_installer(&extra).is_err());

        let mut duplicate_manifest = manifest.clone();
        duplicate_manifest.file_count = 2;
        duplicate_manifest.expanded_bytes = 14;
        duplicate_manifest.files.push(ReleaseFile {
            source: "file.txt".into(),
            scope: FileScope::Product,
            relative_path: "other.txt".into(),
            bytes: 7,
            sha256: manifest.files[0].sha256.clone(),
        });
        let duplicate = write_signed_test_installer(
            dir.path(),
            &duplicate_manifest,
            &[("file.txt", b"payload"), ("other.txt", b"payload")],
            &key,
            "duplicate",
        );
        assert!(verify_single_exe_installer(&duplicate).is_err());
    }

    #[test]
    fn fast_verification_skips_entry_hash_but_deep_verification_rejects_it() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("file.txt");
        fs::write(&source, b"payload").unwrap();
        let manifest = fixture_manifest(&source);
        let key = SigningKey::from_bytes(&[12_u8; 32]);
        let installer = write_signed_test_installer(
            dir.path(),
            &manifest,
            &[("file.txt", b"changed")],
            &key,
            "same-length-content-change",
        );

        assert!(verify_single_exe_installer(&installer).is_ok());
        assert!(verify_single_exe_installer_deep(&installer).is_err());
    }

    #[test]
    fn builds_and_verifies_on_a_small_windows_sized_thread_stack() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let dir = tempdir().unwrap();
                fs::write(dir.path().join("file.txt"), b"payload").unwrap();
                fs::write(dir.path().join("stub.exe"), b"MZ-test-stub").unwrap();
                let manifest = fixture_manifest(&dir.path().join("file.txt"));
                let output = dir.path().join("installer.exe");
                build_single_exe_installer(
                    dir.path(),
                    &manifest,
                    &dir.path().join("stub.exe"),
                    &output,
                    &SigningKey::from_bytes(&[9_u8; 32]),
                )
                .unwrap();
                verify_single_exe_installer(&output).unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
