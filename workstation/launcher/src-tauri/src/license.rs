use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use getrandom::fill;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::vault::PortableVault;
use yunshu_workstation_runtime::windows_process::replace_file_atomic;
use zeroize::Zeroizing;

use crate::commands::AppState;
use crate::error::{CommandError, CommandResult};

const API_BASE_URL: &str = "https://yunshu-license.jackchen6660.dpdns.org";
const ACTIVATION_PATH: &str = "/api/v1/licenses/activate";
const CHECK_IN_PATH: &str = "/api/v1/workstations/check-in";
const ENTITLEMENT_KEY_ID: &str = "yunshu-entitlement-2026-01";
const ENTITLEMENT_PUBLIC_KEY: &str = "SySFL4hAyMLM2jotEJvsQIvhi_GLPyHAR-Eky4P-DNo";
const ENTITLEMENT_PRODUCT: &str = "yunshu-ai-portable-workstation";
pub(crate) const INSTALLATION_PRIVATE_KEY_REF: &str = "license-installation-private-key";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    state: &'static str,
    installation_id: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationInput {
    license_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntitlementEnvelope {
    alg: String,
    key_id: String,
    payload: String,
    signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivationResponse {
    license_type: String,
    installation_id: String,
    activated_at: DateTime<Utc>,
    entitlement: EntitlementEnvelope,
}

#[derive(Debug, Deserialize)]
struct ServerError {
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActivationRequest<'a> {
    license_code: &'a str,
    installation_id: &'a str,
    installation_public_key: &'a str,
    client_version: &'a str,
}

#[derive(Debug, Serialize)]
struct CheckInRequest<'a> {
    installation_id: &'a str,
    startup_id: Uuid,
    client_version: &'a str,
    started_at: DateTime<Utc>,
    last_read_announcement_version: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstationAnnouncement {
    id: Uuid,
    version: String,
    title: String,
    content: String,
    level: String,
    published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedWorkstationAnnouncement {
    announcement: WorkstationAnnouncement,
    is_read: bool,
}

#[derive(Debug, Deserialize)]
struct ServerAnnouncement {
    id: Uuid,
    version: String,
    title: String,
    content: String,
    level: String,
    published_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CheckInResponse {
    #[serde(rename = "server_time")]
    _server_time: DateTime<Utc>,
    #[serde(rename = "startup_recorded")]
    _startup_recorded: bool,
    announcement: Option<ServerAnnouncement>,
    #[serde(rename = "minimum_client_version")]
    _minimum_client_version: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnnouncementState {
    last_read_version: Option<String>,
    read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallationMetadata<'a> {
    schema_version: u32,
    installation_id: &'a str,
    installation_public_key: &'a str,
    created_at: DateTime<Utc>,
}

#[tauri::command]
pub async fn license_status(state: State<'_, AppState>) -> CommandResult<LicenseStatus> {
    let initialization_paths = state.paths.clone();
    if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
        crate::first_use::ensure_initialized(&initialization_paths)
    })
    .await
    .map_err(|error| anyhow!("首次使用初始化任务异常：{error}"))
    .and_then(|result| result)
    {
        return Ok(LicenseStatus {
            state: "invalid",
            installation_id: state.paths.read_installation_id().unwrap_or_default(),
            message: format!("首次使用初始化失败：{error}"),
        });
    }
    Ok(
        local_license_status(&state.paths).unwrap_or_else(|error| LicenseStatus {
            state: "invalid",
            installation_id: state.paths.read_installation_id().unwrap_or_default(),
            message: format!("本地授权文件无效：{error}"),
        }),
    )
}

#[tauri::command]
pub async fn license_activate(
    input: ActivationInput,
    state: State<'_, AppState>,
) -> CommandResult<LicenseStatus> {
    let initialization_paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_initialized_before_activation(&initialization_paths)
    })
    .await
    .map_err(|error| CommandError::from(anyhow!("激活前首次使用初始化任务异常：{error}")))?
    .map_err(CommandError::from)?;
    activate(&state.paths, input)
        .await
        .map_err(CommandError::from)?;
    local_license_status(&state.paths).map_err(CommandError::from)
}

fn ensure_initialized_before_activation(paths: &WorkstationPaths) -> Result<()> {
    crate::first_use::ensure_initialized(paths).context("激活前首次使用初始化检查失败")
}

#[tauri::command]
pub async fn license_check_in(
    state: State<'_, AppState>,
) -> CommandResult<Option<WorkstationAnnouncement>> {
    check_in(&state.paths).await.map_err(CommandError::from)
}

#[tauri::command]
pub async fn license_latest_announcement(
    state: State<'_, AppState>,
) -> CommandResult<Option<CachedWorkstationAnnouncement>> {
    Ok(read_cached_announcement(&state.paths))
}

#[tauri::command]
pub async fn license_mark_announcement_read(
    version: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    if version.len() > 128 || version.trim().is_empty() {
        return Err(CommandError::from_message("公告版本格式无效"));
    }
    let payload = AnnouncementState {
        last_read_version: Some(version),
        read_at: Some(Utc::now()),
    };
    write_json_atomic(&announcement_state_file(&state.paths), &payload).map_err(CommandError::from)
}

fn local_license_status(paths: &WorkstationPaths) -> Result<LicenseStatus> {
    let installation_id = paths.ensure_installation_id()?;
    let entitlement_path = entitlement_file(paths);
    if !entitlement_path.is_file() {
        return Ok(LicenseStatus {
            state: "activation_required",
            installation_id,
            message: "首次使用需要输入云枢工作站卡密。激活成功后可离线启动。".to_string(),
        });
    }
    let signing_key = installation_signing_key(paths, false)?;
    let response: ActivationResponse = serde_json::from_slice(
        &fs::read(&entitlement_path)
            .with_context(|| format!("无法读取授权文件 {}", entitlement_path.display()))?,
    )
    .context("授权文件格式无效")?;
    verify_entitlement(&response, &installation_id, &signing_key)?;
    Ok(LicenseStatus {
        state: "activated",
        installation_id,
        message: "永久授权已在当前移动工作站中生效。".to_string(),
    })
}

async fn activate(paths: &WorkstationPaths, input: ActivationInput) -> Result<()> {
    let license_code = Zeroizing::new(input.license_code.trim().to_string());
    if license_code.len() < 8 || license_code.len() > 256 {
        bail!("卡密格式不正确");
    }
    if license_code.chars().any(char::is_control) {
        bail!("卡密包含无效字符");
    }

    let installation_id = paths.ensure_installation_id()?;
    let signing_key = installation_signing_key(paths, true)?;
    let public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().as_bytes());
    ensure_installation_metadata(paths, &installation_id, &public_key)?;
    let request = ActivationRequest {
        license_code: &license_code,
        installation_id: &installation_id,
        installation_public_key: &public_key,
        client_version: env!("CARGO_PKG_VERSION"),
    };
    let client = http_client(Duration::from_secs(15))?;
    let response = client
        .post(format!("{API_BASE_URL}{ACTIVATION_PATH}"))
        .header("Idempotency-Key", format!("activation-{}", Uuid::new_v4()))
        .json(&request)
        .send()
        .await
        .context("无法连接云枢授权服务器")?;
    if response.status() != StatusCode::OK {
        let status = response.status();
        let error = response.json::<ServerError>().await.ok();
        bail!(
            "{}",
            error
                .and_then(|value| value.message)
                .unwrap_or_else(|| format!("卡密激活失败（HTTP {status}）"))
        );
    }
    let activation = response
        .json::<ActivationResponse>()
        .await
        .context("授权服务器返回了无法识别的数据")?;
    verify_entitlement(&activation, &installation_id, &signing_key)?;
    write_json_atomic(&entitlement_file(paths), &activation)?;
    verify_entitlement(
        &serde_json::from_slice::<ActivationResponse>(&fs::read(entitlement_file(paths))?)?,
        &installation_id,
        &signing_key,
    )?;
    Ok(())
}

