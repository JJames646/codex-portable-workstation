use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use getrandom::fill;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::Engine;

#[derive(Debug, Clone)]
pub struct WorkstationPaths {
    pub root: PathBuf,
    pub codex_desktop: PathBuf,
    pub codex_cli: PathBuf,
    pub claude_exe: PathBuf,
    pub openclaw_node: PathBuf,
    pub openclaw_entry: PathBuf,
    pub bundled_node: PathBuf,
    pub bundled_python: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub provider_config: PathBuf,
    pub secrets: PathBuf,
    pub codex_browser_data: PathBuf,
    pub codex_home: PathBuf,
    pub claude_home: PathBuf,
    pub openclaw_home: PathBuf,
    pub workspace: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub backups: PathBuf,
    pub backend: PathBuf,
    pub manifests: PathBuf,
    pub runtime_state: PathBuf,
    pub bridge_state: PathBuf,
    pub skills_config: PathBuf,
    pub skills_data: PathBuf,
    pub skills_registry: PathBuf,
    pub skills_transactions: PathBuf,
    pub skills_cache: PathBuf,
    pub skills_catalog_cache: PathBuf,
    pub skills_download_cache: PathBuf,
    pub skills_skillhub_cache: PathBuf,
    pub skills_staging: PathBuf,
    pub skills_backups: PathBuf,
    pub plugins_config: PathBuf,
    pub plugins_sources: PathBuf,
    pub plugins_catalog: PathBuf,
    pub plugins_i18n: PathBuf,
    pub plugins_payloads: PathBuf,
    pub plugins_data: PathBuf,
    pub plugins_registry: PathBuf,
    pub plugins_transactions: PathBuf,
    pub plugins_cache: PathBuf,
    pub plugins_staging: PathBuf,
    pub plugins_backups: PathBuf,
    pub plugins_install_root: PathBuf,
    pub plugins_ui: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedLayout {
    pub root: PathBuf,
    pub created_or_verified: Vec<PathBuf>,
}

impl WorkstationPaths {
    pub fn resolve(explicit_root: Option<PathBuf>) -> Result<Self> {
        let root = match explicit_root {
            Some(path) => absolute_path(path)?,
            None => infer_root_from_current_exe()?,
        };
        Ok(Self::from_root(root))
    }

    pub fn from_root(root: PathBuf) -> Self {
        let data = root.join("data");
        let cache = root.join("cache");
        let config = root.join("config");
        let provider_config = config.join("providers");
        let secrets = data.join("secrets");
        let runtime_state = data.join("runtime");
        let skills_config = config.join("skills");
        let skills_data = data.join("skills");
        let skills_cache = cache.join("skills");
        let plugins_config = config.join("plugins");
        let plugins_catalog = root.join("catalogs").join("plugins");
        let plugins_data = data.join("plugins");
        let plugins_cache = cache.join("plugins");
        Self {
            codex_desktop: root.join("codex").join("app").join("ChatGPT.exe"),
            codex_cli: root
                .join("codex")
                .join("app")
                .join("resources")
                .join("codex.exe"),
            claude_exe: root
                .join("claude")
                .join("node_modules")
                .join("@anthropic-ai")
                .join("claude-code")
                .join("bin")
                .join("claude.exe"),
            openclaw_node: root
                .join("openclaw")
                .join("app")
                .join("runtime")
                .join("node-win-x64")
                .join("node.exe"),
            openclaw_entry: root
                .join("openclaw")
                .join("app")
                .join("core")
                .join("node_modules")
                .join("openclaw")
                .join("openclaw.mjs"),
            bundled_node: root.join("runtimes").join("node").join("node.exe"),
            bundled_python: root.join("runtimes").join("python").join("python.exe"),
            config,
            provider_config,
            secrets,
            codex_browser_data: data.join("Codex"),
            codex_home: data.join(".codex"),
            claude_home: data.join(".claude"),
            openclaw_home: data.join(".openclaw"),
            workspace: root.join("workspace"),
            logs: root.join("logs"),
            backups: root.join("backups"),
            backend: root.join("backend"),
            manifests: root.join("manifests"),
            bridge_state: runtime_state.join("bridge.json"),
            skills_registry: skills_data.join("registry.json"),
            skills_transactions: skills_data.join("transactions"),
            skills_catalog_cache: skills_cache.join("catalogs"),
            skills_download_cache: skills_cache.join("downloads"),
            skills_skillhub_cache: skills_cache.join("skillhub"),
            skills_staging: skills_cache.join("staging"),
            skills_backups: root.join("backups").join("skills"),
            skills_config,
            skills_data,
            skills_cache,
            plugins_sources: plugins_config.join("sources.json"),
            plugins_i18n: plugins_catalog.join("plugin-i18n.zh-CN.json"),
            plugins_payloads: root.join("payloads").join("plugins"),
            plugins_registry: plugins_data.join("registry.json"),
            plugins_transactions: plugins_data.join("transactions"),
            plugins_staging: plugins_cache.join("staging"),
            plugins_backups: root.join("backups").join("plugins"),
            plugins_install_root: data
                .join(".codex")
                .join("plugins")
                .join("cache")
                .join("yunshu_plugins"),
            plugins_ui: root.join("ui").join("plugins"),
            plugins_config,
            plugins_catalog,
            plugins_data,
            plugins_cache,
            runtime_state,
            data,
            cache,
            root,
        }
    }

