use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::model::SkillTarget;
use crate::paths::WorkstationPaths;
use crate::windows_process::replace_file_atomic;

use super::dependencies::{DependencyInstallResult, install_dependencies};
use super::metadata::{
    InstallPlan, SKILLS_SCHEMA_VERSION, SkillRecord, SourceSpec, TransactionAction,
    TransactionState, new_operation_id, now_unix_ms, read_plan, read_registry, write_json_atomic,
    write_plan, write_registry, write_transaction,
};
use super::package::{
    hash_directory, inspect_source, path_from_slash, stage_skillhub_source, stage_source,
};
use super::{catalog, skillhub};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInspection {
    pub source: String,
    pub skills: Vec<super::metadata::SkillDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub transaction_id: String,
    pub records: Vec<SkillRecord>,
    pub dependencies: Vec<DependencyInstallResult>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStatus {
    pub record: SkillRecord,
    pub present: bool,
    pub actual_sha256: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallResult {
    pub transaction_id: String,
    pub removed: Vec<SkillRecord>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub repaired_transactions: Vec<String>,
    pub skipped_transactions: Vec<String>,
    pub lock_removed: bool,
}

pub fn inspect_local(source: &Path) -> Result<SkillInspection> {
    Ok(SkillInspection {
        source: source.display().to_string(),
        skills: inspect_source(source)?,
    })
}

pub async fn create_plan(
    paths: &WorkstationPaths,
    source: &str,
    targets: Vec<SkillTarget>,
    install_dependencies_flag: bool,
) -> Result<InstallPlan> {
    create_plan_with_mode(paths, source, targets, install_dependencies_flag, false).await
}

pub async fn create_plan_with_mode(
    paths: &WorkstationPaths,
    source: &str,
    targets: Vec<SkillTarget>,
    install_dependencies_flag: bool,
    offline: bool,
) -> Result<InstallPlan> {
    paths.prepare_layout()?;
    super::catalog::ensure_default_config(paths)?;
    let targets = normalize_targets(targets)?;
    let plan_id = new_operation_id("skill-plan")?;
    let mut skillhub_package = None;
    let (source_path, source_spec, allowed_targets) = if source.starts_with("skillhub:") {
        let download = skillhub::download_package_with_mode(paths, source, offline).await?;
        let source_path = download.path.clone();
        let source_spec = SourceSpec {
            kind: "skillhub".to_string(),
            reference: download.coordinate.clone(),
            display_name: format!(
                "{}（@{}/{}@{}）",
                download.display_name, download.namespace, download.slug, download.version
            ),
            expected_sha256: Some(download.package_sha256.clone()),
            source_id: Some("skillhub".to_string()),
            coordinate: Some(download.coordinate.clone()),
            namespace: Some(download.namespace.clone()),
            content_hash: Some(download.content_hash.clone()),
            signature_key_id: Some(download.signature_key_id.clone()),
            signature_verified: download.signature_verified,
            target_directory: Some(download.target_directory.clone()),
        };
        skillhub_package = Some(download);
        (source_path, source_spec, Some(SkillTarget::ALL.to_vec()))
    } else if source.starts_with("catalog:") {
        let (download, item, catalog_id) = catalog::download_package(paths, source).await?;
        (
            download,
            SourceSpec {
                kind: "catalog".to_string(),
                reference: source.to_string(),
                display_name: format!("{catalog_id}/{}@{}", item.slug, item.version),
                expected_sha256: Some(item.sha256),
                source_id: Some(catalog_id.to_string()),
                coordinate: Some(source.to_string()),
                namespace: None,
                content_hash: None,
                signature_key_id: None,
                signature_verified: true,
                target_directory: None,
            },
            Some(item.targets),
        )
    } else {
        let source_path = PathBuf::from(source);
        (
            source_path.clone(),
            SourceSpec {
                kind: if source_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("zip"))
                {
                    "local-zip".to_string()
                } else {
                    "local-file".to_string()
                },
                reference: source_path.display().to_string(),
                display_name: source_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("local-skill")
                    .to_string(),
                expected_sha256: None,
                source_id: None,
                coordinate: None,
                namespace: None,
                content_hash: None,
                signature_key_id: None,
                signature_verified: false,
                target_directory: None,
            },
            None,
        )
    };
    if let Some(allowed) = allowed_targets {
        for target in &targets {
            if !allowed.contains(target) {
                bail!("Catalog Skill不支持安装到目标：{target}");
            }
        }
    }

    let staging_root = paths.skills_staging.join(&plan_id);
    let mut staged = if skillhub_package.is_some() {
        stage_skillhub_source(&source_path, &staging_root)?
    } else {
        stage_source(&source_path, &staging_root)?
    };
    if let Some(download) = &skillhub_package {
        if staged.skills.len() != 1 {
            bail!(
                "SkillHub包必须且只能包含一个Skill，实际为{}个",
                staged.skills.len()
            );
        }
        let descriptor = &mut staged.skills[0];
        descriptor.slug = download.slug.clone();
        descriptor.name = download.display_name.clone();
        descriptor.version = download.version.clone();
        if !download.description_zh.is_empty() {
            descriptor.description = download.description_zh.clone();
        }
    }
    let plan = InstallPlan {
        schema_version: SKILLS_SCHEMA_VERSION,
        plan_id,
        created_at_unix_ms: now_unix_ms(),
        source: source_spec,
        targets,
        install_dependencies: install_dependencies_flag,
        skills: staged.skills,
        staging_root: relative_to_root(paths, &staged.staging_root)?,
    };
    write_plan(paths, &plan)?;
    Ok(plan)
}

pub fn install_plan(paths: &WorkstationPaths, plan_id: &str) -> Result<InstallResult> {
    paths.prepare_layout()?;
    let _lock = SkillsLock::acquire(paths)?;
    let plan = read_plan(paths, plan_id)?;
    if plan.schema_version != SKILLS_SCHEMA_VERSION {
        bail!("不支持的Skill计划schemaVersion：{}", plan.schema_version);
    }
    let staging_root = resolve_relative_root(paths, &plan.staging_root)?;
    if !staging_root.starts_with(&paths.skills_staging) {
        bail!("Skill计划的staging路径越界");
    }
    let transaction_id = new_operation_id("skill-install")?;
    let backup_root = paths.skills_backups.join(&transaction_id);
    fs::create_dir_all(&backup_root)?;
    let registry_backup = backup_root.join("registry.before.json");
    backup_registry(paths, &registry_backup)?;

    let mut state = TransactionState {
        schema_version: SKILLS_SCHEMA_VERSION,
        transaction_id: transaction_id.clone(),
        kind: "install".to_string(),
        status: "installing".to_string(),
        created_at_unix_ms: now_unix_ms(),
        updated_at_unix_ms: now_unix_ms(),
        registry_backup: relative_to_root(paths, &registry_backup)?,
        actions: Vec::new(),
        message: None,
    };
    write_transaction(paths, &state)?;

    let mut registry = read_registry(paths)?;
    let mut installed_records = Vec::new();
    let mut dependency_results = Vec::new();
    let result = (|| -> Result<()> {
        for skill in &plan.skills {
            validate_slug(&skill.slug)?;
            let staged_skill = staging_root.join(path_from_slash(&skill.relative_root));
            if !staged_skill.join("SKILL.md").is_file() {
                bail!("安装计划的staging Skill不存在：{}", staged_skill.display());
            }
            if plan.install_dependencies && skill.dependencies.any() {
                dependency_results.push(install_dependencies(
                    paths,
                    &staged_skill,
                    &skill.dependencies,
                )?);
            }
            let package_hash = hash_directory(&staged_skill)?;
            for target in &plan.targets {
                let target_parent = paths.skill_target_dir(*target);
                fs::create_dir_all(&target_parent)?;
                let target_directory = plan
                    .source
                    .target_directory
                    .as_deref()
                    .unwrap_or(&skill.slug);
                validate_slug(target_directory)?;
                let target_path = target_parent.join(target_directory);
                let temporary_path =
                    target_parent.join(format!(".{target_directory}.staging-{transaction_id}"));
                let backup_path = backup_root.join(target.slug()).join(target_directory);
                let had_previous = target_path.exists();
                let action = TransactionAction {
                    slug: skill.slug.clone(),
                    target: *target,
                    target_path: relative_to_root(paths, &target_path)?,
                    backup_path: relative_to_root(paths, &backup_path)?,
                    temporary_path: relative_to_root(paths, &temporary_path)?,
                    had_previous,
                    committed: false,
                };
                state.actions.push(action);
                state.updated_at_unix_ms = now_unix_ms();
                write_transaction(paths, &state)?;

                remove_path_if_exists(&temporary_path)?;
                if had_previous {
                    if let Some(parent) = backup_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    remove_path_if_exists(&backup_path)?;
                    fs::rename(&target_path, &backup_path).with_context(|| {
                        format!(
                            "无法备份现有Skill {} -> {}",
                            target_path.display(),
                            backup_path.display()
                        )
                    })?;
                }
                super::package::copy_tree(&staged_skill, &temporary_path)?;
                write_installed_metadata(
                    &temporary_path,
                    skill,
                    &plan.source,
                    *target,
                    &package_hash,
                )?;
                fs::rename(&temporary_path, &target_path).with_context(|| {
                    format!(
                        "无法提交Skill {} -> {}",
                        temporary_path.display(),
                        target_path.display()
                    )
                })?;
                if let Some(last) = state.actions.last_mut() {
                    last.committed = true;
                }
                state.updated_at_unix_ms = now_unix_ms();
                write_transaction(paths, &state)?;

                let record = SkillRecord {
                    slug: skill.slug.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    version: skill.version.clone(),
                    target: *target,
                    source: plan.source.kind.clone(),
                    package_sha256: package_hash.clone(),
                    relative_path: relative_to_root(paths, &target_path)?,
                    installed_at_unix_ms: now_unix_ms(),
                    source_id: plan.source.source_id.clone(),
                    coordinate: plan.source.coordinate.clone(),
                    namespace: plan.source.namespace.clone(),
                    content_hash: plan.source.content_hash.clone(),
                    signature_key_id: plan.source.signature_key_id.clone(),
                    signature_verified: plan.source.signature_verified,
                };
                registry.records.retain(|existing| {
                    let same_identity = match (&record.coordinate, &existing.coordinate) {
                        (Some(record), Some(existing)) => record == existing,
                        _ => existing.slug == record.slug,
                    };
                    !(same_identity && existing.target == *target)
                });
                registry.records.push(record.clone());
                installed_records.push(record);
            }
        }
        registry.schema_version = SKILLS_SCHEMA_VERSION;
        write_registry(paths, &registry)?;
        Ok(())
    })();

    if let Err(error) = result {
        state.status = "rolling-back".to_string();
        state.message = Some(error.to_string());
        state.updated_at_unix_ms = now_unix_ms();
        let _ = write_transaction(paths, &state);
        let rollback_result = rollback_transaction(paths, &state);
        state.status = if rollback_result.is_ok() {
            "rolled-back".to_string()
        } else {
            "rollback-failed".to_string()
        };
        if let Err(rollback_error) = rollback_result {
            state.message = Some(format!("{error}; 回滚失败：{rollback_error}"));
        }
        state.updated_at_unix_ms = now_unix_ms();
        let _ = write_transaction(paths, &state);
        return Err(error);
    }

    state.status = "committed".to_string();
    state.updated_at_unix_ms = now_unix_ms();
    state.message = Some(format!("已安装{}条目标记录", installed_records.len()));
    write_transaction(paths, &state)?;
    remove_path_if_exists(&staging_root)?;
    Ok(InstallResult {
        transaction_id,
        records: installed_records,
        dependencies: dependency_results,
        status: "committed".to_string(),
    })
}

pub fn list_status(paths: &WorkstationPaths) -> Result<Vec<SkillStatus>> {
    let registry = read_registry(paths)?;
    registry
        .records
        .into_iter()
        .map(|record| {
            let path = resolve_relative_root(paths, &record.relative_path)?;
            let actual_sha256 = path.is_dir().then(|| hash_directory(&path)).transpose()?;
            let verified = actual_sha256
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(&record.package_sha256));
            Ok(SkillStatus {
                present: path.is_dir(),
                actual_sha256,
                verified,
                record,
            })
        })
        .collect()
}

