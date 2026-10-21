use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router, serve};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use crate::paths::WorkstationPaths;

#[derive(Clone)]
struct AppState {
    paths: WorkstationPaths,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    query: Option<String>,
    category: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
}

#[derive(Debug)]
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": self.0.to_string()
            })),
        )
            .into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self(value.into())
    }
}

pub async fn serve_plugin_center(paths: WorkstationPaths, port: u16) -> Result<()> {
    if port == 0 {
        bail!("插件中心端口不能为 0");
    }
    super::prepare(&paths)?;
    if !paths.plugins_ui.join("index.html").is_file() {
        bail!(
            "插件中心前端尚未部署：{}",
            paths.plugins_ui.join("index.html").display()
        );
    }
    let state = Arc::new(AppState {
        paths: paths.clone(),
    });
    let index = paths.plugins_ui.join("index.html");
    let static_files = ServeDir::new(&paths.plugins_ui).fallback(ServeFile::new(index));
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/status", get(plugin_status))
        .route("/api/plugins/installed", get(installed_plugins))
        .route("/api/plugins/{id}", get(plugin_detail))
        .route("/api/plugins/{id}/install", post(install_plugin))
        .route("/api/plugins/repair", post(repair_plugins))
        .fallback_service(static_files)
        .with_state(state);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("无法绑定插件中心 http://{address}"))?;
    serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("插件中心服务异常退出")
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "ok": true,
        "service": "yunshu-codex-plugin-center",
        "scope": "codex-only",
        "status": super::status(&state.paths)?
    })))
}

async fn list_plugins(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let data = super::catalog::list(
        &state.paths,
        query.query.as_deref(),
        query.category.as_deref(),
        query.page.unwrap_or(1),
        query.page_size.unwrap_or(24),
    )?;
    Ok(Json(json!({"ok": true, "data": data})))
}

async fn plugin_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "ok": true,
        "data": super::catalog::detail(&state.paths, &id)?
    })))
}

async fn plugin_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "ok": true,
        "data": super::status(&state.paths)?
    })))
}

async fn installed_plugins(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "ok": true,
        "data": super::installer::read_registry(&state.paths)?
    })))
}

async fn install_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || super::installer::install(&paths, &id))
        .await
        .context("插件安装任务异常结束")??;
    Ok(Json(json!({"ok": true, "data": result})))
}

async fn repair_plugins(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let paths = state.paths.clone();
    let result = tokio::task::spawn_blocking(move || super::installer::repair(&paths))
        .await
        .context("插件修复任务异常结束")??;
    Ok(Json(json!({"ok": true, "data": result})))
}
