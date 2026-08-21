//! HTTP 服务 - axum, 6 核心端点

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Json as AxumJson, Response},
    routing::{delete, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::api::{Action, UnifiedApi};
use crate::core::*;
use crate::vision::{PerceiveWhat, Query};

/// 应用状态
pub struct AppState {
    api: UnifiedApi,
    sessions: Arc<RwLock<SessionManager>>,
}

/// 错误 → HTTP 响应
impl IntoResponse for DesktopError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            DesktopError::TargetNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DesktopError::InputFailed(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            DesktopError::CaptureFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            DesktopError::OcrFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            DesktopError::LocateFailed(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DesktopError::ActivationFailed(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            DesktopError::AllStrategiesFailed => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            DesktopError::SessionNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DesktopError::PlatformNotSupported => (StatusCode::NOT_IMPLEMENTED, self.to_string()),
            DesktopError::Other(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, AxumJson(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// 创建路由
pub fn create_router() -> Router {
    let state = Arc::new(AppState {
        api: UnifiedApi::new(),
        sessions: Arc::new(RwLock::new(SessionManager::new())),
    });

    Router::new()
        // 会话管理
        .route("/v2/session", post(create_session))
        .route("/v2/session/:id", delete(close_session))
        // 核心操作
        .route("/v2/session/:id/see", post(session_see))
        .route("/v2/session/:id/find", post(session_find))
        .route("/v2/session/:id/do", post(session_do))
        .route("/v2/session/:id/say", post(session_say))
        // 健康检查
        .route("/health", axum::routing::get(health))
        .with_state(state)
}

// ─────────────────────────────── 请求/响应类型 ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub target: TargetSpec,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub id: Uuid,
    pub kind: AppKind,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SeeRequest {
    #[serde(default)]
    pub scope: Scope,
    #[serde(default = "default_perceive_what")]
    pub what: String,
}

fn default_perceive_what() -> String {
    "all".to_string()
}

#[derive(Debug, Deserialize)]
pub struct FindRequest {
    #[serde(flatten)]
    pub query: QueryRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "by")]
pub enum QueryRequest {
    Image {
        image_base64: String,
    },
    Text {
        query: String,
    },
    Color {
        r: u8,
        g: u8,
        b: u8,
        tolerance: Option<u8>,
    },
}

#[derive(Debug, Deserialize)]
pub struct DoRequest {
    pub action: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub key: Option<String>,
    pub keys: Option<Vec<String>>,
    pub start_x: Option<i32>,
    pub start_y: Option<i32>,
    pub end_x: Option<i32>,
    pub end_y: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SayRequest {
    pub text: String,
    #[serde(default = "default_verify")]
    pub verify: bool,
}

fn default_verify() -> bool {
    true
}

// ─────────────────────────────── 端点实现 ───────────────────────────────

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<AxumJson<CreateSessionResponse>> {
    let mut sessions = state.sessions.write().await;
    let handle = sessions.create(req.target).await?;

    Ok(AxumJson(CreateSessionResponse {
        id: handle.id,
        kind: handle.kind,
        created_at: handle.created_at.to_rfc3339(),
    }))
}

async fn close_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<AxumJson<serde_json::Value>> {
    let mut sessions = state.sessions.write().await;
    sessions.close(id).await?;

    Ok(AxumJson(serde_json::json!({ "success": true })))
}

async fn session_see(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SeeRequest>,
) -> Result<AxumJson<serde_json::Value>> {
    let sessions = state.sessions.read().await;
    let session = sessions.get(id).ok_or(DesktopError::SessionNotFound(id))?;

    let what = match req.what.as_str() {
        "text" => PerceiveWhat::Text,
        "elements" => PerceiveWhat::Elements,
        "colors" => PerceiveWhat::Colors,
        _ => PerceiveWhat::All,
    };

    let perception = state.api.see(&session, req.scope, what).await?;

    // TODO: 将 Perception 序列化为 JSON
    Ok(AxumJson(serde_json::json!({
        "frame_id": perception.frame_id,
        "timestamp": perception.timestamp.to_rfc3339(),
        "texts_count": perception.texts.len(),
        "elements_count": perception.elements.len(),
        "colors_count": perception.colors.len(),
    })))
}

async fn session_find(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<FindRequest>,
) -> Result<AxumJson<serde_json::Value>> {
    let sessions = state.sessions.read().await;
    let session = sessions.get(id).ok_or(DesktopError::SessionNotFound(id))?;

    let query = match req.query {
        QueryRequest::Image { image_base64 } => {
            let bytes = STANDARD
                .decode(image_base64)
                .map_err(|e| DesktopError::Other(anyhow::anyhow!("base64 decode: {}", e)))?;
            Query::Image(bytes)
        }
        QueryRequest::Text { query } => Query::Text(query),
        QueryRequest::Color { r, g, b, tolerance } => Query::Color {
            target: Color { r, g, b, a: 255 },
            tolerance: tolerance.unwrap_or(10),
        },
    };

    let result = state.api.find(&session, &query).await?;

    Ok(AxumJson(serde_json::json!({
        "found": result.found,
        "rect": result.rect.map(|r| serde_json::json!({
            "x": r.x, "y": r.y, "w": r.w, "h": r.h
        })),
        "confidence": result.confidence,
        "method": format!("{:?}", result.method),
    })))
}

async fn session_do(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<DoRequest>,
) -> Result<AxumJson<serde_json::Value>> {
    let sessions = state.sessions.read().await;
    let session = sessions.get(id).ok_or(DesktopError::SessionNotFound(id))?;

    let action = match req.action.as_str() {
        "click" => Action::Click {
            x: req.x.unwrap_or(0),
            y: req.y.unwrap_or(0),
        },
        "double_click" => Action::DoubleClick {
            x: req.x.unwrap_or(0),
            y: req.y.unwrap_or(0),
        },
        "drag" => Action::Drag {
            start: Point {
                x: req.start_x.unwrap_or(0),
                y: req.start_y.unwrap_or(0),
            },
            end: Point {
                x: req.end_x.unwrap_or(0),
                y: req.end_y.unwrap_or(0),
            },
        },
        "press" => Action::Press {
            key: req.key.unwrap_or_default(),
        },
        "hotkey" => Action::Hotkey {
            keys: req.keys.unwrap_or_default(),
        },
        _ => {
            return Err(DesktopError::Other(anyhow::anyhow!(
                "unknown action: {}",
                req.action
            )))
        }
    };

    let result = state.api.do_(&session, action).await?;

    Ok(AxumJson(serde_json::json!({
        "success": matches!(result, crate::api::DoResult::Success),
    })))
}

async fn session_say(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SayRequest>,
) -> Result<AxumJson<serde_json::Value>> {
    let sessions = state.sessions.read().await;
    let session = sessions.get(id).ok_or(DesktopError::SessionNotFound(id))?;

    let result = state.api.say(&session, &req.text).await?;

    let (success, status) = match result {
        crate::api::SayResult::Delivered => (true, "delivered".to_string()),
        crate::api::SayResult::Partial { sent, failed } => {
            (true, format!("partial: sent={}, failed={}", sent, failed))
        }
        crate::api::SayResult::Failed(reason) => (false, reason),
    };

    Ok(AxumJson(serde_json::json!({
        "success": success,
        "status": status,
    })))
}

async fn health() -> AxumJson<serde_json::Value> {
    AxumJson(serde_json::json!({
        "status": "ok",
        "service": "desktop-api",
        "version": "2.0.0",
    }))
}

// base64 解码 (简单实现)
use base64::{engine::general_purpose::STANDARD, Engine as _};
