//! Codex 本机 NTFS 运行镜像。
//!
//! 移动盘仍是会话、配置、Skills、插件与浏览器登录状态的持久主副本；Codex 运行时
//! 使用 `%LOCALAPPDATA%` 下的工作镜像，避免 Chromium、LevelDB 与 SQLite WAL 在
//! exFAT/U 盘上高频同步写入导致输入卡顿。只有 Codex 完全退出后才把持久数据回写。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::paths::WorkstationPaths;
use crate::plugins::{MARKETPLACE_NAME, catalog};
use crate::windows_process::replace_file_atomic;

const MIRROR_SCHEMA_VERSION: u32 = 1;
const OFFICIAL_BUNDLED_PLUGINS: [&str; 4] = ["browser", "computer-use", "sites", "visualize"];
const OFFICIAL_BUNDLED_MARKETPLACE: &str = "openai-bundled";
static COPY_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const FILE_COPY_ATTEMPTS: usize = 12;
const FILE_COPY_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct CodexRuntimePaths {
    pub home: PathBuf,
    pub browser_data: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexMirrorSyncSummary {
    pub copied_files: u64,
    pub copied_bytes: u64,
    pub skipped_files: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKind {
    CodexHome,
    BrowserData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    /// 正常启动时以移动盘为准；关键状态即使大小和时间相同也重新复制。
    PortableAuthority,
    /// 异常退出恢复时只复制比目标更新或大小不同的文件。
    NewerOnly,
    /// 正常停止时以本机运行镜像为准，强制回写关键状态。
    RuntimeAuthority,
}

pub fn runtime_paths(paths: &WorkstationPaths) -> CodexRuntimePaths {
    CodexRuntimePaths {
        home: paths.codex_machine_home(),
        browser_data: paths.codex_machine_browser_data(),
    }
}

/// 准备 Codex 的本机运行镜像。
///
/// 如果上次没有完成安全停止，先把本机较新的状态恢复到移动盘；随后再从移动盘刷新
/// 本机镜像。整个过程不删除移动盘文件，也不会读取或修改 F 盘样本。
pub fn prepare(paths: &WorkstationPaths) -> Result<CodexRuntimePaths> {
    let runtime = runtime_paths(paths);
    fs::create_dir_all(&runtime.home)
        .with_context(|| format!("无法创建Codex本机Home {}", runtime.home.display()))?;
    fs::create_dir_all(&runtime.browser_data).with_context(|| {
        format!(
            "无法创建Codex本机浏览器目录 {}",
            runtime.browser_data.display()
        )
    })?;

    let dirty_marker = paths.codex_machine_dirty_marker();
    if dirty_marker.is_file() {
        sync_tree(
            &runtime.home,
            &paths.codex_home,
            TreeKind::CodexHome,
            SyncMode::NewerOnly,
            true,
        )?;
        sync_tree(
            &runtime.browser_data,
            &paths.codex_browser_data,
            TreeKind::BrowserData,
            SyncMode::NewerOnly,
            true,
        )?;
        merge_runtime_config_to_portable(paths, &runtime.home, false)?;
    }

    let startup_mode = if dirty_marker.is_file() {
        SyncMode::NewerOnly
    } else {
        SyncMode::PortableAuthority
    };
    sync_tree(
        &paths.codex_home,
        &runtime.home,
        TreeKind::CodexHome,
        startup_mode,
        false,
    )?;
    sync_tree(
        &paths.codex_browser_data,
        &runtime.browser_data,
        TreeKind::BrowserData,
        startup_mode,
        false,
    )?;

    clear_runtime_transients(&runtime)?;
    let bundled = prepare_official_bundled_marketplace(paths, &runtime.home)?;
    prepare_runtime_config(paths, &runtime.home, &bundled)?;
    write_marker(&dirty_marker, "running")?;
    Ok(runtime)
}

/// Codex 完全退出后，把会话、配置、Skills、插件和浏览器持久状态回写到移动盘。
/// 缓存、日志、Crashpad、锁文件和 SQLite SHM 不回写。
pub fn sync_to_portable(paths: &WorkstationPaths) -> Result<CodexMirrorSyncSummary> {
    let runtime = runtime_paths(paths);
    if !runtime.home.is_dir() && !runtime.browser_data.is_dir() {
        return Ok(CodexMirrorSyncSummary::default());
    }

    let mut summary = CodexMirrorSyncSummary::default();
    if runtime.home.is_dir() {
        summary.merge(sync_tree(
            &runtime.home,
            &paths.codex_home,
            TreeKind::CodexHome,
            SyncMode::RuntimeAuthority,
            true,
        )?);
    }
    if runtime.browser_data.is_dir() {
        summary.merge(sync_tree(
            &runtime.browser_data,
            &paths.codex_browser_data,
            TreeKind::BrowserData,
            SyncMode::RuntimeAuthority,
            true,
        )?);
    }

    merge_runtime_config_to_portable(paths, &runtime.home, true)?;

    remove_if_exists(&paths.codex_machine_dirty_marker())?;
    write_marker(&paths.codex_machine_last_sync_marker(), "clean")?;
    Ok(summary)
}

impl CodexMirrorSyncSummary {
    fn merge(&mut self, other: Self) {
        self.copied_files += other.copied_files;
        self.copied_bytes += other.copied_bytes;
        self.skipped_files += other.skipped_files;
    }
}

fn sync_tree(
    source: &Path,
    destination: &Path,
    kind: TreeKind,
    mode: SyncMode,
    to_portable: bool,
) -> Result<CodexMirrorSyncSummary> {
    if !source.is_dir() {
        return Ok(CodexMirrorSyncSummary::default());
    }
    fs::create_dir_all(destination)?;
    let mut summary = CodexMirrorSyncSummary::default();
    let mut stack = vec![(source.to_path_buf(), PathBuf::new())];

    while let Some((directory, relative)) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("无法读取Codex同步目录 {}", directory.display()))?
        {
            let entry = entry?;
            let child_relative = relative.join(entry.file_name());
            if to_portable
                && kind == TreeKind::CodexHome
                && (child_relative == Path::new("config.toml")
                    || child_relative == Path::new(crate::provider::CODEX_MODEL_CATALOG))
            {
                continue;
            }
            if sync_entry_is_transient(&child_relative, kind) {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let target = destination.join(&child_relative);
            if file_type.is_dir() {
                fs::create_dir_all(&target)?;
                stack.push((entry.path(), child_relative));
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let force = match mode {
                SyncMode::PortableAuthority | SyncMode::RuntimeAuthority => {
                    sync_entry_is_critical(&child_relative, kind)
                }
                SyncMode::NewerOnly => false,
            };
            if force || source_file_is_newer_or_different(&entry.path(), &target)? {
                copy_file_atomic(&entry.path(), &target)?;
                let length = entry.metadata()?.len();
                summary.copied_files += 1;
                summary.copied_bytes += length;
            } else {
                summary.skipped_files += 1;
            }
        }
    }
    Ok(summary)
}

fn sync_entry_is_transient(relative: &Path, kind: TreeKind) -> bool {
    let components = normalized_components(relative);
    let file_name = components.last().map(String::as_str).unwrap_or_default();
    if file_name.eq_ignore_ascii_case("lock")
        || file_name.eq_ignore_ascii_case("singletonlock")
        || file_name.eq_ignore_ascii_case("singletoncookie")
        || file_name.eq_ignore_ascii_case("singletonsocket")
        || file_name.ends_with(".shm")
        || file_name.ends_with(".tmp")
    {
        return true;
    }

    match kind {
        TreeKind::CodexHome => {
            file_name.eq_ignore_ascii_case("logs_2.sqlite")
                || file_name.eq_ignore_ascii_case("logs_2.sqlite-wal")
                || components.iter().any(|component| {
                    matches!(
                        component.as_str(),
                        ".tmp" | "tmp" | "logs" | "crashpad" | "sentry" | ".venv" | "__pycache__"
                    )
                })
        }
        TreeKind::BrowserData => {
            file_name.eq_ignore_ascii_case("browsermetrics-spare")
                || components.iter().any(|component| {
                    matches!(
                        component.as_str(),
                        "cache"
                            | "cache_data"
                            | "code cache"
                            | "gpucache"
                            | "shadercache"
                            | "dawncache"
                            | "grshadercache"
                            | "graphitedawncache"
                            | "crashpad"
                            | "browsermetrics"
                            | "safe browsing network"
                            | "widevinecdm"
                            | "sentry"
                    )
                })
        }
    }
}

fn sync_entry_is_critical(relative: &Path, kind: TreeKind) -> bool {
    let components = normalized_components(relative);
    let first = components.first().map(String::as_str).unwrap_or_default();
    let file_name = components.last().map(String::as_str).unwrap_or_default();
    match kind {
        TreeKind::CodexHome => {
            matches!(
                file_name,
                "config.toml"
                    | ".codex-global-state.json"
                    | "state_5.sqlite"
                    | "state_5.sqlite-wal"
                    | "memories_1.sqlite"
                    | "memories_1.sqlite-wal"
                    | "goals_1.sqlite"
                    | "goals_1.sqlite-wal"
            ) || matches!(first, "sessions" | "history" | "rules" | "projects")
        }
        TreeKind::BrowserData => {
            matches!(
                file_name,
                "local state"
                    | "preferences"
                    | "secure preferences"
                    | "cookies"
                    | "cookies-journal"
                    | "login data"
                    | "login data for account"
                    | "web data"
                    | "history"
            ) || components.iter().any(|component| {
                matches!(
                    component.as_str(),
                    "local storage" | "indexeddb" | "session storage" | "service worker"
                )
            })
        }
    }
}

fn normalized_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn source_file_is_newer_or_different(source: &Path, destination: &Path) -> Result<bool> {
    if !destination.is_file() {
        return Ok(true);
    }
    let source_metadata = fs::metadata(source)?;
    let destination_metadata = fs::metadata(destination)?;
    if source_metadata.len() != destination_metadata.len() {
        return Ok(true);
    }
    Ok(
        match (source_metadata.modified(), destination_metadata.modified()) {
            (Ok(source_time), Ok(destination_time)) => source_time > destination_time,
            _ => false,
        },
    )
}

fn copy_file_atomic(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension(format!(
        "yunshu-codex-sync-{}-{}.tmp",
        std::process::id(),
        COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut last_error = None;
    for attempt in 0..FILE_COPY_ATTEMPTS {
        let _ = remove_if_exists(&temporary);
        let result = fs::copy(source, &temporary)
            .with_context(|| {
                format!(
                    "无法复制Codex运行数据 {} -> {}",
                    source.display(),
                    temporary.display()
                )
            })
            .and_then(|_| replace_file_atomic(&temporary, destination));
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                let _ = remove_if_exists(&temporary);
                if attempt + 1 < FILE_COPY_ATTEMPTS {
                    thread::sleep(FILE_COPY_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        anyhow!(
            "Codex运行数据在多次尝试后仍无法保存：{}",
            destination.display()
        )
    }))
}

fn clear_runtime_transients(runtime: &CodexRuntimePaths) -> Result<()> {
    for directory in [
        runtime.home.join("tmp"),
        runtime.home.join("logs"),
        runtime.browser_data.join("Cache"),
        runtime.browser_data.join("Code Cache"),
        runtime.browser_data.join("GPUCache"),
        runtime.browser_data.join("ShaderCache"),
        runtime.browser_data.join("DawnCache"),
        runtime.browser_data.join("Crashpad"),
        runtime.browser_data.join("BrowserMetrics"),
        runtime.browser_data.join("sentry"),
    ] {
        if directory.exists() {
            fs::remove_dir_all(&directory)
                .with_context(|| format!("无法清理Codex本机临时缓存 {}", directory.display()))?;
        }
    }
    for file in [
        runtime.home.join("logs_2.sqlite"),
        runtime.home.join("logs_2.sqlite-wal"),
        runtime.home.join("logs_2.sqlite-shm"),
    ] {
        remove_if_exists(&file)?;
    }
    remove_transient_dependency_caches(&runtime.home)?;
    remove_stale_lock_files(&runtime.home)?;
    remove_stale_lock_files(&runtime.browser_data)
}

fn remove_transient_dependency_caches(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if matches!(name.as_str(), ".venv" | "__pycache__") {
                fs::remove_dir_all(entry.path()).with_context(|| {
                    format!("无法清理Codex本机依赖缓存 {}", entry.path().display())
                })?;
            } else {
                stack.push(entry.path());
            }
        }
    }
    Ok(())
}

fn remove_stale_lock_files(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "lock"
                || name == "singletonlock"
                || name == "singletoncookie"
                || name == "singletonsocket"
                || name.ends_with(".shm")
            {
                remove_if_exists(&entry.path())?;
            }
        }
    }
    Ok(())
}