async fn check_in(paths: &WorkstationPaths) -> Result<Option<WorkstationAnnouncement>> {
    let status = local_license_status(paths)?;
    if status.state != "activated" {
        bail!("工作站尚未激活");
    }
    let installation_id = status.installation_id;
    let signing_key = installation_signing_key(paths, false)?;
    let announcement_state = read_announcement_state(paths);
    let request = CheckInRequest {
        installation_id: &installation_id,
        startup_id: Uuid::new_v4(),
        client_version: env!("CARGO_PKG_VERSION"),
        started_at: Utc::now(),
        last_read_announcement_version: announcement_state.last_read_version.as_deref(),
    };
    let body = serde_json::to_vec(&request)?;
    let timestamp = Utc::now().timestamp().to_string();
    let nonce = URL_SAFE_NO_PAD.encode(random_bytes::<24>()?);
    let signed = signing_key.sign(&canonical_check_in_message(&timestamp, &nonce, &body));
    let signature = URL_SAFE_NO_PAD.encode(signed.to_bytes());
    let response = http_client(Duration::from_secs(4))?
        .post(format!("{API_BASE_URL}{CHECK_IN_PATH}"))
        .header("Content-Type", "application/json")
        .header("X-Yunshu-Timestamp", timestamp)
        .header("X-Yunshu-Nonce", nonce)
        .header("X-Yunshu-Signature", signature)
        .body(body)
        .send()
        .await
        .context("启动上报暂时无法连接")?;
    if response.status() != StatusCode::OK {
        bail!("启动上报暂时不可用（HTTP {}）", response.status());
    }
    let announcement = response
        .json::<CheckInResponse>()
        .await
        .context("启动上报响应格式无效")?
        .announcement
        .map(|announcement| WorkstationAnnouncement {
            id: announcement.id,
            version: announcement.version,
            title: announcement.title,
            content: announcement.content,
            level: announcement.level,
            published_at: announcement.published_at,
        });
    if let Some(latest) = announcement.as_ref() {
        // 公告缓存失败不应阻塞启动或吞掉本次公告；下次联网时仍可重新获取。
        let _ = write_json_atomic(&latest_announcement_file(paths), latest);
    }
    Ok(announcement)
}

