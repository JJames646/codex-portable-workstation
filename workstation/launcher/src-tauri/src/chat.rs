use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
use yunshu_workstation_runtime::model::Engine;
use yunshu_workstation_runtime::paths::WorkstationPaths;

use crate::chat_provider::{
    ChatRequestAttachment, ChatRequestMessage, load_active_provider, load_provider,
    request_provider_text,
};
use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};

const MAX_ATTACHMENTS_PER_MESSAGE: usize = 5;
const MAX_TEXT_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMAGE_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
static ACTIVE_CHAT_REQUESTS: AtomicUsize = AtomicUsize::new(0);

struct ActiveChatRequestGuard;

impl ActiveChatRequestGuard {
    fn enter() -> Self {
        ACTIVE_CHAT_REQUESTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ActiveChatRequestGuard {
    fn drop(&mut self) {
        ACTIVE_CHAT_REQUESTS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn active_request_count() -> usize {
    ACTIVE_CHAT_REQUESTS.load(Ordering::SeqCst)
}

const CHAT_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS chat_threads (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    engine TEXT NOT NULL,
    provider_id TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_attachments (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    message_id TEXT REFERENCES chat_messages(id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    stored_name TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_threads_engine_updated
ON chat_threads(engine, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_chat_messages_thread_created
ON chat_messages(thread_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_chat_attachments_thread_message
ON chat_attachments(thread_id, message_id, created_at ASC);
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThreadSummary {
    pub id: String,
    pub title: String,
    pub engine: Engine,
    pub provider_id: String,
    pub model: String,
    pub preview: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub attachments: Vec<ChatAttachment>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub id: String,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThreadDetail {
    pub thread: ChatThreadSummary,
    pub messages: Vec<ChatMessage>,
    pub pending_attachments: Vec<ChatAttachment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEngineInput {
    pub engine: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThreadInput {
    pub thread_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThreadRenameInput {
    pub thread_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendInput {
    pub thread_id: String,
    pub content: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachmentImportInput {
    pub thread_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachmentReferenceInput {
    pub attachment_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatThreadProviderInput {
    pub thread_id: String,
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendResult {
    pub thread: ChatThreadSummary,
    pub user_message: ChatMessage,
    pub assistant_message: ChatMessage,
}

#[tauri::command]
pub fn chat_initialize(state: State<'_, AppState>) -> CommandResult<()> {
    open_database(&state.paths)
        .map(|_| ())
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_list_threads(
    state: State<'_, AppState>,
    input: ChatEngineInput,
) -> CommandResult<Vec<ChatThreadSummary>> {
    list_threads(&state.paths, parse_engine(&input.engine)?).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_create_thread(
    state: State<'_, AppState>,
    input: ChatEngineInput,
) -> CommandResult<ChatThreadDetail> {
    create_thread(&state.paths, parse_engine(&input.engine)?).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_load_thread(
    state: State<'_, AppState>,
    input: ChatThreadInput,
) -> CommandResult<ChatThreadDetail> {
    load_thread(&state.paths, &input.thread_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn chat_send(
    state: State<'_, AppState>,
    input: ChatSendInput,
) -> CommandResult<ChatSendResult> {
    let _active_request = ActiveChatRequestGuard::enter();
    send(
        &state.paths,
        &input.thread_id,
        &input.content,
        &input.attachment_ids,
    )
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_import_attachments(
    state: State<'_, AppState>,
    input: ChatAttachmentImportInput,
) -> CommandResult<Vec<ChatAttachment>> {
    import_attachments(&state.paths, &input.thread_id, &input.paths).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_remove_attachment(
    state: State<'_, AppState>,
    input: ChatAttachmentReferenceInput,
) -> CommandResult<()> {
    remove_attachment(&state.paths, &input.attachment_id).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_set_thread_provider(
    state: State<'_, AppState>,
    input: ChatThreadProviderInput,
) -> CommandResult<ChatThreadDetail> {
    set_thread_provider(
        &state.paths,
        &input.thread_id,
        &input.provider_id,
        &input.model,
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_delete_thread(state: State<'_, AppState>, input: ChatThreadInput) -> CommandResult<()> {
    delete_thread(&state.paths, &input.thread_id).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_rename_thread(
    state: State<'_, AppState>,
    input: ChatThreadRenameInput,
) -> CommandResult<ChatThreadSummary> {
    rename_thread(&state.paths, &input.thread_id, &input.title).map_err(CommandError::from)
}

#[tauri::command]
pub fn chat_export_thread(
    state: State<'_, AppState>,
    input: ChatThreadInput,
) -> CommandResult<String> {
    export_thread(&state.paths, &input.thread_id)
        .map(|path| path.display().to_string())
        .map_err(CommandError::from)
}

fn open_database(paths: &WorkstationPaths) -> Result<Connection> {
    let directory = paths.data.join("chat");
    fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建对话数据目录 {}", directory.display()))?;
    let database = directory.join("chat.sqlite");
    let connection = Connection::open(&database)
        .with_context(|| format!("无法打开对话数据库 {}", database.display()))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .context("无法设置对话数据库等待时间")?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
        .context("无法初始化对话数据库日志模式")?;
    connection
        .execute_batch(CHAT_SCHEMA)
        .context("无法初始化对话数据库结构")?;
    ensure_thread_provider_column(&connection)?;
    Ok(connection)
}

pub(crate) fn initialize_storage(paths: &WorkstationPaths) -> Result<()> {
    open_database(paths).map(drop)
}

fn ensure_thread_provider_column(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(chat_threads)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "provider_id") {
        connection.execute(
            "ALTER TABLE chat_threads ADD COLUMN provider_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

fn parse_engine(value: &str) -> CommandResult<Engine> {
    Engine::from_str(value).map_err(CommandError::from)
}

fn list_threads(paths: &WorkstationPaths, engine: Engine) -> Result<Vec<ChatThreadSummary>> {
    let connection = open_database(paths)?;
    let mut statement = connection.prepare(
        r#"
        SELECT t.id, t.title, t.engine, t.provider_id, t.model,
               COALESCE((
                   SELECT m.content
                   FROM chat_messages m
                   WHERE m.thread_id = t.id
                   ORDER BY m.created_at DESC, m.rowid DESC
                   LIMIT 1
               ), '尚未发送消息') AS preview,
               t.created_at, t.updated_at
        FROM chat_threads t
        WHERE t.engine = ?1
        ORDER BY t.updated_at DESC
        "#,
    )?;
    let rows = statement.query_map([engine.slug()], map_thread_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("无法读取本地对话列表")
}

fn create_thread(paths: &WorkstationPaths, engine: Engine) -> Result<ChatThreadDetail> {
    let config = load_active_provider(paths)?;
    let now = Utc::now().timestamp_millis();
    let id = Uuid::new_v4().to_string();
    let connection = open_database(paths)?;
    connection.execute(
        r#"
        INSERT INTO chat_threads(id, title, engine, provider_id, model, created_at, updated_at)
        VALUES (?1, '新的对话', ?2, ?3, ?4, ?5, ?5)
        "#,
        params![id, engine.slug(), config.id, config.default_model, now],
    )?;
    load_thread(paths, &id)
}

fn load_thread(paths: &WorkstationPaths, thread_id: &str) -> Result<ChatThreadDetail> {
    let connection = open_database(paths)?;
    let thread = load_thread_summary(&connection, thread_id)?;
    let mut statement = connection.prepare(
        r#"
        SELECT id, thread_id, role, content, created_at
        FROM chat_messages
        WHERE thread_id = ?1
        ORDER BY created_at ASC, rowid ASC
        "#,
    )?;
    let mut messages = statement
        .query_map([thread_id], map_message_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("无法读取本地对话消息")?;
    for message in &mut messages {
        message.attachments = list_attachments_for_message(&connection, &message.id)?;
    }
    let pending_attachments = list_pending_attachments(&connection, thread_id)?;
    Ok(ChatThreadDetail {
        thread,
        messages,
        pending_attachments,
    })
}

async fn send(
    paths: &WorkstationPaths,
    thread_id: &str,
    content: &str,
    attachment_ids: &[String],
) -> Result<ChatSendResult> {
    let content = content.trim();
    if content.is_empty() && attachment_ids.is_empty() {
        bail!("消息内容和附件不能同时为空");
    }
    if content.chars().count() > 100_000 {
        bail!("单条消息不能超过100000个字符");
    }
    if attachment_ids.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        bail!("单条消息最多只能携带5个附件");
    }

    let (thread, user_message, request_messages) = {
        let mut connection = open_database(paths)?;
        let transaction = connection.transaction()?;
        let mut thread = load_thread_summary(&transaction, thread_id)?;
        if thread.provider_id.trim().is_empty() {
            let provider = load_active_provider(paths)?;
            transaction.execute(
                "UPDATE chat_threads SET provider_id = ?1, model = ?2 WHERE id = ?3",
                params![&provider.id, &provider.default_model, thread_id],
            )?;
            thread.provider_id = provider.id;
            thread.model = provider.default_model;
        }
        let now = Utc::now().timestamp_millis();
        let mut user_message = ChatMessage {
            id: Uuid::new_v4().to_string(),
            thread_id: thread_id.to_string(),
            role: "user".to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            created_at: now,
        };
        transaction.execute(
            r#"
            INSERT INTO chat_messages(id, thread_id, role, content, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                user_message.id,
                user_message.thread_id,
                user_message.role,
                user_message.content,
                user_message.created_at
            ],
        )?;
        bind_attachments(
            &transaction,
            paths,
            thread_id,
            &user_message.id,
            attachment_ids,
        )?;
        user_message.attachments = list_attachments_for_message(&transaction, &user_message.id)?;
        let title = if thread.title == "新的对话" {
            if content.is_empty() {
                user_message
                    .attachments
                    .first()
                    .map(|attachment| attachment.file_name.clone())
                    .unwrap_or_else(|| "附件对话".to_string())
            } else {
                title_from_content(content)
            }
        } else {
            thread.title.clone()
        };
        transaction.execute(
            "UPDATE chat_threads SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, thread_id],
        )?;
        let request_messages = read_request_messages(&transaction, paths, thread_id)?;
        transaction.commit()?;
        (thread, user_message, request_messages)
    };

    let config = load_provider(paths, &thread.provider_id)?;
    let mut request_config = config.clone();
    if !thread.model.trim().is_empty() {
        request_config.default_model = thread.model.clone();
    }
    let assistant_content =
        request_provider_text(paths, &request_config, &request_messages).await?;
    let assistant_message = ChatMessage {
        id: Uuid::new_v4().to_string(),
        thread_id: thread_id.to_string(),
        role: "assistant".to_string(),
        content: assistant_content,
        attachments: Vec::new(),
        created_at: Utc::now().timestamp_millis(),
    };
    {
        let connection = open_database(paths)?;
        connection.execute(
            r#"
            INSERT INTO chat_messages(id, thread_id, role, content, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                assistant_message.id,
                assistant_message.thread_id,
                assistant_message.role,
                assistant_message.content,
                assistant_message.created_at
            ],
        )?;
        connection.execute(
            "UPDATE chat_threads SET model = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                request_config.default_model,
                assistant_message.created_at,
                thread_id
            ],
        )?;
    }
    Ok(ChatSendResult {
        thread: load_thread(paths, thread_id)?.thread,
        user_message,
        assistant_message,
    })
}

fn read_request_messages(
    connection: &Connection,
    paths: &WorkstationPaths,
    thread_id: &str,
) -> Result<Vec<ChatRequestMessage>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, role, content
        FROM chat_messages
        WHERE thread_id = ?1
        ORDER BY created_at ASC, rowid ASC
        LIMIT 200
        "#,
    )?;
    let rows = statement
        .query_map([thread_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("无法整理模型上下文")?;
    rows.into_iter()
        .map(|(message_id, role, content)| {
            Ok(ChatRequestMessage {
                role,
                content,
                attachments: list_request_attachments(connection, paths, &message_id)?,
            })
        })
        .collect()
}

fn load_thread_summary(connection: &Connection, thread_id: &str) -> Result<ChatThreadSummary> {
    connection
        .query_row(
            r#"
            SELECT t.id, t.title, t.engine, t.provider_id, t.model,
                   COALESCE((
                       SELECT m.content FROM chat_messages m
                       WHERE m.thread_id = t.id
                       ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1
                   ), '尚未发送消息'),
                   t.created_at, t.updated_at
            FROM chat_threads t WHERE t.id = ?1
            "#,
            [thread_id],
            map_thread_row,
        )
        .optional()?
        .ok_or_else(|| anyhow!("本地对话不存在"))
}

fn map_thread_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadSummary> {
    let engine: String = row.get(2)?;
    let engine = Engine::from_str(&engine).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(ChatThreadSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        engine,
        provider_id: row.get(3)?,
        model: row.get(4)?,
        preview: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    Ok(ChatMessage {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        attachments: Vec::new(),
        created_at: row.get(4)?,
    })
}

fn delete_thread(paths: &WorkstationPaths, thread_id: &str) -> Result<()> {
    let mut connection = open_database(paths)?;
    let transaction = connection.transaction()?;
    let stored_names = list_stored_attachment_names(&transaction, thread_id)?;
    let affected = transaction.execute("DELETE FROM chat_threads WHERE id = ?1", [thread_id])?;
    if affected == 0 {
        bail!("本地对话不存在");
    }
    transaction.commit()?;
    for stored_name in stored_names {
        remove_stored_attachment(paths, &stored_name);
    }
    Ok(())
}

fn rename_thread(
    paths: &WorkstationPaths,
    thread_id: &str,
    title: &str,
) -> Result<ChatThreadSummary> {
    let title = title.trim();
    if title.is_empty() {
        bail!("对话名称不能为空");
    }
    if title.chars().count() > 200 {
        bail!("对话名称不能超过200个字符");
    }
    let connection = open_database(paths)?;
    let affected = connection.execute(
        "UPDATE chat_threads SET title = ?1 WHERE id = ?2",
        params![title, thread_id],
    )?;
    if affected == 0 {
        bail!("本地对话不存在");
    }
    load_thread_summary(&connection, thread_id)
}

fn export_thread(paths: &WorkstationPaths, thread_id: &str) -> Result<PathBuf> {
    let detail = load_thread(paths, thread_id)?;
    let directory = paths.workspace.join("ChatExports");
    fs::create_dir_all(&directory)?;
    let file_name = format!(
        "{}-{}.md",
        safe_file_component(&detail.thread.title),
        &detail.thread.id[..8.min(detail.thread.id.len())]
    );
    let destination = directory.join(file_name);
    let mut markdown = format!(
        "# {}\n\n- 智能体：{}\n- 模型：{}\n\n",
        detail.thread.title, detail.thread.engine, detail.thread.model
    );
    for message in detail.messages {
        let role = if message.role == "user" {
            "用户"
        } else {
            "AI"
        };
        markdown.push_str(&format!("## {role}\n\n"));
        if !message.attachments.is_empty() {
            markdown.push_str("附件：\n");
            for attachment in &message.attachments {
                markdown.push_str(&format!("- {}\n", attachment.file_name));
            }
            markdown.push('\n');
        }
        if !message.content.is_empty() {
            markdown.push_str(&message.content);
            markdown.push_str("\n\n");
        }
    }
    fs::write(&destination, markdown)
        .with_context(|| format!("无法导出对话 {}", destination.display()))?;
    Ok(destination)
}

fn title_from_content(content: &str) -> String {
    let title = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(24)
        .collect::<String>();
    if title.is_empty() {
        "新的对话".to_string()
    } else {
        title
    }
}

fn safe_file_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .take(48)
        .collect::<String>();
    if safe.trim().is_empty() {
        "对话记录".to_string()
    } else {
        safe
    }
}

fn attachment_root(paths: &WorkstationPaths) -> PathBuf {
    paths.data.join("chat").join("attachments")
}

fn import_attachments(
    paths: &WorkstationPaths,
    thread_id: &str,
    source_paths: &[String],
) -> Result<Vec<ChatAttachment>> {
    if source_paths.is_empty() {
        bail!("请选择要上传的文件");
    }
    if source_paths.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        bail!("每次最多选择5个附件");
    }

    let mut connection = open_database(paths)?;
    load_thread_summary(&connection, thread_id)?;
    let pending_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM chat_attachments WHERE thread_id = ?1 AND message_id IS NULL",
        [thread_id],
        |row| row.get(0),
    )?;
    if pending_count as usize + source_paths.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        bail!("单条消息最多只能携带5个附件");
    }

    let root = attachment_root(paths);
    fs::create_dir_all(&root).with_context(|| format!("无法创建附件目录 {}", root.display()))?;
    let transaction = connection.transaction()?;
    let mut imported = Vec::with_capacity(source_paths.len());
    let mut copied_files = Vec::with_capacity(source_paths.len());
    let import_result = (|| -> Result<()> {
        for source_value in source_paths {
            let source = PathBuf::from(source_value);
            if !source.is_file() {
                bail!("附件不存在或不是普通文件：{}", source.display());
            }
            let (media_type, limit, extension) = supported_attachment(&source)?;
            let metadata = fs::metadata(&source)
                .with_context(|| format!("无法读取附件信息 {}", source.display()))?;
            if metadata.len() == 0 {
                bail!("附件不能是空文件：{}", source.display());
            }
            if metadata.len() > limit {
                bail!("{} 超过允许的大小限制", source.display());
            }
            let file_name = source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| anyhow!("附件名称无法识别：{}", source.display()))?
                .to_string();
            let id = Uuid::new_v4().to_string();
            let stored_name = format!("{id}.{extension}");
            let destination = root.join(&stored_name);
            fs::copy(&source, &destination)
                .with_context(|| format!("无法复制附件 {}", source.display()))?;
            copied_files.push(destination);
            let created_at = Utc::now().timestamp_millis();
            transaction.execute(
                r#"
                INSERT INTO chat_attachments(
                    id, thread_id, message_id, file_name, media_type, stored_name, size_bytes, created_at
                ) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    id,
                    thread_id,
                    file_name,
                    media_type,
                    stored_name,
                    metadata.len() as i64,
                    created_at
                ],
            )?;
            imported.push(ChatAttachment {
                id,
                thread_id: thread_id.to_string(),
                message_id: None,
                file_name,
                media_type: media_type.to_string(),
                size_bytes: metadata.len(),
                created_at,
            });
        }
        Ok(())
    })();
    if let Err(error) = import_result {
        cleanup_copied_attachments(&copied_files);
        return Err(error);
    }
    if let Err(error) = transaction.commit() {
        cleanup_copied_attachments(&copied_files);
        return Err(error).context("无法保存附件记录");
    }
    Ok(imported)
}

fn supported_attachment(source: &Path) -> Result<(&'static str, u64, &'static str)> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow!("附件缺少可识别的扩展名"))?;
    match extension.as_str() {
        "png" => Ok(("image/png", MAX_IMAGE_ATTACHMENT_BYTES, "png")),
        "jpg" | "jpeg" => Ok(("image/jpeg", MAX_IMAGE_ATTACHMENT_BYTES, "jpg")),
        "webp" => Ok(("image/webp", MAX_IMAGE_ATTACHMENT_BYTES, "webp")),
        "txt" => Ok(("text/plain", MAX_TEXT_ATTACHMENT_BYTES, "txt")),
        "md" => Ok(("text/markdown", MAX_TEXT_ATTACHMENT_BYTES, "md")),
        "csv" => Ok(("text/csv", MAX_TEXT_ATTACHMENT_BYTES, "csv")),
        "json" => Ok(("application/json", MAX_TEXT_ATTACHMENT_BYTES, "json")),
        _ => bail!("暂仅支持 PNG、JPG、WEBP、TXT、MD、CSV 和 JSON 附件"),
    }
}

fn cleanup_copied_attachments(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn bind_attachments(
    connection: &Connection,
    paths: &WorkstationPaths,
    thread_id: &str,
    message_id: &str,
    attachment_ids: &[String],
) -> Result<()> {
    for attachment_id in attachment_ids {
        let stored_name = connection
            .query_row(
                r#"
                SELECT stored_name FROM chat_attachments
                WHERE id = ?1 AND thread_id = ?2 AND message_id IS NULL
                "#,
                params![attachment_id, thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("附件不存在、已发送或不属于当前对话"))?;
        if !stored_attachment_path(paths, &stored_name)?.is_file() {
            bail!("附件实体文件不存在");
        }
        connection.execute(
            "UPDATE chat_attachments SET message_id = ?1 WHERE id = ?2",
            params![message_id, attachment_id],
        )?;
    }
    Ok(())
}

fn remove_attachment(paths: &WorkstationPaths, attachment_id: &str) -> Result<()> {
    let mut connection = open_database(paths)?;
    let transaction = connection.transaction()?;
    let stored_name = transaction
        .query_row(
            "SELECT stored_name FROM chat_attachments WHERE id = ?1 AND message_id IS NULL",
            [attachment_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("附件不存在或已经发送"))?;
    transaction.execute(
        "DELETE FROM chat_attachments WHERE id = ?1",
        [attachment_id],
    )?;
    transaction.commit()?;
    remove_stored_attachment(paths, &stored_name);
    Ok(())
}

fn set_thread_provider(
    paths: &WorkstationPaths,
    thread_id: &str,
    provider_id: &str,
    model: &str,
) -> Result<ChatThreadDetail> {
    let provider = load_provider(paths, provider_id)?;
    let model = model.trim();
    if model.is_empty() {
        bail!("请选择或填写对话模型");
    }
    let connection = open_database(paths)?;
    let affected = connection.execute(
        "UPDATE chat_threads SET provider_id = ?1, model = ?2, updated_at = ?3 WHERE id = ?4",
        params![provider.id, model, Utc::now().timestamp_millis(), thread_id],
    )?;
    if affected == 0 {
        bail!("本地对话不存在");
    }
    load_thread(paths, thread_id)
}

fn list_pending_attachments(
    connection: &Connection,
    thread_id: &str,
) -> Result<Vec<ChatAttachment>> {
    list_attachments(
        connection,
        "thread_id = ?1 AND message_id IS NULL",
        thread_id,
    )
}

fn list_attachments_for_message(
    connection: &Connection,
    message_id: &str,
) -> Result<Vec<ChatAttachment>> {
    list_attachments(connection, "message_id = ?1", message_id)
}

fn list_attachments(
    connection: &Connection,
    predicate: &str,
    value: &str,
) -> Result<Vec<ChatAttachment>> {
    let sql = format!(
        r#"
        SELECT id, thread_id, message_id, file_name, media_type, size_bytes, created_at
        FROM chat_attachments WHERE {predicate}
        ORDER BY created_at ASC, rowid ASC
        "#
    );
    let mut statement = connection.prepare(&sql)?;
    statement
        .query_map([value], |row| {
            let size_bytes = row.get::<_, i64>(5)?;
            Ok(ChatAttachment {
                id: row.get(0)?,
                thread_id: row.get(1)?,
                message_id: row.get(2)?,
                file_name: row.get(3)?,
                media_type: row.get(4)?,
                size_bytes: size_bytes.max(0) as u64,
                created_at: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("无法读取对话附件")
}

fn list_request_attachments(
    connection: &Connection,
    paths: &WorkstationPaths,
    message_id: &str,
) -> Result<Vec<ChatRequestAttachment>> {
    let mut statement = connection.prepare(
        r#"
        SELECT file_name, media_type, stored_name
        FROM chat_attachments WHERE message_id = ?1
        ORDER BY created_at ASC, rowid ASC
        "#,
    )?;
    let rows = statement
        .query_map([message_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(file_name, media_type, stored_name)| {
            Ok(ChatRequestAttachment {
                file_name,
                media_type,
                path: stored_attachment_path(paths, &stored_name)?,
            })
        })
        .collect()
}

fn list_stored_attachment_names(connection: &Connection, thread_id: &str) -> Result<Vec<String>> {
    let mut statement =
        connection.prepare("SELECT stored_name FROM chat_attachments WHERE thread_id = ?1")?;
    statement
        .query_map([thread_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("无法读取对话附件清单")
}

fn stored_attachment_path(paths: &WorkstationPaths, stored_name: &str) -> Result<PathBuf> {
    let value = Path::new(stored_name);
    if value.components().count() != 1
        || value.file_name().and_then(|name| name.to_str()) != Some(stored_name)
    {
        bail!("附件存储路径不安全");
    }
    Ok(attachment_root(paths).join(value))
}

fn remove_stored_attachment(paths: &WorkstationPaths, stored_name: &str) {
    if let Ok(path) = stored_attachment_path(paths, stored_name) {
        if path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_history_stays_under_portable_data() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let connection = open_database(&paths).unwrap();
        drop(connection);
        assert!(paths.data.join("chat").join("chat.sqlite").is_file());
    }

    #[test]
    fn old_chat_database_gets_provider_column() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("旧版 数据"));
        let directory = paths.data.join("chat");
        fs::create_dir_all(&directory).unwrap();
        let database = directory.join("chat.sqlite");
        let connection = Connection::open(database).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE chat_threads (
                    id TEXT PRIMARY KEY NOT NULL,
                    title TEXT NOT NULL,
                    engine TEXT NOT NULL,
                    model TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                "#,
            )
            .unwrap();
        drop(connection);
        let connection = open_database(&paths).unwrap();
        let mut statement = connection
            .prepare("PRAGMA table_info(chat_threads)")
            .unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "provider_id"));
    }

    #[test]
    fn imported_attachment_is_copied_under_portable_data() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let config = crate::chat_provider::ChatProviderSaveInput {
            id: None,
            display_name: "测试".to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            default_model: "model-a".to_string(),
            models: vec!["model-a".to_string()],
            protocol: "chat-completions".to_string(),
            timeout_seconds: Some(30),
        };
        crate::chat_provider::save_provider_for_test(&paths, config).unwrap();
        let thread = create_thread(&paths, Engine::Codex).unwrap();
        let source = temporary.path().join("说明.md");
        fs::write(&source, "# 内容").unwrap();
        let attachments =
            import_attachments(&paths, &thread.thread.id, &[source.display().to_string()]).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(fs::read_dir(attachment_root(&paths)).unwrap().count(), 1);
        assert!(attachment_root(&paths).starts_with(&paths.data));
    }

    #[test]
    fn renamed_thread_title_is_trimmed_and_persisted() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let config = crate::chat_provider::ChatProviderSaveInput {
            id: None,
            display_name: "测试".to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            default_model: "model-a".to_string(),
            models: vec!["model-a".to_string()],
            protocol: "chat-completions".to_string(),
            timeout_seconds: Some(30),
        };
        crate::chat_provider::save_provider_for_test(&paths, config).unwrap();
        let thread = create_thread(&paths, Engine::Codex).unwrap();

        let renamed = rename_thread(&paths, &thread.thread.id, "  新名称  ").unwrap();

        assert_eq!(renamed.title, "新名称");
        assert_eq!(
            load_thread(&paths, &thread.thread.id).unwrap().thread.title,
            "新名称"
        );
        assert!(rename_thread(&paths, &thread.thread.id, "   ").is_err());
    }

    #[test]
    fn deleting_thread_removes_database_rows_and_stored_attachments() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let config = crate::chat_provider::ChatProviderSaveInput {
            id: None,
            display_name: "测试".to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            default_model: "model-a".to_string(),
            models: vec!["model-a".to_string()],
            protocol: "chat-completions".to_string(),
            timeout_seconds: Some(30),
        };
        crate::chat_provider::save_provider_for_test(&paths, config).unwrap();
        let thread = create_thread(&paths, Engine::Codex).unwrap();
        let source = temporary.path().join("说明.md");
        fs::write(&source, "# 内容").unwrap();
        import_attachments(&paths, &thread.thread.id, &[source.display().to_string()]).unwrap();

        delete_thread(&paths, &thread.thread.id).unwrap();

        assert!(load_thread(&paths, &thread.thread.id).is_err());
        assert!(list_threads(&paths, Engine::Codex).unwrap().is_empty());
        assert_eq!(fs::read_dir(attachment_root(&paths)).unwrap().count(), 0);
    }

    #[test]
    fn exported_file_name_removes_windows_reserved_characters() {
        assert_eq!(safe_file_component("a:b/c?d*"), "a_b_c_d_");
    }
}