pub fn verify_skill(paths: &WorkstationPaths, slug: &str) -> Result<Vec<SkillStatus>> {
    let matches = list_status(paths)?
        .into_iter()
        .filter(|status| {
            status.record.slug == slug
                || status
                    .record
                    .coordinate
                    .as_deref()
                    .is_some_and(|value| value == slug)
        })
        .collect::<Vec<_>>();
    if matches.is_empty() {
        bail!("未安装Skill：{slug}");
    }
    Ok(matches)
}

pub fn uninstall(
    paths: &WorkstationPaths,
    slug: &str,
    targets: Vec<SkillTarget>,
) -> Result<UninstallResult> {
    validate_skill_selector(slug)?;
    paths.prepare_layout()?;
    let _lock = SkillsLock::acquire(paths)?;
    let targets = normalize_targets(targets)?;
    let transaction_id = new_operation_id("skill-uninstall")?;
    let backup_root = paths.skills_backups.join(&transaction_id);
    fs::create_dir_all(&backup_root)?;
    let registry_backup = backup_root.join("registry.before.json");
    backup_registry(paths, &registry_backup)?;
    let mut registry = read_registry(paths)?;
    let removed = registry
        .records
        .iter()
        .filter(|record| {
            (record.slug == slug
                || record
                    .coordinate
                    .as_deref()
                    .is_some_and(|value| value == slug))
                && targets.contains(&record.target)
        })
        .cloned()
        .collect::<Vec<_>>();
    if removed.is_empty() {
        bail!("指定目标中没有安装Skill：{slug}");
    }
    let mut state = TransactionState {
        schema_version: SKILLS_SCHEMA_VERSION,
        transaction_id: transaction_id.clone(),
        kind: "uninstall".to_string(),
        status: "uninstalling".to_string(),
        created_at_unix_ms: now_unix_ms(),
        updated_at_unix_ms: now_unix_ms(),
        registry_backup: relative_to_root(paths, &registry_backup)?,
        actions: Vec::new(),
        message: None,
    };
    write_transaction(paths, &state)?;

    let result = (|| -> Result<()> {
        for record in &removed {
            let target_path = resolve_relative_root(paths, &record.relative_path)?;
            let target_name = target_path.file_name().context("Skill目标缺少目录名")?;
            let backup_path = backup_root.join(record.target.slug()).join(target_name);
            let action = TransactionAction {
                slug: slug.to_string(),
                target: record.target,
                target_path: record.relative_path.clone(),
                backup_path: relative_to_root(paths, &backup_path)?,
                temporary_path: String::new(),
                had_previous: target_path.exists(),
                committed: false,
            };
            state.actions.push(action);
            write_transaction(paths, &state)?;
            if target_path.exists() {
                fs::create_dir_all(backup_path.parent().context("备份目录缺少父目录")?)?;
                fs::rename(&target_path, &backup_path)?;
            }
            if let Some(last) = state.actions.last_mut() {
                last.committed = true;
            }
            state.updated_at_unix_ms = now_unix_ms();
            write_transaction(paths, &state)?;
        }
        registry.records.retain(|record| {
            !((record.slug == slug
                || record
                    .coordinate
                    .as_deref()
                    .is_some_and(|value| value == slug))
                && targets.contains(&record.target))
        });
        write_registry(paths, &registry)
    })();
    if let Err(error) = result {
        let _ = rollback_transaction(paths, &state);
        state.status = "rolled-back".to_string();
        state.message = Some(error.to_string());
        state.updated_at_unix_ms = now_unix_ms();
        let _ = write_transaction(paths, &state);
        return Err(error);
    }
    state.status = "committed".to_string();
    state.updated_at_unix_ms = now_unix_ms();
    write_transaction(paths, &state)?;
    Ok(UninstallResult {
        transaction_id,
        removed,
        status: "committed".to_string(),
    })
}