    pub fn prepare_layout(&self) -> Result<PreparedLayout> {
        let directories = vec![
            self.data.clone(),
            self.config.clone(),
            self.provider_config.clone(),
            self.secrets.clone(),
            self.codex_browser_data.clone(),
            self.codex_home.clone(),
            self.claude_home.clone(),
            self.openclaw_home.clone(),
            self.workspace.clone(),
            self.cache.clone(),
            self.cache.join("claude"),
            self.cache.join("npm"),
            self.cache.join("openclaw-node"),
            self.logs.clone(),
            self.backups.clone(),
            self.backend.clone(),
            self.manifests.clone(),
            self.runtime_state.clone(),
            self.skills_config.clone(),
            self.skills_data.clone(),
            self.skills_transactions.clone(),
            self.skills_cache.clone(),
            self.skills_catalog_cache.clone(),
            self.skills_download_cache.clone(),
            self.skills_skillhub_cache.clone(),
            self.skills_staging.clone(),
            self.skills_backups.clone(),
            self.codex_home.join("skills"),
            self.claude_home.join("skills"),
            self.openclaw_home.join("skills"),
            self.plugins_config.clone(),
            self.plugins_catalog.clone(),
            self.plugins_payloads.clone(),
            self.plugins_data.clone(),
            self.plugins_transactions.clone(),
            self.plugins_cache.clone(),
            self.plugins_staging.clone(),
            self.plugins_backups.clone(),
            self.plugins_install_root.clone(),
            self.plugins_ui.clone(),
        ];
        for directory in &directories {
            fs::create_dir_all(directory)
                .with_context(|| format!("无法创建便携目录 {}", directory.display()))?;
        }
        self.ensure_installation_id()?;
        Ok(PreparedLayout {
            root: self.root.clone(),
            created_or_verified: directories,
        })
    }

    /// 只准备当前智能体启动所需的目录。
    ///
    /// 完整布局包含 Skills、插件、教程缓存等大量目录。引擎启动路径不应触碰这些目录，
    /// 否则在大分配单元的移动盘上会把一次简单启动放大成大量小文件元数据操作。
    pub fn prepare_engine_layout(&self, engine: Engine) -> Result<PreparedLayout> {
        let mut directories = vec![
            self.data.clone(),
            self.config.clone(),
            self.provider_config.clone(),
            self.secrets.clone(),
            self.workspace.clone(),
            self.logs.clone(),
            self.backups.clone(),
            self.backend.clone(),
            self.manifests.clone(),
            self.runtime_state.clone(),
        ];
        match engine {
            Engine::Codex => {
                directories.push(self.codex_browser_data.clone());
                directories.push(self.codex_home.clone());
            }
            Engine::Claude => {
                directories.push(self.claude_home.clone());
                directories.push(self.cache.join("claude"));
                directories.push(self.cache.join("npm"));
            }
            Engine::OpenClaw => {
                directories.push(self.openclaw_home.clone());
                directories.push(self.cache.join("openclaw-node"));
            }
        }
        for directory in &directories {
            fs::create_dir_all(directory)
                .with_context(|| format!("无法创建便携目录 {}", directory.display()))?;
        }
        self.ensure_installation_id()?;
        Ok(PreparedLayout {
            root: self.root.clone(),
            created_or_verified: directories,
        })
    }

