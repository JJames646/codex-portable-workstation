use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

const RELAY_PREVIEW_WINDOW_LABEL: &str = "relay-preview";
const RELAY_BASE_URL: Option<&str> = option_env!("CODEX_WORKSTATION_RELAY_BASE_URL");
const USER_UI_REQUEST_HEADER: &str = "X-User-UI-Request";

static RELAY_SESSION: OnceLock<Mutex<Option<RelaySession>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult<T>
where
    T: Serialize,
{
    pub status: String,
    pub message: String,
    #[serde(flatten)]
    pub payload: T,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayDashboardPayload {
    pub configured: bool,
    pub auth_status: String,
    pub account_label: Option<String>,
    pub balance: f64,
    pub stats: Option<RelayDashboardStats>,
    pub trend: Vec<RelayTrendPoint>,
    pub models: Vec<RelayModelStat>,
    pub subscription: Option<RelaySubscriptionSummary>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelayDashboardStats {
    pub total_api_keys: i64,
    pub active_api_keys: i64,
    pub total_requests: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub total_actual_cost: f64,
    pub today_requests: i64,
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_tokens: i64,
    pub today_cost: f64,
    pub today_actual_cost: f64,
    pub average_duration_ms: f64,
    pub rpm: i64,
    pub tpm: i64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelayTrendPoint {
    pub date: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub actual_cost: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelayModelStat {
    pub model: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub actual_cost: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelaySubscriptionSummary {
    pub active_count: i64,
    pub total_used_usd: f64,
    pub subscriptions: Vec<RelaySubscriptionItem>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RelaySubscriptionItem {
    pub group_name: String,
    pub status: String,
    pub daily_used_usd: f64,
    pub daily_limit_usd: f64,
    pub weekly_used_usd: f64,
    pub weekly_limit_usd: f64,
    pub monthly_used_usd: f64,
    pub monthly_limit_usd: f64,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
struct RelaySession {
    access_token: String,
    refresh_token: Option<String>,
    expires_at_ms: Option<u64>,
    user: Value,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    code: Value,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    requires_2fa: bool,
    #[serde(default)]
    user: Value,
}

#[derive(Debug, Serialize)]
struct LoginBody<'a> {
    email: &'a str,
    password: &'a str,
    turnstile_token: &'a str,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RelayUser {
    email: String,
    username: String,
    balance: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RelayTrendResponse {
    trend: Vec<RelayTrendPoint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RelayModelsResponse {
    models: Vec<RelayModelStat>,
}

#[derive(Debug)]
struct RelayApiError {
    message: String,
    unauthorized: bool,
}

#[tauri::command]
pub async fn relay_dashboard_status() -> CommandResult<RelayDashboardPayload> {
    let base_url = match parse_relay_base_url(RELAY_BASE_URL) {
        Ok(base_url) => base_url,
        Err(error) => return failed(&error, empty_dashboard(false, "unavailable")),
    };
    let Some(session) = current_session() else {
        return ok(
            "请先登录中转站账号后查看个人用量。",
            empty_dashboard(true, "required"),
        );
    };

    match load_dashboard(&base_url, &session).await {
        Ok(payload) => ok("中转站个人用量已更新。", payload),
        Err(error) if error.unauthorized => {
            clear_session();
            ok(
                "中转站登录已失效，请重新登录。",
                empty_dashboard(true, "required"),
            )
        }
        Err(error) => failed(
            &format!("读取中转站个人用量失败：{}", error.message),
            empty_dashboard(true, "authenticated"),
        ),
    }
}

#[tauri::command]
pub async fn relay_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: RelayLoginRequest,
) -> CommandResult<RelayDashboardPayload> {
    let base_url = match parse_relay_base_url(RELAY_BASE_URL) {
        Ok(base_url) => base_url,
        Err(error) => return failed(&error, empty_dashboard(false, "unavailable")),
    };
    let email = request.email.trim();
    if email.is_empty() || email.len() > 320 || !email.contains('@') {
        return failed(
            "请输入有效的中转站登录邮箱。",
            empty_dashboard(true, "required"),
        );
    }
    if request.password.is_empty() || request.password.len() > 4096 {
        return failed(
            "请输入有效的中转站登录密码。",
            empty_dashboard(true, "required"),
        );
    }

    let client = match relay_http_client(&base_url) {
        Ok(client) => client,
        Err(error) => {
            return failed(
                &format!("创建中转站连接失败：{error}"),
                empty_dashboard(true, "required"),
            );
        }
    };
    let endpoint = match base_url.join("api/v1/auth/login") {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return failed(
                &format!("生成中转站登录地址失败：{error}"),
                empty_dashboard(true, "required"),
            );
        }
    };
    let response = client
        .post(endpoint)
        .header(USER_UI_REQUEST_HEADER, "1")
        .json(&LoginBody {
            email,
            password: &request.password,
            turnstile_token: "",
        })
        .send()
        .await;
    let login = match response {
        Ok(response) => match parse_api_response::<LoginResponse>(response).await {
            Ok(login) => login,
            Err(error) => {
                return failed(
                    &format!("中转站登录失败：{}", error.message),
                    empty_dashboard(true, "required"),
                );
            }
        },
        Err(error) => {
            return failed(
                &format!(
                    "无法连接中转站登录服务：{}",
                    concise_error(&error.to_string())
                ),
                empty_dashboard(true, "required"),
            );
        }
    };
    if login.requires_2fa {
        return failed(
            "当前账号启用了二次验证，本地预览暂未接入验证码步骤。",
            empty_dashboard(true, "required"),
        );
    }
    if login.access_token.trim().is_empty() {
        return failed(
            "中转站登录响应缺少访问令牌。",
            empty_dashboard(true, "required"),
        );
    }

    let session = RelaySession {
        access_token: login.access_token,
        refresh_token: (!login.refresh_token.is_empty()).then_some(login.refresh_token),
        expires_at_ms: (login.expires_in > 0)
            .then(|| unix_time_ms().saturating_add(login.expires_in.saturating_mul(1000))),
        user: login.user,
    };
    replace_session(session.clone());
    if let Some(window) = app.get_webview_window(RELAY_PREVIEW_WINDOW_LABEL) {
        let _ = window.destroy();
    }

    match load_dashboard(&base_url, &session).await {
        Ok(payload) => ok("中转站登录成功，个人用量已加载。", payload),
        Err(error) if error.unauthorized => {
            clear_session();
            failed(
                "中转站登录状态未通过用户接口校验，请重新登录。",
                empty_dashboard(true, "required"),
            )
        }
        Err(error) => failed(
            &format!("登录成功，但读取个人用量失败：{}", error.message),
            empty_dashboard(true, "authenticated"),
        ),
    }
}

#[tauri::command]
pub fn relay_logout<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> CommandResult<RelayDashboardPayload> {
    clear_session();
    if let Some(window) = app.get_webview_window(RELAY_PREVIEW_WINDOW_LABEL) {
        let _ = window.destroy();
    }
    ok(
        "已退出中转站账号。",
        empty_dashboard(parse_relay_base_url(RELAY_BASE_URL).is_ok(), "required"),
    )
}

#[tauri::command]
pub async fn open_relay_preview<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> CommandResult<RelayDashboardPayload> {
    let base_url = match parse_relay_base_url(RELAY_BASE_URL) {
        Ok(base_url) => base_url,
        Err(error) => return failed(&error, empty_dashboard(false, "unavailable")),
    };
    let Some(session) = current_session() else {
        return failed(
            "请先在首页登录中转站账号。",
            empty_dashboard(true, "required"),
        );
    };
    if let Some(window) = app.get_webview_window(RELAY_PREVIEW_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return ok(
            "已显示中转站用户仪表盘。",
            empty_dashboard(true, "authenticated"),
        );
    }

    let dashboard_url = match base_url.join("dashboard") {
        Ok(url) => url,
        Err(error) => {
            return failed(
                &format!("生成中转站用户页面地址失败：{error}"),
                empty_dashboard(true, "authenticated"),
            );
        }
    };
    let initialization_script = relay_session_initialization_script(&session);
    let allowed_origin = base_url.clone();
    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        RELAY_PREVIEW_WINDOW_LABEL,
        tauri::WebviewUrl::External(dashboard_url),
    )
    .title("中转站个人仪表盘")
    .inner_size(1280.0, 860.0)
    .min_inner_size(960.0, 700.0)
    .incognito(true)
    .devtools(cfg!(debug_assertions))
    .initialization_script(&initialization_script)
    .on_navigation(move |target| relay_preview_navigation_allowed(target, &allowed_origin));

    match builder.build() {
        Ok(_) => ok(
            "已打开中转站用户仪表盘。",
            empty_dashboard(true, "authenticated"),
        ),
        Err(error) => failed(
            &format!("打开中转站用户仪表盘失败：{error}"),
            empty_dashboard(true, "authenticated"),
        ),
    }
}

async fn load_dashboard(
    base_url: &tauri::webview::Url,
    session: &RelaySession,
) -> Result<RelayDashboardPayload, RelayApiError> {
    let client = relay_http_client(base_url).map_err(|error| RelayApiError {
        message: concise_error(&error.to_string()),
        unauthorized: false,
    })?;
    let user: RelayUser = api_get(
        &client,
        base_url,
        "api/v1/user/profile",
        &session.access_token,
    )
    .await?;
    let stats: RelayDashboardStats = api_get(
        &client,
        base_url,
        "api/v1/usage/dashboard/stats",
        &session.access_token,
    )
    .await?;
    let trend = api_get::<RelayTrendResponse>(
        &client,
        base_url,
        "api/v1/usage/dashboard/trend?granularity=day",
        &session.access_token,
    )
    .await
    .unwrap_or_default();
    let models = api_get::<RelayModelsResponse>(
        &client,
        base_url,
        "api/v1/usage/dashboard/models",
        &session.access_token,
    )
    .await
    .unwrap_or_default();
    let subscription = api_get::<RelaySubscriptionSummary>(
        &client,
        base_url,
        "api/v1/subscriptions/summary",
        &session.access_token,
    )
    .await
    .ok();
    let account_label = if user.username.trim().is_empty() {
        user.email
    } else {
        user.username
    };

    Ok(RelayDashboardPayload {
        configured: true,
        auth_status: "authenticated".to_string(),
        account_label: Some(account_label),
        balance: user.balance,
        stats: Some(stats),
        trend: trend.trend,
        models: models.models,
        subscription,
    })
}

async fn api_get<T: DeserializeOwned>(
    client: &reqwest::Client,
    base_url: &tauri::webview::Url,
    path: &str,
    access_token: &str,
) -> Result<T, RelayApiError> {
    let endpoint = base_url.join(path).map_err(|error| RelayApiError {
        message: format!("接口地址无效：{error}"),
        unauthorized: false,
    })?;
    let response = client
        .get(endpoint)
        .header(USER_UI_REQUEST_HEADER, "1")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| RelayApiError {
            message: concise_error(&error.to_string()),
            unauthorized: false,
        })?;
    parse_api_response(response).await
}

async fn parse_api_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, RelayApiError> {
    let status = response.status();
    let body = response.text().await.map_err(|error| RelayApiError {
        message: concise_error(&error.to_string()),
        unauthorized: status == StatusCode::UNAUTHORIZED,
    })?;
    let envelope = serde_json::from_str::<ApiEnvelope<T>>(&body).map_err(|_| RelayApiError {
        message: format!("服务器返回了无法识别的响应（HTTP {}）。", status.as_u16()),
        unauthorized: status == StatusCode::UNAUTHORIZED,
    })?;
    let success = envelope.code.as_i64() == Some(0) || envelope.code.as_str() == Some("0");
    if !status.is_success() || !success {
        return Err(RelayApiError {
            message: concise_error(if envelope.message.is_empty() {
                "请求未成功"
            } else {
                &envelope.message
            }),
            unauthorized: status == StatusCode::UNAUTHORIZED,
        });
    }
    envelope.data.ok_or_else(|| RelayApiError {
        message: "服务器响应缺少 data 字段。".to_string(),
        unauthorized: false,
    })
}

fn relay_http_client(base_url: &tauri::webview::Url) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(format!(
            "CodexPortableWorkstation/{}",
            env!("CARGO_PKG_VERSION")
        ));
    if base_url.host_str().is_some_and(is_private_or_loopback_host) {
        builder = builder.no_proxy();
    }
    Ok(builder.build()?)
}

fn parse_relay_base_url(raw: Option<&str>) -> Result<tauri::webview::Url, String> {
    let raw = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "当前构建未配置中转站服务器地址。".to_string())?;
    let mut endpoint = raw
        .parse::<tauri::webview::Url>()
        .map_err(|error| format!("中转站服务器地址无效：{error}"))?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err("中转站服务器地址只允许使用 http 或 https。".to_string());
    }
    if endpoint.host_str().is_none() {
        return Err("中转站服务器地址缺少主机名。".to_string());
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err("中转站服务器地址不得包含账号或密码。".to_string());
    }
    if endpoint.scheme() == "http" && !endpoint.host_str().is_some_and(is_private_or_loopback_host)
    {
        return Err("公网中转站必须使用 HTTPS；HTTP 仅允许本机或局域网预览。".to_string());
    }
    endpoint.set_path("/");
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(endpoint)
}

fn is_private_or_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => address.is_private() || address.is_loopback(),
        Ok(std::net::IpAddr::V6(address)) => address.is_loopback(),
        Err(_) => false,
    }
}

