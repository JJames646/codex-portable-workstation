pub mod catalog;
pub mod dependencies;
pub mod metadata;
pub mod package;
pub mod skillhub;
pub mod skillhub_signature;
pub mod transaction;

use std::fs;

use anyhow::Result;

use crate::paths::WorkstationPaths;

pub fn prepare(paths: &WorkstationPaths) -> Result<()> {
    paths.prepare_layout()?;
    catalog::ensure_default_config(paths)?;
    skillhub::ensure_default_config(paths)?;
    if !paths.skills_registry.is_file() {
        metadata::write_registry(
            paths,
            &metadata::SkillRegistry {
                schema_version: metadata::SKILLS_SCHEMA_VERSION,
                records: Vec::new(),
            },
        )?;
    }
    let lock_path = paths.skills_data.join("install.lock");
    if lock_path.is_file() {
        let process_id = fs::read_to_string(&lock_path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        if process_id.is_some_and(crate::windows_process::process_is_alive) {
            return Ok(());
        }
    }
    transaction::repair(paths)?;
    Ok(())
}