fn installation_signing_key(paths: &WorkstationPaths, create: bool) -> Result<SigningKey> {
    let mut vault = PortableVault::open(paths)?;
    let encoded = match vault.get(INSTALLATION_PRIVATE_KEY_REF) {
        Ok(value) => value,
        Err(_) if create => {
            let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?));
            vault.set(INSTALLATION_PRIVATE_KEY_REF, encoded.clone())?;
            vault.save(paths)?;
            encoded
        }
        Err(_) => bail!("安装实例私钥不存在"),
    };
    let decoded = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .context("安装实例私钥格式无效")?,
    );
    let bytes: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("安装实例私钥长度无效"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn verify_entitlement(
    response: &ActivationResponse,
    installation_id: &str,
    signing_key: &SigningKey,
) -> Result<()> {
    let envelope = &response.entitlement;
    if response.license_type != "permanent"
        || envelope.alg != "Ed25519"
        || envelope.key_id != ENTITLEMENT_KEY_ID
        || response.installation_id != installation_id
    {
        bail!("授权文件与当前移动工作站不匹配");
    }
    let public_key_bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(ENTITLEMENT_PUBLIC_KEY)?
        .try_into()
        .map_err(|_| anyhow!("内置授权公钥长度无效"))?;
    let payload_bytes = URL_SAFE_NO_PAD.decode(&envelope.payload)?;
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(&envelope.signature)?)?;
    VerifyingKey::from_bytes(&public_key_bytes)?
        .verify(&payload_bytes, &signature)
        .context("授权签名验证失败")?;
    let payload: Value = serde_json::from_slice(&payload_bytes)?;
    let public_key_hash = hex_sha256(signing_key.verifying_key().as_bytes());
    if payload.get("installation_id").and_then(Value::as_str) != Some(installation_id)
        || payload
            .get("installation_public_key_hash")
            .and_then(Value::as_str)
            != Some(&public_key_hash)
        || payload.get("license_type").and_then(Value::as_str) != Some("permanent")
        || payload.get("product").and_then(Value::as_str) != Some(ENTITLEMENT_PRODUCT)
    {
        bail!("授权载荷与当前移动工作站不匹配");
    }
    Ok(())
}

