use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;
use zeroize::Zeroizing;

use crate::locale::{CODEX_LOCALE, CodexLocaleStatus, CodexNativeI18nState};
use crate::native_i18n::{self, NATIVE_I18N_PAYLOAD};
use crate::paths::WorkstationPaths;
use crate::vault::PortableVault;
use crate::windows_process::replace_file_atomic;

const SEED_SCHEMA_VERSION: u32 = 2;
const SEED_MANIFEST: &str = "manifest.json";
const NATIVE_I18N_APPLIED_MARKER: &str = "codex-i18n-applied-v1.json";
const PROXY_SCHEMA_VERSION: u32 = 1;
const PROXY_CONFIG_FILE: &str = "codex-init-proxy.json";
const PROXY_MARKER_FILE: &str = "codex-init-proxy-once.json";
const PROXY_PASSWORD_REF: &str = "codex-init-proxy-password-v1";
const PROXY_MARKER_TTL_MS: u64 = 5 * 60 * 1000;
const SELECTED_LANGUAGES: &str = "zh-CN,zh";
const LOCAL_I18N_PACKAGES: [&str; 9] = [
    "classic-level",
    "abstract-level",
    "module-error",
    "napi-macros",
    "node-gyp-build",
    "is-buffer",
    "level-supports",
    "level-transcoder",
    "maybe-combine-errors",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocaleInitializationStatus {
    pub current: CodexLocaleStatus,
    pub seed: CodexLocaleSeedStatus,
    pub proxy: crate::codex_proxy::CodexProxyStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodexLocaleSeedState {
    Pending,
    Ready,
    Incomplete,
    VersionMismatch,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocaleSeedStatus {
    pub state: CodexLocaleSeedState,
    pub message: String,
    pub manifest_path: PathBuf,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocaleApplyResult {
    pub applied_files: usize,
    pub backup_path: Option<PathBuf>,
    pub restarted: bool,
    pub status: CodexLocaleInitializationStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocaleSeedManifest {
    schema_version: u32,
    codex_sha256: String,
    files: Vec<LocaleSeedFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocaleSeedFile {
    source: String,
    target_root: LocaleSeedTargetRoot,
    target: String,
    mode: LocaleSeedApplyMode,
    sha256: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LocaleSeedTargetRoot {
    CodexHome,
    BrowserData,
    RuntimeState,
}

impl LocaleSeedTargetRoot {
    fn resolve<'a>(self, paths: &'a WorkstationPaths) -> &'a Path {
        match self {
            Self::CodexHome => &paths.codex_home,
            Self::BrowserData => &paths.codex_browser_data,
            Self::RuntimeState => &paths.runtime_state,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::CodexHome => "codex-home",
            Self::BrowserData => "browser-data",
            Self::RuntimeState => "runtime-state",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LocaleSeedApplyMode {
    CopyFile,
    MergeBrowserPreferences,
    MergeCodexConfig,
    MergeBrowserLocalStorage,
}

#[derive(Debug)]
struct ResolvedSeedFile {
    source: PathBuf,
    target: PathBuf,
    target_root: LocaleSeedTargetRoot,
    relative_target: PathBuf,
    mode: LocaleSeedApplyMode,
}

#[derive(Debug)]
struct AppliedTarget {
    target: PathBuf,
    backup: Option<PathBuf>,
    was_directory: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocaleVerificationMarker {
    schema_version: u32,
    verified: bool,
    codex_sha256: String,
    seed_payload_sha256: String,
    native_i18n_state_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeI18nAppliedMarker {
    schema_version: u32,
    codex_sha256: String,
    native_i18n_state_sha256: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexInitProxyProtocol {
    Http,
    Https,
    Socks5,
}

impl CodexInitProxyProtocol {
    fn scheme(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInitProxySaveInput {
    pub protocol: CodexInitProxyProtocol,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredCodexInitProxyConfig {
    schema_version: u32,
    protocol: CodexInitProxyProtocol,
    host: String,
    port: u16,
    username: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInitProxyStatus {
    pub configured: bool,
    pub protocol: Option<CodexInitProxyProtocol>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub credential_present: bool,
    pub pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInitProxyTestResult {
    pub reachable: bool,
    pub latency_ms: u128,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyActivationMarker {
    schema_version: u32,
    expires_at_unix_ms: u64,
}

#[derive(Debug)]
pub struct CodexProxyLaunch {
    pub environment: Vec<(OsString, OsString)>,
    pub chromium_proxy_argument: OsString,
}

pub fn initialization_status(paths: &WorkstationPaths) -> Result<CodexLocaleInitializationStatus> {
    Ok(CodexLocaleInitializationStatus {
        current: crate::locale::codex_locale_status(paths)?,
        seed: inspect_seed(paths),
        proxy: crate::codex_proxy::status(paths)?,
    })
}

pub fn inspect_seed(paths: &WorkstationPaths) -> CodexLocaleSeedStatus {
    let manifest_path = seed_root(paths).join(SEED_MANIFEST);
    if !manifest_path.is_file() {
        return seed_status(
            CodexLocaleSeedState::Pending,
            "中文缓存待固化".to_string(),
            manifest_path,
            0,
        );
    }
    match load_ready_seed(paths) {
        Ok((manifest, _)) => seed_status(
            CodexLocaleSeedState::Ready,
            "随盘中文缓存完整且版本匹配".to_string(),
            manifest_path,
            manifest.files.len(),
        ),
        Err(error) => {
            let message = error.to_string();
            let state = if message.starts_with("Codex版本不匹配") {
                CodexLocaleSeedState::VersionMismatch
            } else {
                CodexLocaleSeedState::Incomplete
            };
            seed_status(state, message, manifest_path, 0)
        }
    }
}

pub fn native_i18n_state(paths: &WorkstationPaths) -> CodexNativeI18nState {
    let (manifest, resolved) = match load_ready_seed(paths) {
        Ok(value) => value,
        Err(error) if error.to_string().starts_with("Codex版本不匹配") => {
            return CodexNativeI18nState::VersionMismatch;
        }
        Err(_) => return CodexNativeI18nState::Missing,
    };
    let Some(entry) = resolved
        .iter()
        .find(|entry| matches!(entry.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage))
    else {
        return CodexNativeI18nState::Missing;
    };
    if native_i18n::load_payload(&entry.source).is_err() {
        return CodexNativeI18nState::Missing;
    }
    let applied_path = paths.runtime_state.join(NATIVE_I18N_APPLIED_MARKER);
    let Ok(applied) = fs::read(&applied_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<NativeI18nAppliedMarker>(&bytes).ok())
        .ok_or(())
    else {
        return CodexNativeI18nState::Missing;
    };
    if applied.schema_version != 1
        || !applied
            .codex_sha256
            .eq_ignore_ascii_case(&manifest.codex_sha256)
        || !applied
            .native_i18n_state_sha256
            .eq_ignore_ascii_case(&sha256_file(&entry.source).unwrap_or_default())
    {
        return CodexNativeI18nState::Missing;
    }
    // LevelDB 在 Codex 运行时会被 Chromium 独占，状态页不能为了检查语言而争抢锁。
    // 此标记只会在 apply() 已对便携库和本机镜像逐项校验成功后原子写入。
    if verification_marker_valid(paths, &manifest, &entry.source).unwrap_or(false) {
        CodexNativeI18nState::Verified
    } else {
        CodexNativeI18nState::WrittenPendingVerification
    }
}

pub fn apply_seed(paths: &WorkstationPaths) -> Result<CodexLocaleApplyResult> {
    apply_seed_internal(paths, true, false)
}

/// 首次使用初始化只写入移动盘主数据；本机 NTFS 运行镜像由首次启动 Codex 时建立。
pub fn apply_seed_portable_only(paths: &WorkstationPaths) -> Result<CodexLocaleApplyResult> {
    apply_seed_internal(paths, false, true)
}

fn apply_seed_internal(
    paths: &WorkstationPaths,
    refresh_machine_mirror: bool,
    stage_blank_native_database: bool,
) -> Result<CodexLocaleApplyResult> {
    let (manifest, resolved) = load_ready_seed(paths)?;
    let backup_root = paths
        .backups
        .join("locale-init")
        .join(now_unix_ms().to_string());
    let mut applied = Vec::new();
    for entry in &resolved {
        if let Err(error) = backup_and_apply(
            paths,
            entry,
            &backup_root,
            &mut applied,
            stage_blank_native_database,
        ) {
            rollback_applied_targets(&applied)?;
            return Err(error);
        }
    }
    let native_entry = resolved
        .iter()
        .find(|entry| matches!(entry.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage))
        .context("中文缓存缺少原生中文状态")?;
    let applied_marker_path = paths.runtime_state.join(NATIVE_I18N_APPLIED_MARKER);
    let applied_marker_backup = if applied_marker_path.is_file() {
        let backup = backup_root
            .join("runtime-state")
            .join(NATIVE_I18N_APPLIED_MARKER);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&applied_marker_path, &backup)?;
        Some(backup)
    } else {
        None
    };
    applied.push(AppliedTarget {
        target: applied_marker_path.clone(),
        backup: applied_marker_backup,
        was_directory: false,
    });
    if let Err(error) = write_json_atomic(
        &applied_marker_path,
        &NativeI18nAppliedMarker {
            schema_version: 1,
            codex_sha256: manifest.codex_sha256.clone(),
            native_i18n_state_sha256: sha256_file(&native_entry.source)?,
        },
    ) {
        rollback_applied_targets(&applied)?;
        return Err(error);
    }
    if refresh_machine_mirror {
        for entry in &resolved {
            let Some(machine_entry) = machine_mirror_entry(paths, entry) else {
                continue;
            };
            let result = if matches!(entry.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage) {
                backup_and_refresh_machine_leveldb(
                    paths,
                    entry,
                    &machine_entry,
                    &backup_root,
                    &mut applied,
                )
            } else {
                backup_and_apply(paths, &machine_entry, &backup_root, &mut applied, false)
            };
            if let Err(error) = result {
                rollback_applied_targets(&applied)?;
                return Err(error);
            }
        }
    }
    Ok(CodexLocaleApplyResult {
        applied_files: manifest.files.len(),
        backup_path: backup_root.is_dir().then_some(backup_root),
        restarted: false,
        status: initialization_status(paths)?,
    })
}

fn backup_and_refresh_machine_leveldb(
    paths: &WorkstationPaths,
    portable_entry: &ResolvedSeedFile,
    machine_entry: &ResolvedSeedFile,
    backup_root: &Path,
    applied: &mut Vec<AppliedTarget>,
) -> Result<()> {
    let backup = if machine_entry.target.is_dir() {
        let backup = backup_root
            .join(machine_entry.target_root.slug())
            .join(&machine_entry.relative_target);
        copy_directory(&machine_entry.target, &backup)?;
        Some(backup)
    } else {
        None
    };
    applied.push(AppliedTarget {
        target: machine_entry.target.clone(),
        backup,
        was_directory: machine_entry.target.is_dir(),
    });
    remove_path_if_exists(&machine_entry.target)?;
    copy_directory(&portable_entry.target, &machine_entry.target)?;
    let payload = native_i18n::load_payload(&portable_entry.source)?;
    if !native_i18n::verify(paths, &machine_entry.target, &payload)? {
        bail!("Codex 本机运行镜像没有写入原生中文状态");
    }
    Ok(())
}

fn backup_and_apply(
    paths: &WorkstationPaths,
    entry: &ResolvedSeedFile,
    backup_root: &Path,
    applied: &mut Vec<AppliedTarget>,
    stage_blank_native_database: bool,
) -> Result<()> {
    let backup = if entry.target.exists() {
        let backup = backup_root
            .join(entry.target_root.slug())
            .join(&entry.relative_target);
        if entry.target.is_dir() {
            copy_directory(&entry.target, &backup)?;
        } else {
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&entry.target, &backup)?;
        }
        Some(backup)
    } else {
        None
    };
    applied.push(AppliedTarget {
        target: entry.target.clone(),
        backup,
        was_directory: entry.target.is_dir(),
    });
    if stage_blank_native_database
        && matches!(entry.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage)
        && target_is_missing_or_empty_directory(&entry.target)?
    {
        apply_native_i18n_via_local_stage(paths, entry)
    } else {
        apply_seed_file(paths, entry)
    }
}

fn target_is_missing_or_empty_directory(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    Ok(path.is_dir() && fs::read_dir(path)?.next().transpose()?.is_none())
}

/// 空白便携库在首次初始化时先于本机 NTFS 写入并关闭，再安全提交回移动盘。
/// 手动“本地中文初始化”和已有 LevelDB 仍继续使用原来的字段级合并路径。
fn apply_native_i18n_via_local_stage(
    paths: &WorkstationPaths,
    entry: &ResolvedSeedFile,
) -> Result<()> {
    let stage_root = paths
        .machine_cache_root()
        .join("first-use-locale-staging")
        .join(format!("{}-{}", now_unix_ms(), std::process::id()));
    let stage_database = stage_root.join("database");
    let local_runtime = stage_root.join("runtime");
    let target_name = entry
        .target
        .file_name()
        .context("原生中文状态目标路径不完整")?
        .to_string_lossy();
    let portable_temporary = entry
        .target
        .parent()
        .context("原生中文状态目标目录不完整")?
        .join(format!(
            ".{target_name}.yunshu-first-use-{}",
            std::process::id()
        ));
    if stage_root.exists() || portable_temporary.exists() {
        bail!("首次中文初始化暂存目录已存在，拒绝覆盖");
    }

    let result = (|| -> Result<()> {
        fs::create_dir_all(&stage_root)?;
        let payload = native_i18n::load_payload(&entry.source)?;
        let local_module_root = stage_local_i18n_runtime(paths, &local_runtime)?;
        native_i18n::apply_with_local_runtime(
            paths,
            &stage_database,
            &payload,
            &local_module_root,
            &local_runtime,
        )
        .context("无法在本机暂存目录写入 Codex 中文默认状态")?;
        copy_directory(&stage_database, &portable_temporary)?;
        if directory_fingerprint(&stage_database)? != directory_fingerprint(&portable_temporary)? {
            bail!("写回移动盘后的 Codex 中文默认状态文件校验失败");
        }
        remove_path_if_exists(&entry.target)?;
        fs::rename(&portable_temporary, &entry.target)
            .with_context(|| format!("无法提交 Codex 中文默认状态：{}", entry.target.display()))
    })();

    let stage_cleanup = remove_path_if_exists(&stage_root);
    let portable_cleanup = remove_path_if_exists(&portable_temporary);
    result.and(stage_cleanup).and(portable_cleanup)
}

fn machine_mirror_entry(
    paths: &WorkstationPaths,
    entry: &ResolvedSeedFile,
) -> Option<ResolvedSeedFile> {
    let target = match entry.target_root {
        LocaleSeedTargetRoot::CodexHome => paths.codex_machine_home().join(&entry.relative_target),
        LocaleSeedTargetRoot::BrowserData => paths
            .codex_machine_browser_data()
            .join(&entry.relative_target),
        LocaleSeedTargetRoot::RuntimeState => return None,
    };
    Some(ResolvedSeedFile {
        source: entry.source.clone(),
        target,
        target_root: entry.target_root,
        relative_target: PathBuf::from("machine-mirror").join(&entry.relative_target),
        mode: entry.mode,
    })
}

fn load_ready_seed(
    paths: &WorkstationPaths,
) -> Result<(LocaleSeedManifest, Vec<ResolvedSeedFile>)> {
    let root = seed_root(paths);
    let manifest_path = root.join(SEED_MANIFEST);
    let manifest: LocaleSeedManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("无法读取中文缓存清单 {}", manifest_path.display()))?,
    )
    .with_context(|| format!("中文缓存清单无法解析 {}", manifest_path.display()))?;
    if manifest.schema_version != SEED_SCHEMA_VERSION {
        bail!("中文缓存清单版本不完整，需要 schema v2 原生中文状态");
    }
    if manifest.files.is_empty() {
        bail!("中文缓存清单没有文件");
    }
    validate_sha256(&manifest.codex_sha256)?;
    if !paths.codex_desktop.is_file() {
        bail!("Codex主体不完整");
    }
    let actual_codex_sha256 = sha256_file(&paths.codex_desktop)?;
    if !actual_codex_sha256.eq_ignore_ascii_case(&manifest.codex_sha256) {
        bail!("Codex版本不匹配，不能套用当前中文缓存");
    }

    let mut targets = HashSet::new();
    let mut resolved = Vec::with_capacity(manifest.files.len());
    for item in &manifest.files {
        let source_relative = safe_relative_path(&item.source)?;
        let target_relative = safe_relative_path(&item.target)?;
        if !matches!(item.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage) {
            reject_sensitive_target(&target_relative)?;
        }
        let source = root.join(&source_relative);
        reject_symlink_path(&root, &source_relative)?;
        if !source.is_file() {
            bail!("中文缓存文件缺失：{}", item.source);
        }
        validate_sha256(&item.sha256)?;
        if !sha256_file(&source)?.eq_ignore_ascii_case(&item.sha256) {
            bail!("中文缓存文件校验失败：{}", item.source);
        }
        validate_mode_target(item.mode, item.target_root, &target_relative)?;
        let target_root = item.target_root.resolve(paths);
        reject_symlink_path(target_root, &target_relative)?;
        let target = target_root.join(&target_relative);
        let target_key = format!("{}:{}", item.target_root.slug(), target_relative.display());
        if !targets.insert(target_key) {
            bail!("中文缓存清单包含重复目标：{}", item.target);
        }
        resolved.push(ResolvedSeedFile {
            source,
            target,
            target_root: item.target_root,
            relative_target: target_relative,
            mode: item.mode,
        });
    }
    let native_entries = manifest
        .files
        .iter()
        .filter(|entry| matches!(entry.mode, LocaleSeedApplyMode::MergeBrowserLocalStorage))
        .collect::<Vec<_>>();
    if native_entries.len() != 1 || native_entries[0].source != NATIVE_I18N_PAYLOAD {
        bail!("中文缓存清单缺少唯一的原生中文状态载荷");
    }
    Ok((manifest, resolved))
}

fn apply_seed_file(paths: &WorkstationPaths, entry: &ResolvedSeedFile) -> Result<()> {
    match entry.mode {
        LocaleSeedApplyMode::CopyFile => {
            let bytes = fs::read(&entry.source)
                .with_context(|| format!("无法读取 {}", entry.source.display()))?;
            write_atomic(&entry.target, &bytes)
        }
        LocaleSeedApplyMode::MergeBrowserPreferences => {
            let source = read_json_object(&entry.source)?;
            let expected = source
                .get("intl")
                .and_then(Value::as_object)
                .and_then(|intl| intl.get("selected_languages"))
                .and_then(Value::as_str);
            if expected != Some(SELECTED_LANGUAGES) {
                bail!("中文缓存 Preferences 不包含预期语言配置");
            }
            let mut target = read_optional_json_object(&entry.target)?;
            let intl = target
                .entry("intl".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| anyhow!("Preferences 的 intl 不是对象"))?;
            intl.insert(
                "selected_languages".to_string(),
                Value::String(SELECTED_LANGUAGES.to_string()),
            );
            let mut bytes = serde_json::to_vec_pretty(&Value::Object(target))?;
            bytes.push(b'\n');
            write_atomic(&entry.target, &bytes)
        }
        LocaleSeedApplyMode::MergeCodexConfig => {
            let source = fs::read_to_string(&entry.source)
                .with_context(|| format!("无法读取 {}", entry.source.display()))?;
            let source_document = source
                .parse::<DocumentMut>()
                .context("中文缓存 config.toml 无法解析")?;
            let expected = source_document
                .get("desktop")
                .and_then(Item::as_table)
                .and_then(|desktop| desktop.get("localeOverride"))
                .and_then(Item::as_value)
                .and_then(toml_edit::Value::as_str);
            if expected != Some(CODEX_LOCALE) {
                bail!("中文缓存 config.toml 不包含预期语言配置");
            }
            let mut target = if entry.target.is_file() {
                fs::read_to_string(&entry.target)
                    .with_context(|| format!("无法读取 {}", entry.target.display()))?
                    .parse::<DocumentMut>()
                    .with_context(|| format!("无法解析 {}", entry.target.display()))?
            } else {
                DocumentMut::new()
            };
            if !target.as_table().contains_key("desktop") {
                target["desktop"] = Item::Table(Table::new());
            }
            let desktop = target["desktop"]
                .as_table_mut()
                .ok_or_else(|| anyhow!("config.toml 的 desktop 不是表结构"))?;
            desktop["localeOverride"] = value(CODEX_LOCALE);
            write_atomic(&entry.target, target.to_string().as_bytes())
        }
        LocaleSeedApplyMode::MergeBrowserLocalStorage => {
            let payload = native_i18n::load_payload(&entry.source)?;
            native_i18n::apply(paths, &entry.target, &payload)
        }
    }
}

fn rollback_applied_targets(applied: &[AppliedTarget]) -> Result<()> {
    for item in applied.iter().rev() {
        if let Some(backup) = &item.backup {
            remove_path_if_exists(&item.target)?;
            if item.was_directory || backup.is_dir() {
                copy_directory(backup, &item.target)?;
            } else {
                let bytes = fs::read(backup)
                    .with_context(|| format!("无法读取中文初始化恢复点 {}", backup.display()))?;
                write_atomic(&item.target, &bytes)?;
            }
        } else {
            remove_path_if_exists(&item.target)?;
        }
    }
    Ok(())
}

pub fn load_proxy_status(paths: &WorkstationPaths) -> Result<CodexInitProxyStatus> {
    let config = read_proxy_config(paths)?;
    let credential_present = if paths.vault_file().is_file() {
        PortableVault::open(paths)?.contains(PROXY_PASSWORD_REF)
    } else {
        false
    };
    let pending = valid_proxy_marker(paths)?.is_some();
    Ok(CodexInitProxyStatus {
        configured: config.is_some(),
        protocol: config.as_ref().map(|item| item.protocol),
        host: config.as_ref().map(|item| item.host.clone()),
        port: config.as_ref().map(|item| item.port),
        username: config.as_ref().and_then(|item| item.username.clone()),
        credential_present,
        pending,
    })
}

pub fn save_proxy_config(
    paths: &WorkstationPaths,
    input: CodexInitProxySaveInput,
) -> Result<CodexInitProxyStatus> {
    let host = validate_proxy_host(&input.host)?;
    if input.port == 0 {
        bail!("代理端口不正确");
    }
    let username = input
        .username
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let config = StoredCodexInitProxyConfig {
        schema_version: PROXY_SCHEMA_VERSION,
        protocol: input.protocol,
        host,
        port: input.port,
        username,
    };
    write_json_atomic(&proxy_config_path(paths), &config)?;
    if let Some(password) = input.password.filter(|value| !value.is_empty()) {
        let mut vault = PortableVault::open(paths)?;
        vault.set(PROXY_PASSWORD_REF, Zeroizing::new(password))?;
        vault.save(paths)?;
    }
    load_proxy_status(paths)
}

pub fn test_proxy_connection(paths: &WorkstationPaths) -> Result<CodexInitProxyTestResult> {
    let config = read_proxy_config(paths)?.context("请先保存 Codex 网络初始化代理")?;
    let started = Instant::now();
    let address = socket_address(&config.host, config.port);
    let addresses = address
        .to_socket_addrs()
        .with_context(|| format!("无法解析代理主机 {}", config.host))?;
    let mut last_error = None;
    for resolved in addresses {
        match TcpStream::connect_timeout(&resolved, Duration::from_secs(8)) {
            Ok(stream) => {
                drop(stream);
                return Ok(CodexInitProxyTestResult {
                    reachable: true,
                    latency_ms: started.elapsed().as_millis(),
                    message: "代理入口可连接".to_string(),
                });
            }
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error
        .map(|item| item.to_string())
        .unwrap_or_else(|| "未解析到可用地址".to_string());
    bail!("代理入口无法连接：{error}")
}

pub fn activate_proxy_for_next_codex_start(
    paths: &WorkstationPaths,
) -> Result<CodexInitProxyStatus> {
    read_proxy_config(paths)?.context("请先保存 Codex 网络初始化代理")?;
    let marker = ProxyActivationMarker {
        schema_version: PROXY_SCHEMA_VERSION,
        expires_at_unix_ms: now_unix_ms().saturating_add(PROXY_MARKER_TTL_MS),
    };
    write_json_atomic(&proxy_marker_path(paths), &marker)?;
    load_proxy_status(paths)
}

pub fn stop_proxy_use(paths: &WorkstationPaths) -> Result<CodexInitProxyStatus> {
    remove_file_if_exists(&proxy_marker_path(paths))?;
    load_proxy_status(paths)
}

pub fn clear_proxy_config(paths: &WorkstationPaths) -> Result<CodexInitProxyStatus> {
    remove_file_if_exists(&proxy_marker_path(paths))?;
    remove_file_if_exists(&proxy_config_path(paths))?;
    if paths.vault_file().is_file() {
        let mut vault = PortableVault::open(paths)?;
        if vault.remove(PROXY_PASSWORD_REF) {
            vault.save(paths)?;
        }
    }
    load_proxy_status(paths)
}

pub fn take_codex_proxy_launch(paths: &WorkstationPaths) -> Result<Option<CodexProxyLaunch>> {
    if valid_proxy_marker(paths)?.is_none() {
        return Ok(None);
    }
    remove_file_if_exists(&proxy_marker_path(paths))?;
    let config = read_proxy_config(paths)?.context("Codex网络初始化代理配置不存在")?;
    let password = if paths.vault_file().is_file() {
        let vault = PortableVault::open(paths)?;
        vault.get(PROXY_PASSWORD_REF).ok()
    } else {
        None
    };
    let proxy_url = proxy_url(&config, password.as_deref().map(String::as_str))?;
    let proxy_value = OsString::from(proxy_url.as_str());
    let no_proxy = OsString::from("127.0.0.1,localhost,::1");
    let environment = vec![
        (OsString::from("HTTP_PROXY"), proxy_value.clone()),
        (OsString::from("HTTPS_PROXY"), proxy_value.clone()),
        (OsString::from("ALL_PROXY"), proxy_value.clone()),
        (OsString::from("http_proxy"), proxy_value.clone()),
        (OsString::from("https_proxy"), proxy_value.clone()),
        (OsString::from("all_proxy"), proxy_value),
        (OsString::from("NO_PROXY"), no_proxy.clone()),
        (OsString::from("no_proxy"), no_proxy),
    ];
    Ok(Some(CodexProxyLaunch {
        environment,
        chromium_proxy_argument: OsString::from(format!(
            "--proxy-server={}://{}:{}",
            config.protocol.scheme(),
            display_proxy_host(&config.host),
            config.port
        )),
    }))
}

fn seed_root(paths: &WorkstationPaths) -> PathBuf {
    paths.root.join("Factory").join("Seeds").join("Codex")
}

fn seed_status(
    state: CodexLocaleSeedState,
    message: String,
    manifest_path: PathBuf,
    file_count: usize,
) -> CodexLocaleSeedStatus {
    CodexLocaleSeedStatus {
        state,
        message,
        manifest_path,
        file_count,
    }
}

fn validate_mode_target(
    mode: LocaleSeedApplyMode,
    target_root: LocaleSeedTargetRoot,
    target: &Path,
) -> Result<()> {
    let normalized = target
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    match mode {
        LocaleSeedApplyMode::MergeBrowserPreferences => {
            let allowed = [
                "default/preferences",
                "codex-browser-app/preferences",
                "default/partitions/codex-browser-app/preferences",
            ];
            if !matches!(target_root, LocaleSeedTargetRoot::BrowserData)
                || !allowed.contains(&normalized.as_str())
            {
                bail!("Preferences 合并目标不正确");
            }
        }
        LocaleSeedApplyMode::MergeCodexConfig => {
            if !matches!(target_root, LocaleSeedTargetRoot::CodexHome)
                || normalized != "config.toml"
            {
                bail!("Codex config.toml 合并目标不正确");
            }
        }
        LocaleSeedApplyMode::CopyFile => {
            let allowed = matches!(target_root, LocaleSeedTargetRoot::BrowserData)
                && normalized == "owl-feature-bootstrap-cache.json"
                || matches!(target_root, LocaleSeedTargetRoot::RuntimeState)
                    && normalized == "codex-i18n-verified-v1.json";
            if !allowed {
                bail!("中文缓存复制目标不在允许清单中");
            }
        }
        LocaleSeedApplyMode::MergeBrowserLocalStorage => {
            if !matches!(target_root, LocaleSeedTargetRoot::BrowserData)
                || normalized != "default/local storage/leveldb"
            {
                bail!("原生中文状态只能合并到 Codex Default Local Storage");
            }
        }
    }
    Ok(())
}

fn verification_marker_valid(
    paths: &WorkstationPaths,
    manifest: &LocaleSeedManifest,
    native_payload_path: &Path,
) -> Result<bool> {
    let path = paths.runtime_state.join("codex-i18n-verified-v1.json");
    if !path.is_file() {
        return Ok(false);
    }
    let marker: LocaleVerificationMarker = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("无法解析 {}", path.display()))?;
    let payload_hash = seed_payload_hash(paths, manifest)?;
    Ok(marker.schema_version == 1
        && marker.verified
        && marker
            .codex_sha256
            .eq_ignore_ascii_case(&manifest.codex_sha256)
        && marker
            .seed_payload_sha256
            .eq_ignore_ascii_case(&payload_hash)
        && marker
            .native_i18n_state_sha256
            .eq_ignore_ascii_case(&sha256_file(native_payload_path)?))
}

fn seed_payload_hash(paths: &WorkstationPaths, manifest: &LocaleSeedManifest) -> Result<String> {
    let root = seed_root(paths);
    let mut items = manifest
        .files
        .iter()
        .filter(|entry| entry.source != "codex-i18n-verified-v1.json")
        .map(|entry| format!("{}={}", entry.source, entry.sha256.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    items.sort();
    items.dedup();
    if items.is_empty() || !root.is_dir() {
        bail!("中文种子载荷为空");
    }
    let text = format!("{}\n", items.join("\n"));
    Ok(format!("{:x}", Sha256::digest(text.as_bytes())))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    let mut stack = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!("中文初始化恢复点不能包含符号链接");
            }
            let target = to.join(entry.file_name());
            if file_type.is_dir() {
                fs::create_dir_all(&target)?;
                stack.push((entry.path(), target));
            } else if file_type.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), target)?;
            }
        }
    }
    Ok(())
}

fn directory_fingerprint(root: &Path) -> Result<Vec<(PathBuf, u64, String)>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!("Codex 中文状态数据库不能包含符号链接");
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                files.push((
                    entry.path().strip_prefix(root)?.to_path_buf(),
                    entry.metadata()?.len(),
                    sha256_file(&entry.path())?,
                ));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn stage_local_i18n_runtime(paths: &WorkstationPaths, root: &Path) -> Result<PathBuf> {
    let source = paths
        .codex_desktop
        .parent()
        .context("Codex 主体路径不完整")?
        .join("resources/plugins/openai-bundled/plugins/browser/scripts/node_modules");
    let target = root.join("node_modules");
    for package in LOCAL_I18N_PACKAGES {
        let package_source = source.join(package);
        if !package_source.is_dir() {
            bail!("Codex 原生中文状态依赖缺失：{package}");
        }
        copy_directory(&package_source, &target.join(package))?;
    }
    Ok(target.join("classic-level"))
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("无法删除 {}", path.display()))?;
    } else if path.is_file() {
        fs::remove_file(path).with_context(|| format!("无法删除 {}", path.display()))?;
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    if value.trim().is_empty()
        || value.contains('\\')
        || value.contains(':')
        || value.starts_with('/')
    {
        bail!("中文缓存清单包含非法路径：{value}");
    }
    let path = PathBuf::from(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("中文缓存清单包含非法路径：{value}");
    }
    Ok(path)
}

fn reject_sensitive_target(path: &Path) -> Result<()> {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let forbidden = [
        "sessions",
        "history",
        "projects",
        "skills",
        "plugins",
        "auth.json",
        ".env",
        "login data",
        "cookies",
        "web data",
        "local storage",
        "indexeddb",
        "session storage",
        "service worker",
    ];
    if forbidden.iter().any(|item| {
        normalized == *item
            || normalized.starts_with(&format!("{item}/"))
            || normalized.ends_with(&format!("/{item}"))
            || normalized.contains(&format!("/{item}/"))
    }) {
        bail!("中文缓存清单试图覆盖用户数据：{}", path.display());
    }
    Ok(())
}

fn reject_symlink_path(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    if current.exists() && fs::symlink_metadata(&current)?.file_type().is_symlink() {
        bail!("中文缓存路径不能使用符号链接：{}", current.display());
    }
    for component in relative.components() {
        let Component::Normal(component) = component else {
            bail!("中文缓存路径不正确");
        };
        current.push(component);
        if current.exists() && fs::symlink_metadata(&current)?.file_type().is_symlink() {
            bail!("中文缓存路径不能使用符号链接：{}", current.display());
        }
    }
    Ok(())
}

fn proxy_config_path(paths: &WorkstationPaths) -> PathBuf {
    paths.config.join(PROXY_CONFIG_FILE)
}

fn proxy_marker_path(paths: &WorkstationPaths) -> PathBuf {
    paths.runtime_state.join(PROXY_MARKER_FILE)
}

fn read_proxy_config(paths: &WorkstationPaths) -> Result<Option<StoredCodexInitProxyConfig>> {
    let path = proxy_config_path(paths);
    if !path.is_file() {
        return Ok(None);
    }
    let config: StoredCodexInitProxyConfig = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?,
    )
    .with_context(|| format!("无法解析 {}", path.display()))?;
    if config.schema_version != PROXY_SCHEMA_VERSION {
        bail!("Codex网络初始化代理配置版本不支持");
    }
    validate_proxy_host(&config.host)?;
    if config.port == 0 {
        bail!("Codex网络初始化代理端口不正确");
    }
    Ok(Some(config))
}

fn valid_proxy_marker(paths: &WorkstationPaths) -> Result<Option<ProxyActivationMarker>> {
    let path = proxy_marker_path(paths);
    if !path.is_file() {
        return Ok(None);
    }
    let marker: ProxyActivationMarker = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?,
    )
    .with_context(|| format!("无法解析 {}", path.display()))?;
    if marker.schema_version != PROXY_SCHEMA_VERSION || marker.expires_at_unix_ms <= now_unix_ms() {
        remove_file_if_exists(&path)?;
        return Ok(None);
    }
    Ok(Some(marker))
}

fn validate_proxy_host(value: &str) -> Result<String> {
    let host = value.trim();
    if host.is_empty()
        || host.chars().any(char::is_whitespace)
        || host.contains('/')
        || host.contains('\\')
        || host.contains('@')
    {
        bail!("代理主机格式不正确");
    }
    let display = display_proxy_host(host);
    Url::parse(&format!("http://{display}/")).context("代理主机格式不正确")?;
    Ok(host.to_string())
}

fn display_proxy_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn socket_address(host: &str, port: u16) -> String {
    format!("{}:{port}", display_proxy_host(host))
}

fn proxy_url(config: &StoredCodexInitProxyConfig, password: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(&format!(
        "{}://{}:{}/",
        config.protocol.scheme(),
        display_proxy_host(&config.host),
        config.port
    ))
    .context("无法构造 Codex 网络初始化代理")?;
    if let Some(username) = config.username.as_deref() {
        url.set_username(username)
            .map_err(|_| anyhow!("代理用户名格式不正确"))?;
    }
    if let Some(password) = password {
        url.set_password(Some(password))
            .map_err(|_| anyhow!("代理密码格式不正确"))?;
    }
    Ok(url)
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    let bytes = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
    serde_json::from_slice::<Value>(&bytes)
        .with_context(|| format!("无法解析 {}", path.display()))?
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("{} 不是 JSON 对象", path.display()))
}

fn read_optional_json_object(path: &Path) -> Result<Map<String, Value>> {
    if path.is_file() {
        read_json_object(path)
    } else {
        Ok(Map::new())
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
    }
    let temporary = path.with_extension("locale-init.tmp");
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入临时文件 {}", temporary.display()))?;
    replace_file_atomic(&temporary, path)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("无法删除 {}", path.display()))?;
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("SHA-256格式不正确");
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("无法读取 {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:X}", digest.finalize()))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use tempfile::tempdir;

    fn write_seed_file(root: &Path, relative: &str, bytes: &[u8]) -> String {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        sha256_file(&path).unwrap()
    }

    fn sample_paths() -> (tempfile::TempDir, WorkstationPaths) {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths
            .prepare_engine_layout(crate::model::Engine::Codex)
            .unwrap();
        fs::create_dir_all(paths.codex_desktop.parent().unwrap()).unwrap();
        fs::write(&paths.codex_desktop, b"codex-test-binary").unwrap();
        fs::create_dir_all(paths.codex_desktop.parent().unwrap().join("locales")).unwrap();
        fs::write(
            paths
                .codex_desktop
                .parent()
                .unwrap()
                .join("locales/zh-CN.pak"),
            b"language-pack",
        )
        .unwrap();
        (temporary, paths)
    }

    fn create_valid_seed(paths: &WorkstationPaths) {
        let root = seed_root(paths);
        let preferences = br#"{"intl":{"selected_languages":"zh-CN,zh"}}"#;
        let config = b"[desktop]\nlocaleOverride = \"zh-CN\"\n";
        let bootstrap = br#"{"enabledOwlFeatureNames":["OwlAuth","OwlAutofillAndPasswords","OwlDownloads","OwlPermissions","OwlWebViewEnhancements","OwlPrinting"],"disabledOwlFeatureNames":["OwlExtensions","OwlHistory","OwlOpenAIGoLinks"]}"#;
        let make_native_record = |kind: &str, id: &str, rule_id: &str, is_in_layer: bool| {
            let value = serde_json::json!({
                "name": id,
                "rule_id": rule_id,
                "secondary_exposures": [],
                "version": 1,
                "value": {"enable_i18n": true, "locale_source": "IDE"},
                "id_type": "userID",
                "is_device_based": false,
                "explicit_parameters": [],
                "is_in_layer": is_in_layer
            });
            let bytes = serde_json::to_vec(&value).unwrap();
            serde_json::json!({
                "kind": kind,
                "id": id,
                "valueBase64": base64::engine::general_purpose::STANDARD.encode(&bytes),
                "sha256": format!("{:x}", Sha256::digest(&bytes))
            })
        };
        let native = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "storageOrigin": "_app://-",
            "database": "Default/Local Storage/leveldb",
            "keyDerivation": "statsigDjb2CurrentStableId",
            "statsigStorageSuffix": "685440364",
            "records": [
                make_native_record("dynamicConfig", "118392242", "targetingGate", true),
                make_native_record("layerConfig", "72216192", "default", false)
            ]
        }))
        .unwrap();
        let mut files = vec![
            serde_json::json!({
                "source":"profile-preferences.json","targetRoot":"browserData","target":"Default/Preferences","mode":"mergeBrowserPreferences",
                "sha256":write_seed_file(&root,"profile-preferences.json",preferences)
            }),
            serde_json::json!({
                "source":"profile-preferences.json","targetRoot":"browserData","target":"codex-browser-app/Preferences","mode":"mergeBrowserPreferences",
                "sha256":sha256_file(&root.join("profile-preferences.json")).unwrap()
            }),
            serde_json::json!({
                "source":"profile-preferences.json","targetRoot":"browserData","target":"Default/Partitions/codex-browser-app/Preferences","mode":"mergeBrowserPreferences",
                "sha256":sha256_file(&root.join("profile-preferences.json")).unwrap()
            }),
            serde_json::json!({
                "source":"config.toml","targetRoot":"codexHome","target":"config.toml","mode":"mergeCodexConfig",
                "sha256":write_seed_file(&root,"config.toml",config)
            }),
            serde_json::json!({
                "source":"owl-feature-bootstrap-cache.json","targetRoot":"browserData","target":"owl-feature-bootstrap-cache.json","mode":"copyFile",
                "sha256":write_seed_file(&root,"owl-feature-bootstrap-cache.json",bootstrap)
            }),
            serde_json::json!({
                "source":"native-i18n-state-v1.json","targetRoot":"browserData","target":"Default/Local Storage/leveldb","mode":"mergeBrowserLocalStorage",
                "sha256":write_seed_file(&root,"native-i18n-state-v1.json",&native)
            }),
        ];
        let codex_sha256 = sha256_file(&paths.codex_desktop).unwrap();
        let mut payload_items = files
            .iter()
            .map(|entry| {
                format!(
                    "{}={}",
                    entry["source"].as_str().unwrap(),
                    entry["sha256"].as_str().unwrap().to_ascii_lowercase()
                )
            })
            .collect::<Vec<_>>();
        payload_items.sort();
        payload_items.dedup();
        let payload_sha256 = format!(
            "{:x}",
            Sha256::digest(format!("{}\n", payload_items.join("\n")).as_bytes())
        );
        let native_sha256 = sha256_file(&root.join("native-i18n-state-v1.json")).unwrap();
        let marker = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "verified": true,
            "codexSha256": codex_sha256,
            "seedPayloadSha256": payload_sha256,
            "nativeI18nStateSha256": native_sha256
        }))
        .unwrap();
        files.push(serde_json::json!({
            "source":"codex-i18n-verified-v1.json","targetRoot":"runtimeState","target":"codex-i18n-verified-v1.json","mode":"copyFile",
            "sha256":write_seed_file(&root,"codex-i18n-verified-v1.json",&marker)
        }));
        write_json_atomic(
            &root.join(SEED_MANIFEST),
            &serde_json::json!({
                "schemaVersion":2,
                "codexSha256":codex_sha256,
                "files":files
            }),
        )
        .unwrap();

        fs::create_dir_all(paths.bundled_node.parent().unwrap()).unwrap();
        fs::write(&paths.bundled_node, "YUNSHU_NATIVE_I18N_TEST_OK").unwrap();
        let node_modules = paths
            .codex_desktop
            .parent()
            .unwrap()
            .join("resources/plugins/openai-bundled/plugins/browser/scripts/node_modules");
        for package in LOCAL_I18N_PACKAGES {
            fs::create_dir_all(node_modules.join(package)).unwrap();
        }
        let classic_level = node_modules.join("classic-level");
        fs::create_dir_all(&classic_level).unwrap();
        fs::write(classic_level.join("index.js"), b"test fixture").unwrap();
    }

    #[test]
    fn locale_seed_merges_only_locale_fields_and_preserves_user_data() {
        let (_temporary, paths) = sample_paths();
        create_valid_seed(&paths);
        let preferences = paths.codex_browser_data.join("Default/Preferences");
        fs::create_dir_all(preferences.parent().unwrap()).unwrap();
        fs::write(
            &preferences,
            r#"{"intl":{"selected_languages":"en-US"},"profile":{"name":"便携用户"}}"#.as_bytes(),
        )
        .unwrap();
        fs::write(
            paths.codex_home.join("config.toml"),
            "model = \"gpt-test\"\n[provider]\napi_key_ref = \"provider-codex\"\n",
        )
        .unwrap();
        let machine_config = paths.codex_machine_home().join("config.toml");
        fs::create_dir_all(machine_config.parent().unwrap()).unwrap();
        fs::write(&machine_config, "model = \"machine-test\"\n").unwrap();
        let session = paths.codex_home.join("sessions/user-session.jsonl");
        fs::create_dir_all(session.parent().unwrap()).unwrap();
        fs::write(&session, b"private-session").unwrap();

        let result = apply_seed(&paths).unwrap();

        assert_eq!(result.applied_files, 7);
        assert!(
            result.status.current.ready,
            "language={} config={:?} profiles={} owl={} native={:?}",
            result.status.current.language_pack_present,
            result.status.current.config_locale,
            result
                .status
                .current
                .profile_locales
                .iter()
                .all(|profile| profile.ready),
            result.status.current.feature_bootstrap.ready,
            result.status.current.native_i18n_state
        );
        assert_eq!(fs::read(&session).unwrap(), b"private-session");
        let preferences: Value = serde_json::from_slice(&fs::read(&preferences).unwrap()).unwrap();
        assert_eq!(preferences["intl"]["selected_languages"], "zh-CN,zh");
        assert_eq!(preferences["profile"]["name"], "便携用户");
        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(config.contains("model = \"gpt-test\""));
        assert!(config.contains("api_key_ref = \"provider-codex\""));
        assert!(config.contains("localeOverride = \"zh-CN\""));
        let machine_config = fs::read_to_string(machine_config).unwrap();
        assert!(machine_config.contains("model = \"machine-test\""));
        assert!(machine_config.contains("localeOverride = \"zh-CN\""));
        for relative in [
            "Default/Preferences",
            "codex-browser-app/Preferences",
            "Default/Partitions/codex-browser-app/Preferences",
        ] {
            assert!(paths.codex_machine_browser_data().join(relative).is_file());
        }
        assert!(
            paths
                .runtime_state
                .join(NATIVE_I18N_APPLIED_MARKER)
                .is_file()
        );
        assert!(
            !paths
                .machine_cache_root()
                .join("first-use-locale-staging")
                .exists()
        );
    }

    #[test]
    fn first_use_locale_seed_stages_only_a_blank_native_database() {
        let (_temporary, paths) = sample_paths();
        create_valid_seed(&paths);
        let database = paths
            .codex_browser_data
            .join("Default/Local Storage/leveldb");

        apply_seed_portable_only(&paths).unwrap();

        assert!(database.is_dir());
        let staging_root = paths.machine_cache_root().join("first-use-locale-staging");
        assert!(!staging_root.exists() || fs::read_dir(staging_root).unwrap().next().is_none());
    }

    #[test]
    fn locale_seed_rejects_version_mismatch_without_touching_data() {
        let (_temporary, paths) = sample_paths();
        create_valid_seed(&paths);
        fs::write(&paths.codex_desktop, b"different-version").unwrap();
        let sentinel = paths.codex_home.join("config.toml");
        fs::write(&sentinel, b"model = \"keep\"\n").unwrap();

        let status = inspect_seed(&paths);

        assert_eq!(status.state, CodexLocaleSeedState::VersionMismatch);
        assert!(apply_seed(&paths).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"model = \"keep\"\n");
    }

    #[test]
    fn locale_seed_rejects_sensitive_or_traversing_targets() {
        assert!(safe_relative_path("../sessions/private.jsonl").is_err());
        assert!(safe_relative_path("C:/Users/test").is_err());
        assert!(reject_sensitive_target(Path::new("sessions/private.jsonl")).is_err());
        assert!(reject_sensitive_target(Path::new("Default/Cookies")).is_err());
        assert!(
            validate_mode_target(
                LocaleSeedApplyMode::CopyFile,
                LocaleSeedTargetRoot::CodexHome,
                Path::new("config.toml"),
            )
            .is_err()
        );
    }

    #[test]
    fn schema_v1_seed_is_incomplete() {
        let (_temporary, paths) = sample_paths();
        let root = seed_root(&paths);
        write_json_atomic(
            &root.join(SEED_MANIFEST),
            &serde_json::json!({
                "schemaVersion": 1,
                "codexSha256": sha256_file(&paths.codex_desktop).unwrap(),
                "files": []
            }),
        )
        .unwrap();
        assert_eq!(inspect_seed(&paths).state, CodexLocaleSeedState::Incomplete);
    }

    #[test]
    fn locale_seed_rolls_back_portable_and_machine_data_when_native_merge_fails() {
        let (_temporary, paths) = sample_paths();
        create_valid_seed(&paths);
        fs::write(&paths.bundled_node, "YUNSHU_NATIVE_I18N_TEST_FAIL").unwrap();
        let portable = paths.codex_home.join("config.toml");
        let machine = paths.codex_machine_home().join("config.toml");
        fs::create_dir_all(machine.parent().unwrap()).unwrap();
        fs::write(&portable, b"model = \"portable\"\n").unwrap();
        fs::write(&machine, b"model = \"machine\"\n").unwrap();

        assert!(apply_seed(&paths).is_err());
        assert_eq!(fs::read(&portable).unwrap(), b"model = \"portable\"\n");
        assert_eq!(fs::read(&machine).unwrap(), b"model = \"machine\"\n");
        assert!(
            !paths
                .runtime_state
                .join(NATIVE_I18N_APPLIED_MARKER)
                .exists()
        );
    }

    #[test]
    fn proxy_config_encrypts_password_and_is_consumed_once() {
        let (_temporary, paths) = sample_paths();
        save_proxy_config(
            &paths,
            CodexInitProxySaveInput {
                protocol: CodexInitProxyProtocol::Socks5,
                host: "127.0.0.1".to_string(),
                port: 7890,
                username: Some("portable".to_string()),
                password: Some("private-proxy-password".to_string()),
            },
        )
        .unwrap();
        assert!(
            !fs::read_to_string(proxy_config_path(&paths))
                .unwrap()
                .contains("private-proxy-password")
        );
        assert!(
            !String::from_utf8_lossy(&fs::read(paths.vault_file()).unwrap())
                .contains("private-proxy-password")
        );

        activate_proxy_for_next_codex_start(&paths).unwrap();
        let launch = take_codex_proxy_launch(&paths).unwrap().unwrap();
        assert_eq!(
            launch.chromium_proxy_argument,
            OsString::from("--proxy-server=socks5://127.0.0.1:7890")
        );
        assert!(launch.environment.iter().any(|(key, value)| {
            key == "HTTPS_PROXY" && value.to_string_lossy().contains("private-proxy-password")
        }));
        assert!(take_codex_proxy_launch(&paths).unwrap().is_none());

        clear_proxy_config(&paths).unwrap();
        assert!(!proxy_config_path(&paths).exists());
        assert!(
            !PortableVault::open(&paths)
                .unwrap()
                .contains(PROXY_PASSWORD_REF)
        );
    }
}