pub fn repair(paths: &WorkstationPaths) -> Result<RepairResult> {
    paths.prepare_layout()?;
    let lock_path = paths.skills_data.join("install.lock");
    if lock_path.is_file() {
        let owner = fs::read_to_string(&lock_path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        if let Some(process_id) = owner
            && process_id != std::process::id()
            && crate::windows_process::process_is_alive(process_id)
        {
            bail!("Skills事务仍由活动进程PID={process_id}持有，拒绝强制repair");
        }
    }
    let mut result = RepairResult {
        repaired_transactions: Vec::new(),
        skipped_transactions: Vec::new(),
        lock_removed: false,
    };
    for entry in fs::read_dir(&paths.skills_transactions)? {
        let entry = entry?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".json") && !name.ends_with(".plan.json"))
        {
            continue;
        }
        let bytes = fs::read(&path)?;
        let mut state: TransactionState = match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(_) => {
                result.skipped_transactions.push(path.display().to_string());
                continue;
            }
        };
        if matches!(
            state.status.as_str(),
            "installing" | "uninstalling" | "rolling-back" | "rollback-failed"
        ) {
            rollback_transaction(paths, &state)?;
            state.status = "rolled-back".to_string();
            state.updated_at_unix_ms = now_unix_ms();
            state.message = Some("prepare/repair已恢复未完成事务".to_string());
            write_transaction(paths, &state)?;
            result
                .repaired_transactions
                .push(state.transaction_id.clone());
        } else {
            result
                .skipped_transactions
                .push(state.transaction_id.clone());
        }
    }
    if lock_path.exists() {
        fs::remove_file(&lock_path)?;
        result.lock_removed = true;
    }
    Ok(result)
}

