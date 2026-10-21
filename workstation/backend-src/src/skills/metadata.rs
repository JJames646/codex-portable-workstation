use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use getrandom::fill;
use serde::{Deserialize, Serialize};

use crate::model::SkillTarget;
use crate::paths::WorkstationPaths;
use crate::windows_process::replace_file_atomic;

pub const SKILLS_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDescriptor {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub relative_root: String,
    pub package_sha256: String,
    pub dependencies: DependencySummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencySummary {
    pub requirements_txt: bool,
    pub pyproject_toml: bool,
    pub package_json: bool,
    pub package_lock: bool,
    pub pnpm_lock: bool,
    pub yarn_lock: bool,
}

impl DependencySummary {
    pub fn any(&self) -> bool {
        self.requirements_txt
            || self.pyproject_toml
            || self.package_json
            || self.package_lock
            || self.pnpm_lock
            || self.yarn_lock
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpec {
    pub kind: String,
    pub reference: String,
    pub display_name: String,
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub coordinate: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub signature_key_id: Option<String>,
    #[serde(default)]
    pub signature_verified: bool,
    #[serde(default)]
    pub target_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub created_at_unix_ms: u128,
    pub source: SourceSpec,
    pub targets: Vec<SkillTarget>,
    pub install_dependencies: bool,
    pub skills: Vec<SkillDescriptor>,
    pub staging_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecord {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub target: SkillTarget,
    pub source: String,
    pub package_sha256: String,
    pub relative_path: String,
    pub installed_at_unix_ms: u128,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub coordinate: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub signature_key_id: Option<String>,
    #[serde(default)]
    pub signature_verified: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRegistry {
    pub schema_version: u32,
    pub records: Vec<SkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionAction {
    pub slug: String,
    pub target: SkillTarget,
    pub target_path: String,
    pub backup_path: String,
    pub temporary_path: String,
    pub had_previous: bool,
    pub committed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionState {
    pub schema_version: u32,
    pub transaction_id: String,
    pub kind: String,
    pub status: String,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub registry_backup: String,
    pub actions: Vec<TransactionAction>,
    pub message: Option<String>,
}

pub fn read_registry(paths: &WorkstationPaths) -> Result<SkillRegistry> {
    if !paths.skills_registry.is_file() {
        return Ok(SkillRegistry {
            schema_version: SKILLS_SCHEMA_VERSION,
            records: Vec::new(),
        });
    }
    let bytes = fs::read(&paths.skills_registry)
        .with_context(|| format!("无法读取 {}", paths.skills_registry.display()))?;
    let mut registry: SkillRegistry = serde_json::from_slice(&bytes)
        .with_context(|| format!("无法解析 {}", paths.skills_registry.display()))?;
    if registry.schema_version == 0 {
        registry.schema_version = SKILLS_SCHEMA_VERSION;
    }
    if registry.schema_version > SKILLS_SCHEMA_VERSION {
        anyhow::bail!(
            "Skills registry schemaVersion过新：{} > {}",
            registry.schema_version,
            SKILLS_SCHEMA_VERSION
        );
    }
    Ok(registry)
}

pub fn write_registry(paths: &WorkstationPaths, registry: &SkillRegistry) -> Result<()> {
    write_json_atomic(&paths.skills_registry, registry)
}

pub fn write_plan(paths: &WorkstationPaths, plan: &InstallPlan) -> Result<()> {
    write_json_atomic(&plan_path(paths, &plan.plan_id), plan)
}

pub fn read_plan(paths: &WorkstationPaths, plan_id: &str) -> Result<InstallPlan> {
    let path = plan_path(paths, plan_id);
    let bytes = fs::read(&path).with_context(|| format!("无法读取安装计划 {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("无法解析安装计划 {}", path.display()))
}

pub fn plan_path(paths: &WorkstationPaths, plan_id: &str) -> std::path::PathBuf {
    paths
        .skills_transactions
        .join(format!("{plan_id}.plan.json"))
}

pub fn transaction_path(paths: &WorkstationPaths, transaction_id: &str) -> std::path::PathBuf {
    paths
        .skills_transactions
        .join(format!("{transaction_id}.json"))
}

pub fn write_transaction(paths: &WorkstationPaths, state: &TransactionState) -> Result<()> {
    write_json_atomic(&transaction_path(paths, &state.transaction_id), state)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("JSON目标缺少父目录")?;
    fs::create_dir_all(parent).with_context(|| format!("无法创建 {}", parent.display()))?;
    let temporary = path.with_extension("tmp");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).with_context(|| format!("无法写入 {}", temporary.display()))?;
    replace_file_atomic(&temporary, path)
}

pub fn new_operation_id(prefix: &str) -> Result<String> {
    let mut random = [0_u8; 6];
    fill(&mut random).map_err(|error| anyhow::anyhow!("无法生成事务ID：{error}"))?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{prefix}-{}-{suffix}", now_unix_ms()))
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
