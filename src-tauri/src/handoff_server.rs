//! 外部 Agent 交接门铃 HTTP server（axum，仅绑 127.0.0.1）
//!
//! 服务模式对齐 desktop-api（api/http.rs + main.rs: TcpListener::bind + axum::serve）。
//! 状态不落在这里：事件与令牌都在 nuphus lib 的 HandoffStore 全局单例中，
//! 本模块只是「HTTP → store」的薄适配层，runtime 注入侧经同一 store 被动 drain。
//!
//! 生命周期：Tauri setup 阶段 spawn；bind 失败优雅降级（warn 日志，门铃不可用，
//! 不影响应用启动）。serve 返回（极端情况）同样只记 warn。

use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;

/// POST /handoff 请求体
#[derive(Deserialize)]
struct HandoffPayload {
    id: String,
    status: String,
    summary: String,
    report_path: Option<String>,
}

/// 完工/进度上报入口。令牌错误 → 403，响应体不含任何提示正确令牌的信息。
async fn post_handoff(headers: HeaderMap, Json(payload): Json<HandoffPayload>) -> StatusCode {
    let token_ok = headers
        .get("X-Handoff-Token")
        .and_then(|v| v.to_str().ok())
        .map(nuphus::handoff::verify_token)
        .unwrap_or(false);
    if !token_ok {
        return StatusCode::FORBIDDEN;
    }
    match nuphus::handoff::push_event(
        &payload.id,
        &payload.status,
        &payload.summary,
        payload.report_path,
    ) {
        Ok(()) => StatusCode::OK,
        // 参数非法（status 非枚举值 / id 或 summary 为空）→ 400
        Err(e) => {
            tracing::warn!("[Handoff] 拒绝非法事件: {e}");
            StatusCode::BAD_REQUEST
        }
    }
}

/// 连通性自检，无需令牌
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "handoff-doorbell" }))
}

/// 构建路由（提取为独立函数：spawn 与集成测试共用同一路由定义）
fn create_router() -> Router {
    Router::new()
        .route("/handoff", post(post_handoff))
        .route("/handoff/health", get(health))
}

/// 在 tauri async runtime 上启动门铃 server。
/// 默认端口冲突 → 退化绑 0 由 OS 分配；再失败 → warn 后优雅降级。
pub fn spawn() {
    tauri::async_runtime::spawn(async {
        let app = create_router();

        let listener = match bind_with_fallback().await {
            Some(l) => l,
            None => return, // 优雅降级：日志已在 bind_with_fallback 内记录
        };
        // bind 已成功，local_addr 失败属异常路径——拿不到端口就无法对外公布地址，直接降级
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                tracing::warn!("[Handoff] 读取监听地址失败，门铃降级不可用: {e}");
                return;
            }
        };
        nuphus::handoff::set_bound_port(port);
        tracing::info!("[Handoff] 门铃 server 监听 http://127.0.0.1:{port}/handoff");

        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!("[Handoff] 门铃 server 异常退出: {e}");
        }
    });
}

/// 仅绑 127.0.0.1：固定端口 → 失败退化 0（OS 分配）→ 再失败返回 None。
async fn bind_with_fallback() -> Option<tokio::net::TcpListener> {
    let default = nuphus::handoff::DEFAULT_DOORBELL_PORT;
    match tokio::net::TcpListener::bind(("127.0.0.1", default)).await {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!("[Handoff] 默认端口 {default} 绑定失败（{e}），退化为 OS 分配端口");
            match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
                Ok(l) => Some(l),
                Err(e2) => {
                    tracing::warn!("[Handoff] 门铃 server 启动失败，优雅降级（不影响应用）: {e2}");
                    None
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    //! 端到端自验证：真实 axum server 绑 127.0.0.1 + reqwest 客户端，
    //! 等价于外部 Agent 的 curl 调用路径（无需启动 Tauri 前端）。
    use super::*;

    #[test]
    fn test_doorbell_end_to_end() {
        tokio_test::block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let _ = axum::serve(listener, create_router()).await;
            });
            let base = format!("http://127.0.0.1:{port}");
            let token = nuphus::handoff::doorbell_info().token;
            // no_proxy：本机系统代理（如 Clash）会随机拦截 loopback 请求导致 10053/10054 假失败
            let client = reqwest::Client::builder().no_proxy().build().unwrap();

            // 1. health 无需令牌 → 200 ok
            let r = client
                .get(format!("{base}/handoff/health"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            assert!(r.text().await.unwrap().contains("ok"));

            let payload = serde_json::json!({
                "id": "e2e-01", "status": "done",
                "summary": "完成重构", "report_path": ".nuphus/handoff/e2e-01-report.md"
            });

            // 2. 错误令牌 → 403，且响应体不泄露正确令牌
            let r = client
                .post(format!("{base}/handoff"))
                .header("X-Handoff-Token", "wrong-token")
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 403);
            let body = r.text().await.unwrap();
            assert!(!body.contains(&token), "403 响应不得泄露令牌");

            // 3. 缺令牌头 → 403
            let r = client
                .post(format!("{base}/handoff"))
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 403);

            // 4. 正确令牌 → 200，事件进 store
            let r = client
                .post(format!("{base}/handoff"))
                .header("X-Handoff-Token", &token)
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);

            // 5. 非法 status → 400
            let r = client
                .post(format!("{base}/handoff"))
                .header("X-Handoff-Token", &token)
                .json(&serde_json::json!({"id": "e2e-02", "status": "running", "summary": "x"}))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);

            // 6. 轮次边界 drain 能取到该事件，且取后为空（注入恰好一次）
            let events = nuphus::handoff::drain_for_injection();
            let hit = events
                .iter()
                .find(|e| e.id == "e2e-01")
                .expect("done 事件应已进入待注入队列");
            assert_eq!(hit.status, nuphus::handoff::HandoffStatus::Done);
            assert_eq!(
                hit.report_path.as_deref(),
                Some(".nuphus/handoff/e2e-01-report.md")
            );
            assert!(nuphus::handoff::drain_for_injection().is_empty());
        });
    }
}