fn prepare_official_bundled_marketplace(paths: &WorkstationPaths, home: &Path) -> Result<PathBuf> {
    let source = paths
        .codex_desktop
        .parent()
        .context("Codex桌面主体缺少父目录")?
        .join("resources")
        .join("plugins")
        .join(OFFICIAL_BUNDLED_MARKETPLACE);
    validate_official_bundled(&source)?;

    let parent = home.join(".tmp").join("bundled-marketplaces");
    fs::create_dir_all(&parent)?;
    let destination = parent.join(OFFICIAL_BUNDLED_MARKETPLACE);
    let staging = parent.join(format!(
        ".{OFFICIAL_BUNDLED_MARKETPLACE}.staging-{}-{}",
        std::process::id(),
        COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    sync_tree(
        &source,
        &staging,
        TreeKind::CodexHome,
        SyncMode::PortableAuthority,
        false,
    )?;
    validate_official_bundled(&staging)?;

    let previous = parent.join(format!(".{OFFICIAL_BUNDLED_MARKETPLACE}.previous"));
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
    if destination.exists() {
        fs::rename(&destination, &previous)
            .with_context(|| format!("无法暂存旧的官方插件目录 {}", destination.display()))?;
    }
    if let Err(error) = fs::rename(&staging, &destination) {
        if previous.exists() && !destination.exists() {
            let _ = fs::rename(&previous, &destination);
        }
        return Err(error).context("无法启用完整的官方插件目录");
    }
    if previous.exists() {
        fs::remove_dir_all(previous)?;
    }
    Ok(destination)
}

fn validate_official_bundled(root: &Path) -> Result<()> {
    let marketplace = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace.is_file() {
        bail!("官方插件目录缺少 marketplace.json：{}", root.display());
    }
    let missing = OFFICIAL_BUNDLED_PLUGINS
        .iter()
        .map(|name| root.join("plugins").join(name))
        .filter(|path| !path.is_dir())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "官方插件主体不完整：{}",
            missing
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("；")
        );
    }
    Ok(())
}

