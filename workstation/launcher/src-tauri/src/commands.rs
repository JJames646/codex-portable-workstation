use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use url::Url;
use yunshu_workstation_runtime::bridge;
use yunshu_workstation_runtime::codex_proxy::{
    self, CodexProxyNodeSelectInput, CodexProxyStatus, CodexProxySubscriptionScanInput,
};
use yunshu_workstation_runtime::locale_init::{
    self, CodexLocaleApplyResult, CodexLocaleInitializationStatus, CodexLocaleSeedState,
};
use yunshu_workstation_runtime::model::{Engine, SkillTarget};
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::plugins;
use yunshu_workstation_runtime::provider::{
    self, ConfigureProvider, DiscoverProviderModels, ProviderModels, ProviderProtocol,
    ProviderWorkspace,
};
use yunshu_workstation_runtime::runtime;
use yunshu_workstation_runtime::skills;
use zeroize::Zeroizing;

use crate::error::{CommandError, CommandResult};
use crate::tutorials;

#[derive(Debug, Clone)]
pub struct AppState {
    pub paths: WorkstationPaths,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstationBootstrap {
    pub version: &'static str,
    pub paths: WorkstationPathSummary,
    pub detections: Vec<yunshu_workstation_runtime::model::EngineDetection>,
    pub states: Vec<yunshu_workstation_runtime::model::EngineState>,
    pub providers: Vec<provider::ProviderStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstationPathSummary {
    pub root: String,
    pub data: String,
    pub workspace: String,
    pub cache: String,
    pub logs: String,
    pub backups: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInput {
    pub engine: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsInput {
    pub default: String,
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
    pub subagent: Option<String>,
    pub available: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWorkspaceInput {
    pub choose_on_launch: bool,
    pub relative_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSaveInput {
    pub engine: String,
    pub display_name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub protocol: String,
    pub models: ProviderModelsInput,
    pub workspace: Option<ProviderWorkspaceInput>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDiscoverInput {
    pub engine: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub protocol: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub engine: Engine,
    pub configured: bool,
    pub enabled: bool,
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    pub models: Option<ProviderModels>,
    pub protocol: Option<ProviderProtocol>,
    pub resolved_protocol: Option<ProviderProtocol>,
    pub workspace: Option<ProviderWorkspace>,
    pub credential_present: bool,
    pub applied: bool,
    pub timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillQueryInput {
    pub query: Option<String>,
    pub category: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub sort_by: Option<String>,
    pub offline: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReferenceInput {
    pub reference: String,
    pub offline: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallInput {
    pub reference: String,
    pub target: String,
    pub install_dependencies: Option<bool>,
    pub offline: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLocalSourceInput {
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUninstallInput {
    pub slug: String,
    pub target: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginQueryInput {
    pub query: Option<String>,
    pub category: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInput {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBatchInput {
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBatchItem {
    pub id: String,
    pub success: bool,
    pub result: Option<plugins::installer::PluginInstallResult>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialInput {
    pub engine: String,
    pub file_name: Option<String>,
    pub asset_path: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferences {
    pub reduce_motion: bool,
    pub offline_first: bool,
    pub compact_mode: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub level: &'static str,
    pub published_at: &'static str,
}

#[tauri::command]
pub async fn workstation_bootstrap(
    state: State<'_, AppState>,
) -> CommandResult<WorkstationBootstrap> {
    let workstation_paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<WorkstationBootstrap> {
        // 首页只准备默认 Codex 和公共目录。Skills、插件、教程及另外两个引擎的
        // 专用目录均由对应页面/操作按需准备，避免冷启动触碰大量移动盘元数据。
        workstation_paths.prepare_engine_layout(Engine::Codex)?;
        let paths = path_summary(&workstation_paths);
        let detections = runtime::detect(&workstation_paths, None);
        let states = runtime::status_all(&workstation_paths)?;
        let providers = provider::status_all(&workstation_paths)?;
        Ok(WorkstationBootstrap {
            version: env!("CARGO_PKG_VERSION"),
            paths,
            detections,
            states,
            providers,
        })
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn workstation_paths(state: State<'_, AppState>) -> WorkstationPathSummary {
    path_summary(&state.paths)
}

#[tauri::command]
pub fn workstation_version() -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "runtimeVersion": env!("CARGO_PKG_VERSION"),
        "channel": "phase5-internal"
    })
}

#[tauri::command]
pub fn workstation_diagnose(state: State<'_, AppState>) -> CommandResult<runtime::DiagnoseResult> {
    runtime::diagnose(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn engine_detect_all(
    state: State<'_, AppState>,
) -> Vec<yunshu_workstation_runtime::model::EngineDetection> {
    runtime::detect(&state.paths, None)
}

#[tauri::command]
pub async fn engine_status_all(
    state: State<'_, AppState>,
) -> CommandResult<Vec<yunshu_workstation_runtime::model::EngineState>> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || runtime::status_all(&paths))
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn engine_probe(
    state: State<'_, AppState>,
    input: EngineInput,
) -> CommandResult<runtime::ProbeResult> {
    let engine = parse_engine(&input.engine)?;
    runtime::probe(&state.paths, engine).map_err(CommandError::from)
}

#[tauri::command]
pub async fn engine_start(
    state: State<'_, AppState>,
    input: EngineInput,
) -> CommandResult<yunshu_workstation_runtime::model::EngineState> {
    let engine = parse_engine(&input.engine)?;
    provider::apply(&state.paths, engine).map_err(CommandError::from)?;
    if provider::requires_bridge(&state.paths, engine).map_err(CommandError::from)? {
        bridge::ensure_running(&state.paths)
            .await
            .map_err(CommandError::from)?;
    }
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_state = runtime::start(&paths, engine)?;
        if engine == Engine::OpenClaw {
            runtime::open_openclaw_dashboard(&paths)
        } else {
            Ok(runtime_state)
        }
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn openclaw_open_control(
    state: State<'_, AppState>,
) -> CommandResult<yunshu_workstation_runtime::model::EngineState> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || runtime::open_openclaw_dashboard(&paths))
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn engine_stop(
    state: State<'_, AppState>,
    input: EngineInput,
) -> CommandResult<yunshu_workstation_runtime::model::EngineState> {
    let engine = parse_engine(&input.engine)?;
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        runtime::stop(&paths, engine, Duration::from_secs(75))
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn engine_stop_all(state: State<'_, AppState>) -> CommandResult<Value> {
    let paths = state.paths.clone();
    let engines = tauri::async_runtime::spawn_blocking(move || {
        runtime::stop_all(&paths, Duration::from_secs(75))
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)?;
    let bridge = bridge::stop(&state.paths)
        .await
        .map_err(CommandError::from)?;
    Ok(json!({"engines": engines, "bridge": bridge}))
}

#[tauri::command]
pub fn codex_locale_status(
    state: State<'_, AppState>,
) -> CommandResult<CodexLocaleInitializationStatus> {
    locale_init::initialization_status(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub async fn codex_locale_apply_seed(
    state: State<'_, AppState>,
) -> CommandResult<CodexLocaleApplyResult> {
    let paths = state.paths.clone();
    let seed = locale_init::inspect_seed(&paths);
    if seed.state != CodexLocaleSeedState::Ready {
        return Err(CommandError::from_message(seed.message));
    }
    tauri::async_runtime::spawn_blocking({
        let paths = paths.clone();
        move || -> Result<()> {
            let current = runtime::status(&paths, Engine::Codex)?;
            let active = !matches!(
                current.status,
                yunshu_workstation_runtime::model::RuntimeStatus::Stopped
                    | yunshu_workstation_runtime::model::RuntimeStatus::Failed
            );
            if active {
                bail!("请先停止 Codex，再执行本地中文初始化");
            }
            Ok(())
        }
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)?;

    let result = tauri::async_runtime::spawn_blocking({
        let paths = paths.clone();
        move || locale_init::apply_seed(&paths)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)?;

    Ok(result)
}

#[tauri::command]
pub fn codex_proxy_status(state: State<'_, AppState>) -> CommandResult<CodexProxyStatus> {
    codex_proxy::status(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub async fn codex_proxy_subscription_scan(
    state: State<'_, AppState>,
    input: CodexProxySubscriptionScanInput,
) -> CommandResult<CodexProxyStatus> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || codex_proxy::scan_subscription(&paths, input))
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn codex_proxy_node_select(
    state: State<'_, AppState>,
    input: CodexProxyNodeSelectInput,
) -> CommandResult<CodexProxyStatus> {
    codex_proxy::select_node(&state.paths, input).map_err(CommandError::from)
}

#[tauri::command]
pub async fn codex_proxy_start_once(
    state: State<'_, AppState>,
) -> CommandResult<yunshu_workstation_runtime::model::EngineState> {
    let paths = state.paths.clone();
    let current = tauri::async_runtime::spawn_blocking({
        let paths = paths.clone();
        move || runtime::status(&paths, Engine::Codex)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)?;
    if !matches!(
        current.status,
        yunshu_workstation_runtime::model::RuntimeStatus::Stopped
            | yunshu_workstation_runtime::model::RuntimeStatus::Failed
    ) {
        tauri::async_runtime::spawn_blocking({
            let paths = paths.clone();
            move || runtime::stop(&paths, Engine::Codex, Duration::from_secs(75))
        })
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)?;
    }
    let seed = locale_init::inspect_seed(&paths);
    if seed.state != CodexLocaleSeedState::Ready {
        return Err(CommandError::from_message(seed.message));
    }
    tauri::async_runtime::spawn_blocking({
        let paths = paths.clone();
        move || locale_init::apply_seed(&paths)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)?;
    provider::apply(&paths, Engine::Codex).map_err(CommandError::from)?;
    if provider::requires_bridge(&paths, Engine::Codex).map_err(CommandError::from)? {
        bridge::ensure_running(&paths)
            .await
            .map_err(CommandError::from)?;
    }
    codex_proxy::mark_start_once(&paths).map_err(CommandError::from)?;
    let start_result = tauri::async_runtime::spawn_blocking({
        let paths = paths.clone();
        move || runtime::start(&paths, Engine::Codex)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?;
    match start_result {
        Ok(state) => Ok(state),
        Err(error) => {
            let _ = codex_proxy::disable(&paths);
            Err(CommandError::from(error))
        }
    }
}

#[tauri::command]
pub fn codex_proxy_enable_persistent(
    state: State<'_, AppState>,
) -> CommandResult<CodexProxyStatus> {
    let current = runtime::status(&state.paths, Engine::Codex).map_err(CommandError::from)?;
    if !matches!(
        current.status,
        yunshu_workstation_runtime::model::RuntimeStatus::Stopped
            | yunshu_workstation_runtime::model::RuntimeStatus::Failed
    ) {
        return Err(CommandError::from_message(
            "请先停止 Codex，再启用持续代理模式".to_string(),
        ));
    }
    codex_proxy::enable_persistent(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub async fn codex_proxy_disable(state: State<'_, AppState>) -> CommandResult<CodexProxyStatus> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<CodexProxyStatus> {
        let current = runtime::status(&paths, Engine::Codex)?;
        if !matches!(
            current.status,
            yunshu_workstation_runtime::model::RuntimeStatus::Stopped
                | yunshu_workstation_runtime::model::RuntimeStatus::Failed
        ) {
            runtime::stop(&paths, Engine::Codex, Duration::from_secs(75))?;
        }
        codex_proxy::disable(&paths)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn codex_proxy_clear(state: State<'_, AppState>) -> CommandResult<CodexProxyStatus> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<CodexProxyStatus> {
        if codex_proxy::status(&paths)?.active {
            let current = runtime::status(&paths, Engine::Codex)?;
            if !matches!(
                current.status,
                yunshu_workstation_runtime::model::RuntimeStatus::Stopped
                    | yunshu_workstation_runtime::model::RuntimeStatus::Failed
            ) {
                runtime::stop(&paths, Engine::Codex, Duration::from_secs(75))?;
            }
        }
        codex_proxy::clear(&paths)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn provider_get(state: State<'_, AppState>, input: EngineInput) -> CommandResult<ProviderView> {
    provider_view(&state.paths, parse_engine(&input.engine)?)
}

#[tauri::command]
pub fn provider_get_all(state: State<'_, AppState>) -> CommandResult<Vec<ProviderView>> {
    Engine::ALL
        .into_iter()
        .map(|engine| provider_view(&state.paths, engine))
        .collect()
}

#[tauri::command]
pub fn provider_save(
    state: State<'_, AppState>,
    input: ProviderSaveInput,
) -> CommandResult<provider::ProviderStatus> {
    let engine = parse_engine(&input.engine)?;
    let protocol = ProviderProtocol::from_str(&input.protocol).map_err(CommandError::from)?;
    let models = ProviderModels {
        available: if engine == Engine::Codex {
            input.models.available.unwrap_or_default()
        } else {
            Vec::new()
        },
        default: input.models.default.trim().to_string(),
        opus: optional_model(input.models.opus, &input.models.default),
        sonnet: optional_model(input.models.sonnet, &input.models.default),
        haiku: optional_model(input.models.haiku, &input.models.default),
        subagent: input
            .models
            .subagent
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "inherit".to_string()),
    };
    let workspace = input.workspace.map(|workspace| ProviderWorkspace {
        choose_on_launch: workspace.choose_on_launch,
        relative_path: workspace.relative_path,
    });
    provider::configure(
        &state.paths,
        ConfigureProvider {
            engine,
            display_name: input.display_name,
            base_url: input.base_url,
            model: models.default.clone(),
            models: Some(models),
            protocol,
            workspace,
            timeout_seconds: input.timeout_seconds.unwrap_or(120),
            api_key: input
                .api_key
                .filter(|value| !value.trim().is_empty())
                .map(Zeroizing::new),
        },
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn provider_discover_models(
    state: State<'_, AppState>,
    input: ProviderDiscoverInput,
) -> CommandResult<provider::ModelDiscoveryResult> {
    provider::discover_models(
        &state.paths,
        DiscoverProviderModels {
            engine: parse_engine(&input.engine)?,
            base_url: input.base_url,
            protocol: ProviderProtocol::from_str(&input.protocol).map_err(CommandError::from)?,
            timeout_seconds: input.timeout_seconds.unwrap_or(30),
            api_key: input
                .api_key
                .filter(|value| !value.trim().is_empty())
                .map(Zeroizing::new),
        },
    )
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn provider_test(
    state: State<'_, AppState>,
    input: EngineInput,
) -> CommandResult<bridge::ProviderTestResult> {
    bridge::test_provider(&state.paths, parse_engine(&input.engine)?)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn provider_apply(
    state: State<'_, AppState>,
    input: EngineInput,
) -> CommandResult<provider::ProviderStatus> {
    let engine = parse_engine(&input.engine)?;
    provider::apply(&state.paths, engine).map_err(CommandError::from)?;
    provider::status(&state.paths, engine).map_err(CommandError::from)
}

#[tauri::command]
pub async fn bridge_status(state: State<'_, AppState>) -> CommandResult<bridge::BridgeState> {
    bridge::status(&state.paths)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn skills_categories(
    state: State<'_, AppState>,
    offline: Option<bool>,
) -> CommandResult<skills::skillhub::SkillHubQueryResult> {
    skills::skillhub::categories(&state.paths, offline.unwrap_or(false))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn skills_recommended(
    state: State<'_, AppState>,
    offline: Option<bool>,
) -> CommandResult<skills::skillhub::SkillHubQueryResult> {
    skills::skillhub::recommended(&state.paths, offline.unwrap_or(false))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn skills_search(
    state: State<'_, AppState>,
    input: SkillQueryInput,
) -> CommandResult<skills::skillhub::SkillHubQueryResult> {
    skills::skillhub::search(
        &state.paths,
        input.query.as_deref(),
        input.category.as_deref(),
        input.page.unwrap_or(1),
        input.page_size.unwrap_or(24),
        input.sort_by.as_deref().unwrap_or("downloads"),
        input.offline.unwrap_or(false),
    )
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn skills_detail(
    state: State<'_, AppState>,
    input: SkillReferenceInput,
) -> CommandResult<skills::skillhub::SkillHubQueryResult> {
    skills::skillhub::detail(
        &state.paths,
        &input.reference,
        input.offline.unwrap_or(false),
    )
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn skills_install(
    state: State<'_, AppState>,
    input: SkillInstallInput,
) -> CommandResult<skills::transaction::InstallResult> {
    let target = SkillTarget::from_str(&input.target).map_err(CommandError::from)?;
    let plan = skills::transaction::create_plan_with_mode(
        &state.paths,
        &input.reference,
        vec![target],
        input.install_dependencies.unwrap_or(false),
        input.offline.unwrap_or(false),
    )
    .await
    .map_err(CommandError::from)?;
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        skills::transaction::install_plan(&paths, &plan.plan_id)
    })
    .await
    .map_err(|error| CommandError::from_message(error.to_string()))?
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn skills_inspect_local(
    input: SkillLocalSourceInput,
) -> CommandResult<skills::transaction::SkillInspection> {
    let source = PathBuf::from(input.source);
    skills::transaction::inspect_local(&source).map_err(CommandError::from)
}

#[tauri::command]
pub fn skills_list_installed(
    state: State<'_, AppState>,
) -> CommandResult<Vec<skills::transaction::SkillStatus>> {
    skills::transaction::list_status(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn skills_uninstall(
    state: State<'_, AppState>,
    input: SkillUninstallInput,
) -> CommandResult<skills::transaction::UninstallResult> {
    skills::transaction::uninstall(
        &state.paths,
        &input.slug,
        vec![SkillTarget::from_str(&input.target).map_err(CommandError::from)?],
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn skills_repair(
    state: State<'_, AppState>,
) -> CommandResult<skills::transaction::RepairResult> {
    skills::transaction::repair(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn plugins_status(state: State<'_, AppState>) -> CommandResult<plugins::PluginCenterStatus> {
    plugins::status(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn plugins_list(
    state: State<'_, AppState>,
    input: PluginQueryInput,
) -> CommandResult<plugins::catalog::PluginCatalogResponse> {
    plugins::catalog::list(
        &state.paths,
        input.query.as_deref(),
        input.category.as_deref(),
        input.page.unwrap_or(1),
        input.page_size.unwrap_or(24),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn plugins_detail(
    state: State<'_, AppState>,
    input: PluginInput,
) -> CommandResult<plugins::catalog::PluginCatalogItem> {
    plugins::catalog::detail(&state.paths, &input.id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn plugins_install(
    state: State<'_, AppState>,
    input: PluginInput,
) -> CommandResult<plugins::installer::PluginInstallResult> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || plugins::installer::install(&paths, &input.id))
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn plugins_install_batch(
    state: State<'_, AppState>,
    input: PluginBatchInput,
) -> CommandResult<Vec<PluginBatchItem>> {
    let mut results = Vec::new();
    for id in input.ids {
        let paths = state.paths.clone();
        let install_id = id.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            plugins::installer::install(&paths, &install_id)
        })
        .await
        .map_err(|error| CommandError::from_message(error.to_string()))?;
        match result {
            Ok(result) => results.push(PluginBatchItem {
                id,
                success: true,
                result: Some(result),
                error: None,
            }),
            Err(error) => results.push(PluginBatchItem {
                id,
                success: false,
                result: None,
                error: Some(error.to_string()),
            }),
        }
    }
    Ok(results)
}

#[tauri::command]
pub fn plugins_installed(
    state: State<'_, AppState>,
) -> CommandResult<plugins::installer::PluginRegistry> {
    plugins::installer::read_registry(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn plugins_repair(state: State<'_, AppState>) -> CommandResult<Value> {
    plugins::installer::repair(&state.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn tutorial_list(
    state: State<'_, AppState>,
    input: TutorialInput,
) -> CommandResult<Vec<tutorials::TutorialEntry>> {
    tutorials::list(&state.paths, parse_engine(&input.engine)?).map_err(CommandError::from)
}

#[tauri::command]
pub fn tutorial_read(
    state: State<'_, AppState>,
    input: TutorialInput,
) -> CommandResult<tutorials::TutorialDocument> {
    let file_name = input
        .file_name
        .ok_or_else(|| CommandError::from_message("缺少教程文件名"))?;
    tutorials::read(&state.paths, parse_engine(&input.engine)?, &file_name)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn tutorial_asset_read(
    state: State<'_, AppState>,
    input: TutorialInput,
) -> CommandResult<tutorials::TutorialAsset> {
    let asset_path = input
        .asset_path
        .ok_or_else(|| CommandError::from_message("缺少教程图片路径"))?;
    tutorials::read_asset(&state.paths, parse_engine(&input.engine)?, &asset_path)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn tutorial_search(
    state: State<'_, AppState>,
    input: TutorialInput,
) -> CommandResult<Vec<tutorials::TutorialSearchHit>> {
    tutorials::search(
        &state.paths,
        parse_engine(&input.engine)?,
        input.query.as_deref().unwrap_or_default(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn ui_preferences_load(state: State<'_, AppState>) -> CommandResult<UiPreferences> {
    let path = preferences_path(&state.paths);
    if !path.is_file() {
        return Ok(UiPreferences::default());
    }
    serde_json::from_slice(&fs::read(&path).map_err(CommandError::from)?)
        .map_err(|error| CommandError::from_message(error.to_string()))
}

#[tauri::command]
pub fn ui_preferences_save(
    state: State<'_, AppState>,
    preferences: UiPreferences,
) -> CommandResult<UiPreferences> {
    let path = preferences_path(&state.paths);
    write_json_atomic(&path, &preferences).map_err(CommandError::from)?;
    Ok(preferences)
}

#[tauri::command]
pub fn logs_open(state: State<'_, AppState>) -> CommandResult<()> {
    open_local_path(&state.paths.logs).map_err(CommandError::from)
}

#[tauri::command]
pub fn backups_open(state: State<'_, AppState>) -> CommandResult<()> {
    open_local_path(&state.paths.backups).map_err(CommandError::from)
}

#[tauri::command]
pub fn open_external_url(url: String) -> CommandResult<()> {
    open_whitelisted_url(&url).map_err(CommandError::from)
}

#[tauri::command]
pub fn announcement_list() -> Vec<Announcement> {
    vec![Announcement {
        id: "phase5-local-default",
        title: "云枢AI便携工作站",
        body: "公告服务接口已预留；当前显示移动盘内置公告。",
        level: "info",
        published_at: "2026-07-31",
    }]
}

fn provider_view(paths: &WorkstationPaths, engine: Engine) -> CommandResult<ProviderView> {
    let status = provider::status(paths, engine).map_err(CommandError::from)?;
    let config = provider::load(paths, engine).ok();
    Ok(ProviderView {
        engine,
        configured: status.configured,
        enabled: status.enabled,
        display_name: status.display_name,
        base_url: status.base_url,
        models: status.models,
        protocol: status.protocol,
        resolved_protocol: status.resolved_protocol,
        workspace: status.workspace,
        credential_present: status.credential_present,
        applied: status.applied,
        timeout_seconds: config.map(|value| value.timeout_seconds).unwrap_or(120),
    })
}

fn parse_engine(value: &str) -> CommandResult<Engine> {
    Engine::from_str(value).map_err(CommandError::from)
}

fn optional_model(value: Option<String>, fallback: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.trim().to_string())
}

fn path_summary(paths: &WorkstationPaths) -> WorkstationPathSummary {
    WorkstationPathSummary {
        root: paths.root.display().to_string(),
        data: paths.data.display().to_string(),
        workspace: paths.workspace.display().to_string(),
        cache: paths.cache.display().to_string(),
        logs: paths.logs.display().to_string(),
        backups: paths.backups.display().to_string(),
    }
}

fn preferences_path(paths: &WorkstationPaths) -> PathBuf {
    paths.data.join("ui").join("preferences.json")
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("偏好文件缺少父目录")?;
    fs::create_dir_all(parent).with_context(|| format!("无法创建偏好目录 {}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)
        .with_context(|| format!("无法写入偏好文件 {}", temporary.display()))?;
    yunshu_workstation_runtime::windows_process::replace_file_atomic(&temporary, path)
}

fn open_whitelisted_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw.trim()).context("外部链接格式不正确")?;
    if url.scheme() != "https" || url.username() != "" || url.password().is_some() {
        bail!("只允许打开不含凭据的HTTPS链接");
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !is_whitelisted_external_host(&host) {
        bail!("该链接不在工作站服务白名单中");
    }
    open_target(url.as_str())
}

fn is_whitelisted_external_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "sub2api.jackchen6660.dpdns.org",
        "go1.jackchen6660.dpdns.org",
        "go2.jackchen6660.dpdns.org",
        "skillhub.cn",
        "www.skillhub.cn",
        "sibker.com",
        "www.sibker.com",
    ];
    HOSTS.contains(&host)
}

fn open_local_path(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("无法创建目录 {}", path.display()))?;
    open_target(&path.display().to_string())
}

fn open_target(target: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(target);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(target);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(target);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("无法打开 {target}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_rejects_unknown_host_and_embedded_credentials() {
        assert!(open_whitelisted_url("https://example.com").is_err());
        assert!(open_whitelisted_url("https://user:secret@skillhub.cn/catalog").is_err());
    }

    #[test]
    fn external_url_allows_go1_service_host() {
        assert!(is_whitelisted_external_host("go1.jackchen6660.dpdns.org"));
    }

    #[test]
    fn external_url_allows_go2_service_host() {
        assert!(is_whitelisted_external_host("go2.jackchen6660.dpdns.org"));
    }

    #[test]
    fn provider_view_never_contains_secret_fields() {
        let encoded = serde_json::to_string(&ProviderView {
            engine: Engine::Codex,
            configured: false,
            enabled: false,
            display_name: None,
            base_url: None,
            models: None,
            protocol: None,
            resolved_protocol: None,
            workspace: None,
            credential_present: false,
            applied: false,
            timeout_seconds: 120,
        })
        .unwrap();
        assert!(!encoded.contains("apiKey"));
        assert!(!encoded.contains("credentialRef"));
        assert!(!encoded.contains("localToken"));
    }
}