fn relay_preview_navigation_allowed(
    target: &tauri::webview::Url,
    allowed_origin: &tauri::webview::Url,
) -> bool {
    if target.as_str() == "about:blank" {
        return true;
    }
    target.scheme() == allowed_origin.scheme()
        && target.host_str() == allowed_origin.host_str()
        && target.port_or_known_default() == allowed_origin.port_or_known_default()
}

fn relay_session_initialization_script(session: &RelaySession) -> String {
    let access_token =
        serde_json::to_string(&session.access_token).unwrap_or_else(|_| "\"\"".into());
    let auth_user_value = serde_json::to_string(&session.user).unwrap_or_else(|_| "{}".into());
    let auth_user = serde_json::to_string(&auth_user_value).unwrap_or_else(|_| "\"{}\"".into());
    let refresh_token = session
        .refresh_token
        .as_ref()
        .and_then(|token| serde_json::to_string(token).ok());
    let expires_at = session.expires_at_ms.map(|value| value.to_string());
    let refresh_statement = refresh_token.map_or_else(
        || "localStorage.removeItem('refresh_token');".to_string(),
        |value| format!("localStorage.setItem('refresh_token', {value});"),
    );
    let expiry_statement = expires_at.map_or_else(
        || "localStorage.removeItem('token_expires_at');".to_string(),
        |value| format!("localStorage.setItem('token_expires_at', '{value}');"),
    );
    format!(
        r#"
(() => {{
  try {{
    localStorage.setItem('auth_token', {access_token});
    localStorage.setItem('auth_user', {auth_user});
    {refresh_statement}
    {expiry_statement}
  }} catch (_) {{}}
}})();
"#
    )
}