fn prepare_runtime_config(
    paths: &WorkstationPaths,
    home: &Path,
    official_bundled: &Path,
) -> Result<()> {
    let config_path = home.join("config.toml");
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析Codex本机配置 {}", config_path.display()))?
    };

    let portable_text =
        fs::read_to_string(paths.codex_home.join("config.toml")).unwrap_or_default();
    if !portable_text.trim().is_empty() {
        let portable = portable_text.parse::<DocumentMut>()?;
        if portable.get("model_provider").and_then(Item::as_str) == Some("yunshu-codex") {
            for key in ["model", "model_provider", "preferred_auth_method"] {
                if let Some(item) = portable.get(key) {
                    document[key] = item.clone();
                }
            }
            if let Some(provider) = portable
                .get("model_providers")
                .and_then(Item::as_table)
                .and_then(|providers| providers.get("yunshu-codex"))
            {
                ensure_table(document.as_table_mut(), "model_providers")?;
                document["model_providers"]["yunshu-codex"] = provider.clone();
            }
            let catalog = crate::provider::CODEX_MODEL_CATALOG;
            if portable.get("model_catalog_json").is_some()
                && paths.codex_home.join(catalog).is_file()
            {
                copy_file_atomic(&paths.codex_home.join(catalog), &home.join(catalog))?;
                document["model_catalog_json"] =
                    value(home.join(catalog).to_string_lossy().as_ref());
            }
        }
    }

    ensure_table(document.as_table_mut(), "marketplaces")?;
    let marketplaces = document["marketplaces"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex marketplaces 不是表结构"))?;
    let mut official = Table::new();
    official["source_type"] = value("local");
    official["source"] = value(official_bundled.to_string_lossy().to_string());
    marketplaces.insert(OFFICIAL_BUNDLED_MARKETPLACE, Item::Table(official));
    if marketplaces.contains_key(MARKETPLACE_NAME) {
        if let Ok(snapshot) = catalog::locate_snapshot(paths) {
            set_local_marketplace_source(marketplaces, MARKETPLACE_NAME, &snapshot);
        }
    }

    let invalid_marketplaces = marketplaces
        .iter()
        .filter_map(|(name, item)| {
            let table = item.as_table()?;
            if table.get("source_type").and_then(Item::as_str) != Some("local") {
                return None;
            }
            let source = table.get("source").and_then(Item::as_str)?;
            (!Path::new(source.trim_start_matches(r"\\?\")).exists()).then(|| name.to_string())
        })
        .collect::<Vec<_>>();
    for marketplace in invalid_marketplaces {
        marketplaces.remove(&marketplace);
    }
    let configured_marketplaces = marketplaces
        .iter()
        .map(|(name, _)| name.to_string())
        .collect::<BTreeSet<_>>();

    ensure_table(document.as_table_mut(), "plugins")?;
    let plugins = document["plugins"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex plugins 不是表结构"))?;
    let stale_plugins = plugins
        .iter()
        .filter_map(|(key, _)| {
            let (_, marketplace) = key.rsplit_once('@')?;
            (!configured_marketplaces.contains(marketplace)).then(|| key.to_string())
        })
        .collect::<Vec<_>>();
    for plugin in stale_plugins {
        plugins.remove(&plugin);
    }
    for plugin in OFFICIAL_BUNDLED_PLUGINS {
        let key = format!("{plugin}@{OFFICIAL_BUNDLED_MARKETPLACE}");
        let mut table = plugins
            .get(&key)
            .and_then(Item::as_table)
            .cloned()
            .unwrap_or_default();
        table["enabled"] = value(true);
        plugins.insert(&key, Item::Table(table));
    }
    ensure_runtime_permission_profile(&mut document)?;

    let mut bytes = document.to_string().into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let temporary = config_path.with_extension("toml.yunshu-runtime.tmp");
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, &config_path)
}

/// 只把官方界面里产生的项目、权限、功能开关和有效插件启用状态合并回移动盘。
/// 供应商、模型和密钥引用仍以启动器写入的移动盘配置为准；本机 marketplace 路径
/// 永远不会回写到移动盘。
fn merge_runtime_config_to_portable(
    paths: &WorkstationPaths,
    runtime_home: &Path,
    force: bool,
) -> Result<()> {
    let runtime_path = runtime_home.join("config.toml");
    let portable_path = paths.codex_home.join("config.toml");
    if !runtime_path.is_file() {
        return Ok(());
    }
    if !force
        && portable_path.is_file()
        && !source_file_is_newer_or_different(&runtime_path, &portable_path)?
    {
        return Ok(());
    }

    let runtime_text = fs::read_to_string(&runtime_path)?;
    let portable_text = fs::read_to_string(&portable_path).unwrap_or_default();
    let runtime_document = runtime_text
        .parse::<DocumentMut>()
        .with_context(|| format!("无法解析Codex本机配置 {}", runtime_path.display()))?;
    let mut portable_document = if portable_text.trim().is_empty() {
        DocumentMut::new()
    } else {
        portable_text
            .parse::<DocumentMut>()
            .with_context(|| format!("无法解析Codex便携配置 {}", portable_path.display()))?
    };

    for key in [
        "projects",
        "features",
        "desktop",
        "default_permissions",
        "approval_policy",
        "sandbox_mode",
        "approvals_reviewer",
        "web_search",
        "model_reasoning_effort",
        "personality",
    ] {
        if let Some(item) = runtime_document.as_table().get(key) {
            portable_document[key] = item.clone();
        }
    }
    if let Some(sandbox) = runtime_document
        .get("windows")
        .and_then(Item::as_table)
        .and_then(|windows| windows.get("sandbox"))
        .and_then(Item::as_str)
    {
        ensure_table(portable_document.as_table_mut(), "windows")?;
        portable_document["windows"]["sandbox"] = value(sandbox);
    }

    ensure_table(portable_document.as_table_mut(), "marketplaces")?;
    let portable_marketplaces = portable_document["marketplaces"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex便携 marketplaces 不是表结构"))?;
    let official_source = paths
        .codex_desktop
        .parent()
        .context("Codex桌面主体缺少父目录")?
        .join("resources")
        .join("plugins")
        .join(OFFICIAL_BUNDLED_MARKETPLACE);
    validate_official_bundled(&official_source)?;
    let mut official = portable_marketplaces
        .get(OFFICIAL_BUNDLED_MARKETPLACE)
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    official["source_type"] = value("local");
    official["source"] = value(official_source.to_string_lossy().to_string());
    portable_marketplaces.insert(OFFICIAL_BUNDLED_MARKETPLACE, Item::Table(official));
    let runtime_has_workstation_marketplace = runtime_document
        .get("marketplaces")
        .and_then(Item::as_table)
        .is_some_and(|marketplaces| marketplaces.contains_key(MARKETPLACE_NAME));
    if (portable_marketplaces.contains_key(MARKETPLACE_NAME) || runtime_has_workstation_marketplace)
        && let Ok(snapshot) = catalog::locate_snapshot(paths)
    {
        set_local_marketplace_source(portable_marketplaces, MARKETPLACE_NAME, &snapshot);
    }
    let configured = portable_marketplaces
        .iter()
        .map(|(name, _)| name.to_string())
        .collect::<BTreeSet<_>>();

    ensure_table(portable_document.as_table_mut(), "plugins")?;
    let portable_plugins = portable_document["plugins"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("Codex便携 plugins 不是表结构"))?;
    let stale_plugins = portable_plugins
        .iter()
        .filter_map(|(key, _)| {
            let (_, marketplace) = key.rsplit_once('@')?;
            (!configured.contains(marketplace)).then(|| key.to_string())
        })
        .collect::<Vec<_>>();
    for key in stale_plugins {
        portable_plugins.remove(&key);
    }
    if let Some(runtime_plugins) = runtime_document
        .as_table()
        .get("plugins")
        .and_then(Item::as_table)
    {
        for (key, item) in runtime_plugins {
            let Some((_, marketplace)) = key.rsplit_once('@') else {
                continue;
            };
            if configured.contains(marketplace) {
                portable_plugins.insert(key, item.clone());
            }
        }
    }

    let mut bytes = portable_document.to_string().into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let temporary = portable_path.with_extension("toml.yunshu-merge.tmp");
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, &portable_path)
}

fn set_local_marketplace_source(marketplaces: &mut Table, name: &str, source: &Path) {
    let mut marketplace = marketplaces
        .get(name)
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    marketplace["source_type"] = value("local");
    marketplace["source"] = value(source.to_string_lossy().to_string());
    marketplaces.insert(name, Item::Table(marketplace));
}

fn ensure_runtime_permission_profile(document: &mut DocumentMut) -> Result<()> {
    let explicit_read_only = document.get("default_permissions").and_then(Item::as_str)
        == Some(":read-only")
        && document.get("approval_policy").and_then(Item::as_str) == Some("on-request")
        && document.get("sandbox_mode").and_then(Item::as_str) == Some("read-only");
    if !explicit_read_only {
        document["default_permissions"] = value(":danger-full-access");
        document["approval_policy"] = value("never");
        document["sandbox_mode"] = value("danger-full-access");
    }
    if document.get("approvals_reviewer").is_none() {
        document["approvals_reviewer"] = value("user");
    }
    ensure_table(document.as_table_mut(), "windows")?;
    document["windows"]["sandbox"] = value("unelevated");
    Ok(())
}

fn ensure_table(table: &mut Table, key: &str) -> Result<()> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    if !table.get(key).is_some_and(Item::is_table) {
        bail!("Codex配置中的 {key} 不是表结构");
    }
    Ok(())
}

fn write_marker(path: &Path, state: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let value = serde_json::json!({
        "schemaVersion": MIRROR_SCHEMA_VERSION,
        "state": state,
        "updatedAtUnixMs": timestamp,
    });
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, path)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法删除 {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, WorkstationPaths) {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 路径").join("dev2"));
        paths
            .prepare_engine_layout(crate::model::Engine::Codex)
            .unwrap();
        let bundled = paths
            .codex_desktop
            .parent()
            .unwrap()
            .join("resources/plugins/openai-bundled");
        fs::create_dir_all(bundled.join(".agents/plugins")).unwrap();
        fs::write(bundled.join(".agents/plugins/marketplace.json"), b"{}").unwrap();
        for plugin in OFFICIAL_BUNDLED_PLUGINS {
            fs::create_dir_all(bundled.join("plugins").join(plugin)).unwrap();
            fs::write(
                bundled.join("plugins").join(plugin).join("plugin.json"),
                b"{}",
            )
            .unwrap();
        }
        (temporary, paths)
    }

    fn portable_plugin_snapshot(paths: &WorkstationPaths) -> PathBuf {
        let snapshot = paths
            .plugins_payloads
            .join("openai-curated")
            .join("snapshot-current");
        fs::create_dir_all(snapshot.join(".agents/plugins")).unwrap();
        fs::write(
            snapshot.join(".agents/plugins/marketplace.json"),
            br#"{"name":"yunshu_plugins","plugins":[]}"#,
        )
        .unwrap();
        snapshot
    }

    #[test]
    fn runtime_config_refreshes_only_workstation_provider_keys() {
        let (_temporary, paths) = fixture();
        fs::write(paths.codex_home.join("config.toml"),
            "model='new-model'\nmodel_provider='yunshu-codex'\nmodel_catalog_json='yunshu-model-catalog.json'\n[model_providers.yunshu-codex]\nbase_url='https://new.example/v1'\n").unwrap();
        fs::write(
            paths.codex_home.join("yunshu-model-catalog.json"),
            "{\"models\":[]}",
        )
        .unwrap();
        let home = paths.root.join("test-runtime-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"),
            "model='old-model'\nmodel_provider='old-provider'\npersonality='friendly'\n[model_providers.unrelated]\nbase_url='https://keep.example/v1'\n").unwrap();
        let bundled = paths
            .codex_desktop
            .parent()
            .unwrap()
            .join("resources/plugins/openai-bundled");
        prepare_runtime_config(&paths, &home, &bundled).unwrap();
        let document = fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(document["model"].as_str(), Some("new-model"));
        assert_eq!(document["personality"].as_str(), Some("friendly"));
        assert_eq!(
            document["model_providers"]["unrelated"]["base_url"].as_str(),
            Some("https://keep.example/v1")
        );
        let catalog = document["model_catalog_json"].as_str().unwrap();
        assert_eq!(Path::new(catalog), home.join("yunshu-model-catalog.json"));
        assert_eq!(fs::read(catalog).unwrap(), b"{\"models\":[]}");
    }

    #[test]
    fn runtime_mirror_keeps_durable_state_and_excludes_hot_logs() {
        let (_temporary, paths) = fixture();
        fs::write(paths.codex_home.join("config.toml"), b"model = 'test'\n").unwrap();
        fs::write(paths.codex_home.join("state_5.sqlite"), b"state").unwrap();
        fs::write(paths.codex_home.join("logs_2.sqlite"), b"logs").unwrap();
        let skill = paths.codex_home.join("skills/example");
        fs::create_dir_all(skill.join("scripts")).unwrap();
        fs::create_dir_all(skill.join(".venv/Lib")).unwrap();
        fs::create_dir_all(skill.join("__pycache__")).unwrap();
        fs::write(skill.join("SKILL.md"), b"# Example").unwrap();
        fs::write(skill.join("scripts/tool.py"), b"print('ok')").unwrap();
        fs::write(skill.join(".venv/Lib/native.lib"), b"machine").unwrap();
        fs::write(skill.join("__pycache__/tool.pyc"), b"cache").unwrap();
        fs::write(paths.codex_browser_data.join("Local State"), b"profile").unwrap();
        fs::create_dir_all(paths.codex_browser_data.join("GPUCache")).unwrap();
        fs::write(paths.codex_browser_data.join("GPUCache/data_0"), b"cache").unwrap();

        let runtime = prepare(&paths).unwrap();
        assert!(runtime.home.join("config.toml").is_file());
        assert!(runtime.home.join("state_5.sqlite").is_file());
        assert!(!runtime.home.join("logs_2.sqlite").exists());
        assert!(runtime.home.join("skills/example/SKILL.md").is_file());
        assert!(
            runtime
                .home
                .join("skills/example/scripts/tool.py")
                .is_file()
        );
        assert!(!runtime.home.join("skills/example/.venv").exists());
        assert!(!runtime.home.join("skills/example/__pycache__").exists());
        assert!(runtime.browser_data.join("Local State").is_file());
        assert!(!runtime.browser_data.join("GPUCache/data_0").exists());
        assert!(paths.codex_machine_dirty_marker().is_file());
    }

    #[test]
    fn browser_sync_excludes_regenerable_runtime_files_but_keeps_account_state() {
        assert!(sync_entry_is_transient(
            Path::new("BrowserMetrics-spare"),
            TreeKind::BrowserData
        ));
        assert!(sync_entry_is_transient(
            Path::new("WidevineCdm/4.10/widevinecdm.dll"),
            TreeKind::BrowserData
        ));
        assert!(sync_entry_is_transient(
            Path::new("Default/Partitions/codex-browser-app/Safe Browsing Network/Cookies"),
            TreeKind::BrowserData
        ));
        assert!(sync_entry_is_critical(
            Path::new("Default/Login Data For Account"),
            TreeKind::BrowserData
        ));
        assert!(sync_entry_is_critical(
            Path::new("Default/Local Storage/leveldb/000001.ldb"),
            TreeKind::BrowserData
        ));
        assert!(!sync_entry_is_critical(
            Path::new("Default/Network Persistent State"),
            TreeKind::BrowserData
        ));
    }

    #[test]
    fn atomic_copy_replaces_destination_without_leaving_sync_temporary_file() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source.db");
        let destination = temporary.path().join("destination.db");
        fs::write(&source, b"new-state").unwrap();
        fs::write(&destination, b"old-state").unwrap();

        copy_file_atomic(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new-state");
        assert!(
            fs::read_dir(temporary.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .contains("yunshu-codex-sync"))
        );
    }

    #[test]
    fn runtime_config_prunes_missing_marketplaces_and_enables_complete_official_bundle() {
        let (_temporary, paths) = fixture();
        fs::write(
            paths.codex_home.join("config.toml"),
            br#"
[marketplaces.missing]
source_type = "local"
source = "Z:\\missing"

[plugins."stale@missing"]
enabled = true
"#,
        )
        .unwrap();

        let runtime = prepare(&paths).unwrap();
        let config = fs::read_to_string(runtime.home.join("config.toml")).unwrap();
        assert!(!config.contains("stale@missing"));
        assert!(config.contains("browser@openai-bundled"));
        assert!(config.contains("computer-use@openai-bundled"));
        validate_official_bundled(
            &runtime
                .home
                .join(".tmp/bundled-marketplaces/openai-bundled"),
        )
        .unwrap();
    }

    #[test]
    fn runtime_config_rebases_portable_marketplace_after_drive_change() {
        let (_temporary, paths) = fixture();
        let snapshot = portable_plugin_snapshot(&paths);
        fs::write(
            paths.codex_home.join("config.toml"),
            br#"
[marketplaces.yunshu_plugins]
source_type = "local"
source = "G:\\old-workstation\\payloads\\plugins\\openai-curated\\snapshot-current"

[plugins."actively@yunshu_plugins"]
enabled = true
"#,
        )
        .unwrap();

        let runtime = prepare(&paths).unwrap();
        let config = fs::read_to_string(runtime.home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert!(config.contains("actively@yunshu_plugins"));
        assert_eq!(
            parsed["marketplaces"][MARKETPLACE_NAME]["source"].as_str(),
            Some(snapshot.to_string_lossy().as_ref())
        );
        assert!(!config.contains("G:\\\\old-workstation"));
    }

    #[test]
    fn runtime_config_migrates_previous_windows_sandbox_defaults() {
        let (_temporary, paths) = fixture();
        fs::write(
            paths.codex_home.join("config.toml"),
            br#"default_permissions = ":workspace"
approval_policy = "on-request"
sandbox_mode = "workspace-write"
approvals_reviewer = "user"

[windows]
sandbox = "elevated"
"#,
        )
        .unwrap();

        let runtime = prepare(&paths).unwrap();
        let config = fs::read_to_string(runtime.home.join("config.toml")).unwrap();
        assert!(config.contains("default_permissions = \":danger-full-access\""));
        assert!(config.contains("approval_policy = \"never\""));
        assert!(config.contains("sandbox_mode = \"danger-full-access\""));
        assert!(config.contains("sandbox = \"unelevated\""));
        assert!(!config.contains("sandbox = \"elevated\""));
    }

    #[test]
    fn normal_stop_writes_runtime_changes_back_to_portable_master() {
        let (_temporary, paths) = fixture();
        fs::write(paths.codex_home.join("config.toml"), b"model = 'before'\n").unwrap();
        let runtime = prepare(&paths).unwrap();
        fs::write(
            runtime.home.join("config.toml"),
            b"model = 'after'\napproval_policy = 'never'\n",
        )
        .unwrap();
        fs::write(runtime.browser_data.join("Local State"), b"after").unwrap();

        let summary = sync_to_portable(&paths).unwrap();
        assert!(summary.copied_files >= 1);
        let portable_config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(portable_config.contains("model = 'before'"));
        assert!(portable_config.contains("approval_policy = 'never'"));
        assert_eq!(
            fs::read(paths.codex_browser_data.join("Local State")).unwrap(),
            b"after"
        );
        assert!(!paths.codex_machine_dirty_marker().exists());
    }

    #[test]
    fn normal_stop_persists_complete_portable_permission_profile() {
        let (_temporary, paths) = fixture();
        fs::write(paths.codex_home.join("config.toml"), b"model = 'before'\n").unwrap();
        let runtime = prepare(&paths).unwrap();
        fs::write(
            runtime.home.join("config.toml"),
            br#"model = "runtime-only"
default_permissions = ":danger-full-access"
approval_policy = "never"
sandbox_mode = "danger-full-access"
approvals_reviewer = "user"

[windows]
sandbox = "unelevated"
"#,
        )
        .unwrap();

        sync_to_portable(&paths).unwrap();
        let config = fs::read_to_string(paths.codex_home.join("config.toml")).unwrap();
        assert!(config.contains("model = 'before'"));
        assert!(config.contains("default_permissions = \":danger-full-access\""));
        assert!(config.contains("approvals_reviewer = \"user\""));
        assert!(config.contains("sandbox = \"unelevated\""));
    }
}