fn rollback_transaction(paths: &WorkstationPaths, state: &TransactionState) -> Result<()> {
    for action in state.actions.iter().rev() {
        let target = resolve_relative_root(paths, &action.target_path)?;
        let backup = resolve_relative_root(paths, &action.backup_path)?;
        if !action.temporary_path.is_empty() {
            let temporary = resolve_relative_root(paths, &action.temporary_path)?;
            remove_path_if_exists(&temporary)?;
        }
        if backup.exists() {
            remove_path_if_exists(&target)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&backup, &target).with_context(|| {
                format!(
                    "无法恢复Skill备份 {} -> {}",
                    backup.display(),
                    target.display()
                )
            })?;
        } else if action.committed && !action.had_previous {
            remove_path_if_exists(&target)?;
        } else if action.committed && action.had_previous {
            bail!(
                "事务记录显示已覆盖旧Skill，但备份不存在：{}",
                backup.display()
            );
        }
    }
    let registry_backup = resolve_relative_root(paths, &state.registry_backup)?;
    restore_registry(paths, &registry_backup)
}

fn backup_registry(paths: &WorkstationPaths, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if paths.skills_registry.is_file() {
        fs::copy(&paths.skills_registry, destination)?;
    } else {
        fs::write(
            destination,
            serde_json::to_vec_pretty(&super::metadata::SkillRegistry {
                schema_version: SKILLS_SCHEMA_VERSION,
                records: Vec::new(),
            })?,
        )?;
    }
    Ok(())
}