fn ensure_installation_metadata(
    paths: &WorkstationPaths,
    installation_id: &str,
    public_key: &str,
) -> Result<()> {
    let path = license_dir(paths).join("installation.json");
    if path.is_file() {
        return Ok(());
    }
    write_json_atomic(
        &path,
        &InstallationMetadata {
            schema_version: 1,
            installation_id,
            installation_public_key: public_key,
            created_at: Utc::now(),
        },
    )
}

fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(timeout)
        .user_agent(concat!(
            "YunshuPortableWorkstation/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("无法初始化安全网络客户端")
}

fn canonical_check_in_message(timestamp: &str, nonce: &str, body: &[u8]) -> Vec<u8> {
    format!(
        "POST\n{CHECK_IN_PATH}\n{timestamp}\n{nonce}\n{}",
        hex_sha256(body)
    )
    .into_bytes()
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    fill(&mut bytes).map_err(|error| anyhow!("无法生成安全随机数：{error}"))?;
    Ok(bytes)
}

fn hex_sha256(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn license_dir(paths: &WorkstationPaths) -> std::path::PathBuf {
    paths.data.join("license")
}

fn entitlement_file(paths: &WorkstationPaths) -> std::path::PathBuf {
    license_dir(paths).join("entitlement.json")
}

fn announcement_state_file(paths: &WorkstationPaths) -> std::path::PathBuf {
    license_dir(paths).join("announcement-state.json")
}

fn latest_announcement_file(paths: &WorkstationPaths) -> std::path::PathBuf {
    license_dir(paths).join("latest-announcement.json")
}

fn read_announcement_state(paths: &WorkstationPaths) -> AnnouncementState {
    fs::read(announcement_state_file(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn read_latest_announcement(paths: &WorkstationPaths) -> Option<WorkstationAnnouncement> {
    fs::read(latest_announcement_file(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn read_cached_announcement(paths: &WorkstationPaths) -> Option<CachedWorkstationAnnouncement> {
    let announcement = read_latest_announcement(paths)?;
    let state = read_announcement_state(paths);
    let is_read = state.last_read_version.as_deref() == Some(announcement.version.as_str());
    Some(CachedWorkstationAnnouncement {
        announcement,
        is_read,
    })
}

fn write_json_atomic(path: &std::path::Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("授权文件路径无效"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file_atomic(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::tempdir;

    #[derive(Deserialize)]
    struct SecureTestCardFile {
        codes: Vec<String>,
    }

    #[test]
    fn installation_key_round_trips_in_existing_portable_vault() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        paths.prepare_layout().unwrap();

        let created = installation_signing_key(&paths, true).unwrap();
        let reopened = installation_signing_key(&paths, false).unwrap();

        assert_eq!(created.verifying_key(), reopened.verifying_key());
        let vault_bytes = fs::read(paths.vault_file()).unwrap();
        assert!(!String::from_utf8_lossy(&vault_bytes).contains(INSTALLATION_PRIVATE_KEY_REF));
    }

    #[test]
    fn check_in_canonical_message_is_stable() {
        assert_eq!(
            String::from_utf8(canonical_check_in_message(
                "1785912000",
                "nonce-1234567890abcdef",
                br#"{"installation_id":"ws-test"}"#,
            ))
            .unwrap(),
            "POST\n/api/v1/workstations/check-in\n1785912000\nnonce-1234567890abcdef\n8b74544a9664ff6ae1c8df4efcc06ad65bd582ede89918a84e034eb763952a51"
        );
    }

    #[test]
    fn missing_entitlement_requires_activation_without_network() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("移动盘"));

        let status = local_license_status(&paths).unwrap();

        assert_eq!(status.state, "activation_required");
        assert_eq!(status.installation_id.len(), 32);
        assert!(!entitlement_file(&paths).exists());
    }

    #[test]
    fn activation_is_rejected_before_initialization_completes() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("移动盘"));

        let error = ensure_initialized_before_activation(&paths).unwrap_err();

        assert!(error.to_string().contains("激活前首次使用初始化检查失败"));
        assert!(paths.read_installation_id().is_none());
        assert!(!entitlement_file(&paths).exists());
    }

    #[test]
    fn latest_announcement_round_trips_in_portable_data() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let expected = WorkstationAnnouncement {
            id: Uuid::new_v4(),
            version: "announcement-version-1".to_string(),
            title: "云枢公告".to_string(),
            content: "公告正文".to_string(),
            level: "important".to_string(),
            published_at: Utc::now(),
        };

        write_json_atomic(&latest_announcement_file(&paths), &expected).unwrap();
        let actual = read_latest_announcement(&paths).unwrap();

        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.version, expected.version);
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.content, expected.content);
        assert_eq!(actual.level, expected.level);
        assert_eq!(actual.published_at, expected.published_at);
    }

    #[test]
    fn cached_announcement_reports_portable_read_state() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let expected = WorkstationAnnouncement {
            id: Uuid::new_v4(),
            version: "announcement-version-2".to_string(),
            title: "离线公告".to_string(),
            content: "优先读取移动盘缓存".to_string(),
            level: "important".to_string(),
            published_at: Utc::now(),
        };
        write_json_atomic(&latest_announcement_file(&paths), &expected).unwrap();

        let unread = read_cached_announcement(&paths).unwrap();
        assert!(!unread.is_read);
        assert_eq!(unread.announcement.version, expected.version);

        write_json_atomic(
            &announcement_state_file(&paths),
            &AnnouncementState {
                last_read_version: Some(expected.version.clone()),
                read_at: Some(Utc::now()),
            },
        )
        .unwrap();

        let read = read_cached_announcement(&paths).unwrap();
        assert!(read.is_read);
        assert_eq!(read.announcement.version, expected.version);
    }

    #[tokio::test]
    #[ignore = "需要显式提供阶段七测试卡文件与已确认的移动盘目录"]
    async fn activate_confirmed_portable_root_from_secure_test_file() {
        let product_root = std::env::var_os("YUNSHU_STAGE7_PRODUCT_ROOT")
            .map(std::path::PathBuf::from)
            .expect("YUNSHU_STAGE7_PRODUCT_ROOT is required");
        let card_file = std::env::var_os("YUNSHU_STAGE7_TEST_CARD_FILE")
            .map(std::path::PathBuf::from)
            .expect("YUNSHU_STAGE7_TEST_CARD_FILE is required");
        let card_file: SecureTestCardFile =
            serde_json::from_slice(&fs::read(card_file).expect("test card file is unreadable"))
                .expect("test card file is invalid");
        assert_eq!(card_file.codes.len(), 1, "expected exactly one test card");

        let paths = WorkstationPaths::from_root(product_root);
        activate(
            &paths,
            ActivationInput {
                license_code: card_file.codes.into_iter().next().unwrap(),
            },
        )
        .await
        .expect("stage7 activation failed");

        assert_eq!(local_license_status(&paths).unwrap().state, "activated");
    }

    #[test]
    #[ignore = "需要显式提供已经激活的移动盘目录"]
    fn verify_confirmed_portable_root_entitlement_offline() {
        let product_root = std::env::var_os("YUNSHU_STAGE7_PRODUCT_ROOT")
            .map(std::path::PathBuf::from)
            .expect("YUNSHU_STAGE7_PRODUCT_ROOT is required");
        let paths = WorkstationPaths::from_root(product_root);

        assert_eq!(local_license_status(&paths).unwrap().state, "activated");
    }

    #[tokio::test]
    #[ignore = "需要显式提供已经激活的移动盘目录和阶段七服务器"]
    async fn check_in_confirmed_portable_root_with_signed_request() {
        let product_root = std::env::var_os("YUNSHU_STAGE7_PRODUCT_ROOT")
            .map(std::path::PathBuf::from)
            .expect("YUNSHU_STAGE7_PRODUCT_ROOT is required");
        let paths = WorkstationPaths::from_root(product_root);

        check_in(&paths).await.expect("signed check-in failed");
    }
}