    pub fn ensure_payload_exists(&self) -> Result<()> {
        let required = [
            &self.codex_desktop,
            &self.codex_cli,
            &self.claude_exe,
            &self.openclaw_node,
            &self.openclaw_entry,
            &self.bundled_node,
            &self.bundled_python,
        ];
        let missing: Vec<_> = required
            .into_iter()
            .filter(|path| !path.is_file())
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let joined = missing
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("；");
        bail!("三引擎主体不完整：{joined}")
    }

    pub fn state_file(&self, engine_slug: &str) -> PathBuf {
        if engine_slug.eq_ignore_ascii_case("codex") {
            return self
                .codex_machine_generation_root()
                .join("runtime")
                .join(format!("{engine_slug}.json"));
        }
        if engine_slug.eq_ignore_ascii_case("openclaw") {
            return self
                .machine_engine_cache(engine_slug)
                .join("runtime")
                .join(format!("{engine_slug}.json"));
        }
        self.runtime_state.join(format!("{engine_slug}.json"))
    }

    pub fn stop_request_file(&self, engine_slug: &str) -> PathBuf {
        if engine_slug.eq_ignore_ascii_case("codex") {
            return self
                .codex_machine_generation_root()
                .join("runtime")
                .join(format!("{engine_slug}.stop"));
        }
        if engine_slug.eq_ignore_ascii_case("openclaw") {
            return self
                .machine_engine_cache(engine_slug)
                .join("runtime")
                .join(format!("{engine_slug}.stop"));
        }
        self.runtime_state.join(format!("{engine_slug}.stop"))
    }

    /// OpenClaw 的程序文件预先打成单个 ZIP 放在移动盘。
    /// 新电脑首次运行时只需顺序读取这个文件，再解压到本机 NTFS，避免通过
    /// 虚拟 USB/exFAT 逐个读取数万个小文件。
    pub fn openclaw_payload_archive(&self) -> PathBuf {
        self.root
            .join("openclaw")
            .join("app")
            .join("cache")
            .join("openclaw-payload-v1.zip")
    }

    pub fn provider_file(&self, engine_slug: &str) -> PathBuf {
        self.provider_config.join(format!("{engine_slug}.json"))
    }

    pub fn vault_file(&self) -> PathBuf {
        self.secrets.join("provider-vault.bin")
    }

    pub fn vault_key_file(&self) -> PathBuf {
        self.secrets.join("vault.key")
    }

    /// 返回当前电脑上的易失运行缓存目录。
    ///
    /// 会话、项目、Skills、插件和供应商配置仍保存在移动盘；这里只有 Chromium
    /// 磁盘缓存、引擎 stdout/stderr 和 OpenClaw 的实时锁文件。缓存键同时包含安装ID、
    /// 移动卷标识和卷内相对路径指纹：同一移动盘变更盘符后仍可复用，从 E 盘复制到 G 盘的
    /// 快照则使用独立缓存，避免运行状态和监督 Runtime 串用。
    pub fn machine_cache_root(&self) -> PathBuf {
        let base = local_cache_base();
        let installation_id = self
            .read_installation_id()
            .unwrap_or_else(|| fallback_cache_id(&self.root));
        let volume_id = volume_identity(&self.root).unwrap_or_else(|| "no-volume".to_string());
        let root_scope = portable_root_scope(&self.root);
        let cache_id = format!("{installation_id}-{volume_id}-{root_scope}");
        base.join("YunshuAI")
            .join("PortableWorkstation")
            .join(cache_id)
    }

    /// OpenClaw 的程序载荷按内容指纹存放，不包含会话、API、日志等用户数据。
    /// 因此它可以在同一台电脑上的正式盘和只读快照之间安全复用，避免每份快照
    /// 都重新复制数万个小文件。可变运行状态仍继续使用 `machine_cache_root` 隔离。
    pub fn shared_openclaw_payload_root(&self) -> PathBuf {
        local_cache_base()
            .join("YunshuAI")
            .join("PortableWorkstation")
            .join("SharedRuntime")
            .join("openclaw")
            .join("payload")
    }

    /// 返回启用盘卷隔离之前使用的 OpenClaw 载荷缓存位置。
    /// 这里只用于只读兼容已有的完整不可变载荷，不在该目录写入新状态。
    pub fn legacy_openclaw_payload_root(&self) -> Option<PathBuf> {
        let installation_id = self.read_installation_id()?;
        Some(
            local_cache_base()
                .join("YunshuAI")
                .join("PortableWorkstation")
                .join(installation_id)
                .join("openclaw")
                .join("payload"),
        )
    }