fn current_session() -> Option<RelaySession> {
    relay_session_store()
        .lock()
        .ok()
        .and_then(|session| session.clone())
}

fn replace_session(session: RelaySession) {
    if let Ok(mut current) = relay_session_store().lock() {
        *current = Some(session);
    }
}

fn clear_session() {
    if let Ok(mut current) = relay_session_store().lock() {
        *current = None;
    }
}

fn relay_session_store() -> &'static Mutex<Option<RelaySession>> {
    RELAY_SESSION.get_or_init(|| Mutex::new(None))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn empty_dashboard(configured: bool, auth_status: &str) -> RelayDashboardPayload {
    RelayDashboardPayload {
        configured,
        auth_status: auth_status.to_string(),
        account_label: None,
        balance: 0.0,
        stats: None,
        trend: Vec::new(),
        models: Vec::new(),
        subscription: None,
    }
}

fn concise_error(message: &str) -> String {
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}

fn ok<T: Serialize>(message: &str, payload: T) -> CommandResult<T> {
    CommandResult {
        status: "ok".to_string(),
        message: message.to_string(),
        payload,
    }
}

fn failed<T: Serialize>(message: &str, payload: T) -> CommandResult<T> {
    CommandResult {
        status: "failed".to_string(),
        message: message.to_string(),
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_base_url_accepts_private_http_and_public_https() {
        assert!(parse_relay_base_url(Some("http://192.168.1.181:8080")).is_ok());
        assert!(parse_relay_base_url(Some("https://relay.example.com")).is_ok());
    }

    #[test]
    fn relay_base_url_rejects_public_http_credentials_and_non_http() {
        assert!(parse_relay_base_url(Some("http://relay.example.com")).is_err());
        assert!(parse_relay_base_url(Some("https://user:secret@example.com")).is_err());
        assert!(parse_relay_base_url(Some("file:///C:/Windows/win.ini")).is_err());
    }

    #[test]
    fn relay_navigation_stays_on_configured_origin() {
        let allowed = parse_relay_base_url(Some("http://192.168.1.181:8080")).unwrap();
        let dashboard = "http://192.168.1.181:8080/dashboard"
            .parse::<tauri::webview::Url>()
            .unwrap();
        let different_port = "http://192.168.1.181:9090/dashboard"
            .parse::<tauri::webview::Url>()
            .unwrap();
        let external = "https://example.com"
            .parse::<tauri::webview::Url>()
            .unwrap();

        assert!(relay_preview_navigation_allowed(&dashboard, &allowed));
        assert!(!relay_preview_navigation_allowed(&different_port, &allowed));
        assert!(!relay_preview_navigation_allowed(&external, &allowed));
    }

    #[test]
    fn initialization_script_contains_session_keys_without_password_fields() {
        let session = RelaySession {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            expires_at_ms: Some(123),
            user: serde_json::json!({"email": "user@example.com"}),
        };
        let script = relay_session_initialization_script(&session);

        assert!(script.contains("auth_token"));
        assert!(script.contains("refresh_token"));
        assert!(script.contains("auth_user"));
        assert!(!script.contains("password"));
    }
}
