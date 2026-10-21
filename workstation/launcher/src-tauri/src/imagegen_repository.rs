use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct NewImageJob {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub mode: String,
    pub prompt: String,
    pub negative_prompt: String,
    pub size: String,
    pub quality: String,
    pub requested_count: u8,
    pub reference_relative_path: Option<String>,
    pub reference_mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAssetRecord {
    pub id: String,
    pub job_id: String,
    pub relative_path: String,
    pub thumbnail_relative_path: String,
    pub mime_type: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub revised_prompt: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageJob {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub mode: String,
    pub prompt: String,
    pub negative_prompt: String,
    pub size: String,
    pub quality: String,
    pub requested_count: u8,
    pub status: String,
    pub error_message: Option<String>,
    pub reference_relative_path: Option<String>,
    pub reference_mime_type: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub assets: Vec<ImageAssetRecord>,
}

#[derive(Debug, Clone)]
pub struct ImageRepository {
    path: PathBuf,
}

impl ImageRepository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn initialize(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建 AI 生图数据目录 {}", parent.display()))?;
        }
        let connection = self.connection()?;
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS imagegen_jobs (
                id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                provider_name TEXT NOT NULL,
                model TEXT NOT NULL,
                mode TEXT NOT NULL,
                prompt TEXT NOT NULL,
                negative_prompt TEXT NOT NULL DEFAULT '',
                size TEXT NOT NULL,
                quality TEXT NOT NULL,
                requested_count INTEGER NOT NULL,
                status TEXT NOT NULL,
                error_message TEXT,
                reference_relative_path TEXT,
                reference_mime_type TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS imagegen_assets (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                thumbnail_relative_path TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                revised_prompt TEXT,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(job_id) REFERENCES imagegen_jobs(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_imagegen_jobs_updated
            ON imagegen_jobs(updated_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_imagegen_assets_job
            ON imagegen_assets(job_id, created_at_ms ASC);
            "#,
        )?;
        migrate_legacy_history(&connection)?;
        Ok(())
    }

    pub fn recover_interrupted_jobs(&self) -> Result<usize> {
        self.initialize()?;
        Ok(self.connection()?.execute(
            r#"UPDATE imagegen_jobs
               SET status = 'interrupted',
                   error_message = '工作站上次退出时任务尚未完成。',
                   updated_at_ms = ?1
               WHERE status IN ('queued', 'running')"#,
            [Utc::now().timestamp_millis()],
        )?)
    }

    pub fn create_job(&self, job: &NewImageJob) -> Result<ImageJob> {
        self.initialize()?;
        let now = Utc::now().timestamp_millis();
        self.connection()?.execute(
            r#"INSERT INTO imagegen_jobs(
                id, provider_id, provider_name, model, mode, prompt, negative_prompt,
                size, quality, requested_count, status, reference_relative_path,
                reference_mime_type, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                      'queued', ?11, ?12, ?13, ?13)"#,
            params![
                job.id,
                job.provider_id,
                job.provider_name,
                job.model,
                job.mode,
                job.prompt,
                job.negative_prompt,
                job.size,
                job.quality,
                i64::from(job.requested_count),
                job.reference_relative_path,
                job.reference_mime_type,
                now,
            ],
        )?;
        self.get_job(&job.id)?
            .ok_or_else(|| anyhow::anyhow!("创建 AI 生图任务后无法读取任务"))
    }

    pub fn set_status(&self, job_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.initialize()?;
        let changed = self.connection()?.execute(
            "UPDATE imagegen_jobs SET status = ?2, error_message = ?3, updated_at_ms = ?4 WHERE id = ?1",
            params![job_id, status, error, Utc::now().timestamp_millis()],
        )?;
        if changed == 0 {
            bail!("未找到 AI 生图任务 {job_id}");
        }
        Ok(())
    }

    pub fn add_asset(&self, asset: &ImageAssetRecord) -> Result<()> {
        self.initialize()?;
        self.connection()?.execute(
            r#"INSERT INTO imagegen_assets(
                id, job_id, relative_path, thumbnail_relative_path, mime_type,
                file_size, width, height, revised_prompt, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                asset.id,
                asset.job_id,
                asset.relative_path,
                asset.thumbnail_relative_path,
                asset.mime_type,
                i64::try_from(asset.file_size).unwrap_or(i64::MAX),
                i64::from(asset.width),
                i64::from(asset.height),
                asset.revised_prompt,
                asset.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_jobs(&self, limit: usize) -> Result<Vec<ImageJob>> {
        self.initialize()?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            r#"SELECT id, provider_id, provider_name, model, mode, prompt,
                      negative_prompt, size, quality, requested_count, status,
                      error_message, reference_relative_path, reference_mime_type,
                      created_at_ms, updated_at_ms
               FROM imagegen_jobs ORDER BY updated_at_ms DESC LIMIT ?1"#,
        )?;
        let rows = statement.query_map(
            [i64::try_from(limit.clamp(1, 200)).unwrap_or(80)],
            job_from_row,
        )?;
        let mut jobs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for job in &mut jobs {
            job.assets = assets_for_job(&connection, &job.id)?;
        }
        Ok(jobs)
    }

    pub fn get_job(&self, job_id: &str) -> Result<Option<ImageJob>> {
        self.initialize()?;
        let connection = self.connection()?;
        let mut job = connection
            .query_row(
                r#"SELECT id, provider_id, provider_name, model, mode, prompt,
                          negative_prompt, size, quality, requested_count, status,
                          error_message, reference_relative_path, reference_mime_type,
                          created_at_ms, updated_at_ms
                   FROM imagegen_jobs WHERE id = ?1"#,
                [job_id],
                job_from_row,
            )
            .optional()?;
        if let Some(job) = &mut job {
            job.assets = assets_for_job(&connection, &job.id)?;
        }
        Ok(job)
    }

    pub fn get_asset(&self, asset_id: &str) -> Result<Option<ImageAssetRecord>> {
        self.initialize()?;
        self.connection()?
            .query_row(
                r#"SELECT id, job_id, relative_path, thumbnail_relative_path, mime_type,
                          file_size, width, height, revised_prompt, created_at_ms
                   FROM imagegen_assets WHERE id = ?1"#,
                [asset_id],
                asset_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn delete_job(&self, job_id: &str) -> Result<Option<ImageJob>> {
        self.initialize()?;
        let Some(job) = self.get_job(job_id)? else {
            return Ok(None);
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM imagegen_assets WHERE job_id = ?1", [job_id])?;
        transaction.execute("DELETE FROM imagegen_jobs WHERE id = ?1", [job_id])?;
        transaction.commit()?;
        Ok(Some(job))
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("无法打开 AI 生图数据库 {}", self.path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(connection)
    }
}

fn migrate_legacy_history(connection: &Connection) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='image_jobs')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO imagegen_jobs(
            id, provider_id, provider_name, model, mode, prompt, negative_prompt,
            size, quality, requested_count, status, error_message,
            reference_relative_path, reference_mime_type, created_at_ms, updated_at_ms
        )
        SELECT id, 'legacy-' || engine, engine || '旧版供应商', model,
               'generation', prompt, '', size, quality, 1, 'completed', NULL,
               NULL, NULL, created_at, created_at
        FROM image_jobs;

        INSERT OR IGNORE INTO imagegen_assets(
            id, job_id, relative_path, thumbnail_relative_path, mime_type,
            file_size, width, height, revised_prompt, created_at_ms
        )
        SELECT 'legacy-' || id, id, 'assets/' || file_name, 'assets/' || file_name,
               media_type, 0, 0, 0, NULL, created_at
        FROM image_jobs;
        "#,
    )?;
    Ok(())
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImageJob> {
    Ok(ImageJob {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_name: row.get(2)?,
        model: row.get(3)?,
        mode: row.get(4)?,
        prompt: row.get(5)?,
        negative_prompt: row.get(6)?,
        size: row.get(7)?,
        quality: row.get(8)?,
        requested_count: row.get(9)?,
        status: row.get(10)?,
        error_message: row.get(11)?,
        reference_relative_path: row.get(12)?,
        reference_mime_type: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        assets: Vec::new(),
    })
}

fn asset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImageAssetRecord> {
    Ok(ImageAssetRecord {
        id: row.get(0)?,
        job_id: row.get(1)?,
        relative_path: row.get(2)?,
        thumbnail_relative_path: row.get(3)?,
        mime_type: row.get(4)?,
        file_size: row.get(5)?,
        width: row.get(6)?,
        height: row.get(7)?,
        revised_prompt: row.get(8)?,
        created_at_ms: row.get(9)?,
    })
}

fn assets_for_job(connection: &Connection, job_id: &str) -> Result<Vec<ImageAssetRecord>> {
    let mut statement = connection.prepare(
        r#"SELECT id, job_id, relative_path, thumbnail_relative_path, mime_type,
                  file_size, width, height, revised_prompt, created_at_ms
           FROM imagegen_assets WHERE job_id = ?1 ORDER BY created_at_ms ASC, rowid ASC"#,
    )?;
    let rows = statement.query_map([job_id], asset_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_records_without_deleting_old_table() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("history.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"CREATE TABLE image_jobs(
                    id TEXT PRIMARY KEY, engine TEXT, model TEXT, prompt TEXT, size TEXT,
                    quality TEXT, file_name TEXT, media_type TEXT, created_at INTEGER
                );
                INSERT INTO image_jobs VALUES(
                    'old-job', 'codex', 'old-image', '旧图片', '1024x1024',
                    'standard', 'old.png', 'image/png', 1
                );"#,
            )
            .unwrap();
        drop(connection);
        let repository = ImageRepository::new(path.clone());
        repository.initialize().unwrap();
        let job = repository.get_job("old-job").unwrap().unwrap();
        assert_eq!(job.assets.len(), 1);
        let connection = Connection::open(path).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM image_jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn recovers_interrupted_jobs() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = ImageRepository::new(temporary.path().join("history.sqlite"));
        let created = repository
            .create_job(&NewImageJob {
                id: "job-1".to_string(),
                provider_id: "provider".to_string(),
                provider_name: "图片供应商".to_string(),
                model: "image-model".to_string(),
                mode: "generation".to_string(),
                prompt: "一座未来城市".to_string(),
                negative_prompt: String::new(),
                size: "1024x1024".to_string(),
                quality: "auto".to_string(),
                requested_count: 1,
                reference_relative_path: None,
                reference_mime_type: None,
            })
            .unwrap();
        repository.set_status(&created.id, "running", None).unwrap();
        repository.recover_interrupted_jobs().unwrap();
        assert_eq!(
            repository.get_job(&created.id).unwrap().unwrap().status,
            "interrupted"
        );
    }
}