    pub fn machine_engine_cache(&self, engine_slug: &str) -> PathBuf {
        self.machine_cache_root().join(engine_slug)
    }

    fn codex_machine_generation_root(&self) -> PathBuf {
        let legacy = self.machine_engine_cache("codex");
        match self.read_factory_generation_id() {
            Some(generation_id) => legacy.join("generations").join(generation_id),
            None => legacy,
        }
    }

    pub fn codex_machine_home(&self) -> PathBuf {
        self.codex_machine_generation_root()
            .join("working")
            .join(".codex")
    }

    pub fn codex_machine_browser_data(&self) -> PathBuf {
        self.codex_machine_generation_root()
            .join("working")
            .join("browser")
    }

    pub fn codex_machine_dirty_marker(&self) -> PathBuf {
        self.codex_machine_generation_root()
            .join("mirror-dirty.json")
    }

    pub fn codex_machine_last_sync_marker(&self) -> PathBuf {
        self.codex_machine_generation_root().join("last-sync.json")
    }

    pub fn factory_generation_file(&self) -> PathBuf {
        self.data.join("factory-generation-id")
    }

    pub fn read_factory_generation_id(&self) -> Option<String> {
        let value = fs::read_to_string(self.factory_generation_file()).ok()?;
        let value = value.trim().to_ascii_lowercase();
        (value.len() == 32 && value.chars().all(|character| character.is_ascii_hexdigit()))
            .then_some(value)
    }

    pub fn rotate_factory_generation_id(&self) -> Result<String> {
        fs::create_dir_all(&self.data)?;
        let mut random = [0_u8; 16];
        fill(&mut random).map_err(|error| anyhow::anyhow!("无法生成出厂缓存代际：{error}"))?;
        let value = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = self.factory_generation_file();
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, format!("{value}\n"))?;
        crate::windows_process::replace_file_atomic(&temporary, &path)
            .context("无法写入出厂缓存代际")?;
        Ok(value)
    }

    pub fn installation_id_file(&self) -> PathBuf {
        self.data.join("installation-id")
    }

    pub fn read_installation_id(&self) -> Option<String> {
        let value = fs::read_to_string(self.installation_id_file()).ok()?;
        let value = value.trim().to_ascii_lowercase();
        (value.len() == 32 && value.chars().all(|character| character.is_ascii_hexdigit()))
            .then_some(value)
    }

    pub fn ensure_installation_id(&self) -> Result<String> {
        if let Some(value) = self.read_installation_id() {
            return Ok(value);
        }
        fs::create_dir_all(&self.data)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seed = format!(
            "{}:{}:{}",
            self.root.display(),
            std::process::id(),
            timestamp
        );
        let digest = Sha256::digest(seed.as_bytes());
        let value = digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = self.installation_id_file();
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, format!("{value}\n"))?;
        match fs::rename(&temporary, &path) {
            Ok(()) => Ok(value),
            Err(error) if path.is_file() => {
                let _ = fs::remove_file(temporary);
                self.read_installation_id()
                    .ok_or_else(|| anyhow::anyhow!("工作站安装ID并发创建失败：{error}"))
            }
            Err(error) => Err(error).context("无法创建工作站安装ID"),
        }
    }

    pub fn bridge_stop_file(&self) -> PathBuf {
        self.runtime_state.join("bridge.stop")
    }

    pub fn skill_target_dir(&self, target: crate::model::SkillTarget) -> PathBuf {
        match target {
            crate::model::SkillTarget::Codex => self.codex_home.join("skills"),
            crate::model::SkillTarget::Claude => self.claude_home.join("skills"),
            crate::model::SkillTarget::OpenClaw => self.openclaw_home.join("skills"),
        }
    }
}

#[cfg(windows)]
pub fn available_disk_space(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("无法读取磁盘可用空间 {}", path.display()));
    }
    Ok(available)
}

#[cfg(not(windows))]
pub fn available_disk_space(_path: &Path) -> Result<u64> {
    // 出厂恢复只在 Win11 交付；非 Windows 构建用于单元测试和前端联调。
    Ok(u64::MAX)
}

fn fallback_cache_id(root: &Path) -> String {
    let normalized = root.to_string_lossy().replace('/', "\\").to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn local_cache_base() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
}