fn restore_registry(paths: &WorkstationPaths, backup: &Path) -> Result<()> {
    if !backup.is_file() {
        bail!("事务缺少registry备份：{}", backup.display());
    }
    if let Some(parent) = paths.skills_registry.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = paths.skills_registry.with_extension("restore.tmp");
    fs::copy(backup, &temporary)?;
    replace_file_atomic(&temporary, &paths.skills_registry)
}

fn write_installed_metadata(
    root: &Path,
    skill: &super::metadata::SkillDescriptor,
    source: &SourceSpec,
    target: SkillTarget,
    package_hash: &str,
) -> Result<()> {
    fs::write(root.join(".skill-source"), format!("{}\n", source.kind))?;
    write_json_atomic(
        &root.join("_meta.json"),
        &serde_json::json!({
            "schemaVersion": SKILLS_SCHEMA_VERSION,
            "slug": skill.slug,
            "name": skill.name,
            "description": skill.description,
            "version": skill.version,
            "source": source.kind,
            "sourceDisplayName": source.display_name,
            "sourceId": source.source_id,
            "coordinate": source.coordinate,
            "namespace": source.namespace,
            "contentHash": source.content_hash,
            "signatureKeyId": source.signature_key_id,
            "signatureVerified": source.signature_verified,
            "target": target,
            "installedAt": now_unix_ms(),
            "packageSha256": package_hash,
        }),
    )
}

