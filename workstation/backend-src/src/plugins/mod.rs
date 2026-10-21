//! Codex 专用插件中心。
//!
//! Skills 是三个智能体共享的能力包；本模块只管理 Codex 插件。插件主体优先从移动盘的
//! 固定快照读取，安装时复制到便携 `CODEX_HOME`，不会读取或修改本机用户目录。

pub mod catalog;
pub mod installer;
pub mod server;

use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::WorkstationPaths;
use crate::skills::metadata::write_json_atomic;

pub const MARKETPLACE_NAME: &str = "yunshu_plugins";
pub const PLUGINS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSources {
    pub schema_version: u32,
    pub mode: String,
    pub marketplace: String,
    pub payload_root: String,
    pub remote_catalog_url: Option<String>,
    pub remote_payload_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCenterStatus {
    pub scope: &'static str,
    pub marketplace: &'static str,
    pub payload_ready: bool,
    pub payload_snapshot: Option<String>,
    pub catalog_count: usize,
    pub installed_count: usize,
    pub source_mode: String,
    pub ui_ready: bool,
    pub codex_configured: bool,
}

pub fn prepare(paths: &WorkstationPaths) -> Result<()> {
    for directory in [
        &paths.plugins_config,
        &paths.plugins_catalog,
        &paths.plugins_payloads,
        &paths.plugins_data,
        &paths.plugins_transactions,
        &paths.plugins_cache,
        &paths.plugins_staging,
        &paths.plugins_backups,
        &paths.plugins_install_root,
        &paths.plugins_ui,
    ] {
        fs::create_dir_all(directory)
            .with_context(|| format!("无法创建插件目录 {}", directory.display()))?;
    }

    if !paths.plugins_sources.is_file() {
        write_json_atomic(
            &paths.plugins_sources,
            &PluginSources {
                schema_version: PLUGINS_SCHEMA_VERSION,
                mode: "local-first".to_string(),
                marketplace: MARKETPLACE_NAME.to_string(),
                payload_root: "../../payloads/plugins".to_string(),
                remote_catalog_url: None,
                remote_payload_url: None,
            },
        )?;
    }
    if !paths.plugins_i18n.is_file() {
        write_json_atomic(
            &paths.plugins_i18n,
            &serde_json::json!({
                "schemaVersion": PLUGINS_SCHEMA_VERSION,
                "locale": "zh-CN",
                "plugins": {}
            }),
        )?;
    }

    // 新移动盘第一次部署插件快照前，prepare 仍应允许三个智能体正常启动。
    if let Ok(snapshot) = catalog::locate_snapshot(paths) {
        catalog::ensure_portable_marketplace_metadata(&snapshot)?;
        installer::ensure_codex_marketplace(paths, &snapshot)?;
    }
    installer::repair(paths)?;
    Ok(())
}

pub fn status(paths: &WorkstationPaths) -> Result<PluginCenterStatus> {
    let snapshot = catalog::locate_snapshot(paths).ok();
    let catalog_count = snapshot
        .as_ref()
        .and_then(|_| catalog::list(paths, None, None, 1, u32::MAX).ok())
        .map(|response| response.total)
        .unwrap_or_default();
    let installed_count = installer::read_registry(paths)?.records.len();
    let source_mode = fs::read(&paths.plugins_sources)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PluginSources>(&bytes).ok())
        .map(|source| source.mode)
        .unwrap_or_else(|| "local-first".to_string());
    let codex_configured = snapshot
        .as_ref()
        .is_some_and(|snapshot| installer::codex_marketplace_is_current(paths, snapshot));
    Ok(PluginCenterStatus {
        scope: "codex-only",
        marketplace: MARKETPLACE_NAME,
        payload_ready: snapshot.is_some(),
        payload_snapshot: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.file_name())
            .map(|value| value.to_string_lossy().into_owned()),
        catalog_count,
        installed_count,
        source_mode,
        ui_ready: paths.plugins_ui.join("index.html").is_file(),
        codex_configured,
    })
}