/// 仅使用卷内相对路径生成指纹，不把 Windows 盘符编入缓存键。
/// 因此 `E:\\dev2` 变成 `F:\\dev2` 时仍是同一个 scope，而
/// `G:\\YunshuSnapshots\\...` 会与开发盘分离。
fn portable_root_scope(root: &Path) -> String {
    let normalized = root.to_string_lossy().replace('/', "\\").to_lowercase();
    let bytes = normalized.as_bytes();
    let relative = if bytes.len() >= 2 && bytes[1] == b':' {
        &normalized[2..]
    } else {
        normalized.as_str()
    };
    let digest = Sha256::digest(relative.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(windows)]
fn volume_identity(root: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let volume_root = root.ancestors().last()?;
    let wide = volume_root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut serial = 0_u32;
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    (ok != 0).then(|| format!("{serial:08x}"))
}

#[cfg(not(windows))]
fn volume_identity(_root: &Path) -> Option<String> {
    None
}

fn infer_root_from_current_exe() -> Result<PathBuf> {
    let executable = env::current_exe().context("无法读取当前运行程序路径")?;
    let executable_dir = executable
        .parent()
        .context("当前运行程序没有父目录")?
        .to_path_buf();
    let root = if executable_dir
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("backend"))
    {
        executable_dir
            .parent()
            .context("backend目录缺少工作站根目录")?
            .to_path_buf()
    } else {
        executable_dir
    };
    absolute_path(root)
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(normalize_without_io(&path));
    }
    let current = env::current_dir().context("无法读取当前工作目录")?;
    Ok(normalize_without_io(&current.join(path)))
}

fn normalize_without_io(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_paths_follow_the_workstation_root() {
        let root = PathBuf::from(r"X:\中文 路径\dev2");
        let paths = WorkstationPaths::from_root(root.clone());
        assert_eq!(
            paths.codex_desktop,
            root.join("codex").join("app").join("ChatGPT.exe")
        );
        assert_eq!(paths.workspace, root.join("workspace"));
        assert_eq!(paths.codex_home, root.join("data").join(".codex"));
        assert_eq!(
            paths.skill_target_dir(crate::model::SkillTarget::Claude),
            root.join("data").join(".claude").join("skills")
        );
        assert_eq!(
            paths.plugins_install_root,
            root.join("data")
                .join(".codex")
                .join("plugins")
                .join("cache")
                .join("yunshu_plugins")
        );
        assert_eq!(
            paths.plugins_payloads,
            root.join("payloads").join("plugins")
        );
    }

    #[test]
    fn portable_root_scope_ignores_drive_letter_but_separates_snapshot_folder() {
        let development = portable_root_scope(Path::new(r"E:\dev2"));
        let moved_drive = portable_root_scope(Path::new(r"F:\dev2"));
        let snapshot = portable_root_scope(Path::new(
            r"G:\YunshuSnapshots\dev2-stable-v4-20260803-205624",
        ));

        assert_eq!(development, moved_drive);
        assert_ne!(development, snapshot);
    }

    #[test]
    fn codex_launch_layout_does_not_prepare_other_engines_or_extensions() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 路径").join("dev2"));

        let prepared = paths.prepare_engine_layout(Engine::Codex).unwrap();

        assert!(paths.codex_browser_data.is_dir());
        assert!(paths.codex_home.is_dir());
        assert!(!paths.claude_home.exists());
        assert!(!paths.openclaw_home.exists());
        assert!(!paths.plugins_data.exists());
        assert!(!paths.skills_data.exists());
        assert!(prepared.created_or_verified.contains(&paths.codex_home));
        assert!(paths.data.join("installation-id").is_file());
        assert!(
            paths
                .state_file("codex")
                .starts_with(paths.machine_engine_cache("codex"))
        );
        assert!(
            paths
                .stop_request_file("codex")
                .starts_with(paths.machine_engine_cache("codex"))
        );
        assert!(
            paths
                .state_file("openclaw")
                .starts_with(paths.machine_engine_cache("openclaw"))
        );
        assert!(
            paths
                .stop_request_file("openclaw")
                .starts_with(paths.machine_engine_cache("openclaw"))
        );
        assert_eq!(
            paths.openclaw_payload_archive(),
            paths
                .root
                .join("openclaw/app/cache/openclaw-payload-v1.zip")
        );
    }
}