fn normalize_targets(targets: Vec<SkillTarget>) -> Result<Vec<SkillTarget>> {
    let mut normalized = Vec::new();
    for target in targets {
        if !normalized.contains(&target) {
            normalized.push(target);
        }
    }
    if normalized.is_empty() {
        bail!("至少选择一个Skill安装目标");
    }
    Ok(normalized)
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty()
        || slug == "."
        || slug == ".."
        || slug.contains(['/', '\\', ':'])
        || slug.ends_with(['.', ' '])
    {
        bail!("不安全的Skill slug：{slug}");
    }
    Ok(())
}

fn validate_skill_selector(selector: &str) -> Result<()> {
    if selector.starts_with("skillhub:") {
        super::skillhub::parse_reference(selector)?;
        return Ok(());
    }
    validate_slug(selector)
}

fn relative_to_root(paths: &WorkstationPaths, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(&paths.root)
        .with_context(|| format!("路径不在工作站根目录内：{}", path.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn resolve_relative_root(paths: &WorkstationPaths, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() {
        bail!("便携元数据中禁止绝对路径：{relative}");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => bail!("便携元数据路径越界：{relative}"),
        }
    }
    Ok(paths.root.join(path_from_slash(relative)))
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(path).with_context(|| format!("无法删除符号链接 {}", path.display()))
        }
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(path).with_context(|| format!("无法删除目录 {}", path.display()))
        }
        Ok(_) => fs::remove_file(path).with_context(|| format!("无法删除文件 {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法检查 {}", path.display())),
    }
}

struct SkillsLock {
    path: PathBuf,
}

impl SkillsLock {
    fn acquire(paths: &WorkstationPaths) -> Result<Self> {
        let path = paths.skills_data.join("install.lock");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .with_context(
                || "另一个Skills安装/卸载事务正在运行；如确认没有进程，请执行skills repair",
            )?;
        fs::write(&path, format!("{}\n", std::process::id()))?;
        Ok(Self { path })
    }
}

impl Drop for SkillsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn prepared_paths() -> WorkstationPaths {
        let temporary = tempdir().unwrap().keep();
        let paths = WorkstationPaths::from_root(temporary.join("中文 dev2"));
        paths.prepare_layout().unwrap();
        paths
    }

    #[tokio::test]
    async fn one_skill_installs_to_three_targets_and_uninstalls() {
        let paths = prepared_paths();
        let source = paths.root.join("测试技能.md");
        fs::write(
            &source,
            "---\nname: portable-test\nversion: 1.0.0\n---\n测试\n",
        )
        .unwrap();
        let plan = create_plan(
            &paths,
            source.to_str().unwrap(),
            SkillTarget::ALL.to_vec(),
            false,
        )
        .await
        .unwrap();
        let installed = install_plan(&paths, &plan.plan_id).unwrap();
        assert_eq!(installed.records.len(), 3);
        assert!(
            list_status(&paths)
                .unwrap()
                .iter()
                .all(|item| item.verified)
        );

        let removed = uninstall(&paths, "portable-test", SkillTarget::ALL.to_vec()).unwrap();
        assert_eq!(removed.removed.len(), 3);
        assert!(read_registry(&paths).unwrap().records.is_empty());
    }

    #[tokio::test]
    async fn second_target_failure_rolls_back_first_target() {
        let paths = prepared_paths();
        let source = paths.root.join("rollback.md");
        fs::write(
            &source,
            "---\nname: rollback-test\nversion: 1.0.0\n---\n测试\n",
        )
        .unwrap();
        let plan = create_plan(
            &paths,
            source.to_str().unwrap(),
            vec![SkillTarget::Codex, SkillTarget::Claude],
            false,
        )
        .await
        .unwrap();
        let claude_skills = paths.skill_target_dir(SkillTarget::Claude);
        fs::remove_dir_all(&claude_skills).unwrap();
        fs::write(&claude_skills, b"block target directory").unwrap();

        assert!(install_plan(&paths, &plan.plan_id).is_err());
        assert!(
            !paths
                .skill_target_dir(SkillTarget::Codex)
                .join("rollback-test")
                .exists()
        );
        assert!(read_registry(&paths).unwrap().records.is_empty());
    }

    #[test]
    fn relative_metadata_rejects_parent_escape() {
        let paths = prepared_paths();
        assert!(resolve_relative_root(&paths, "../outside").is_err());
    }
}
