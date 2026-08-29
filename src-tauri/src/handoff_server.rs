//! 外部 Agent 交接门铃 HTTP server（axum，仅绑 127.0.0.1）
//!
//! 服务模式对齐 desktop-api（api/http.rs + main.rs: TcpListener::bind + axum::serve）。
//! 状态不落在这里：事件与令牌都在 nuphus lib 的 HandoffStore 全局单例中，
//! 本模块只是「HTTP → store」的薄适配层，runtime 注入侧经同一 store 被动 drain。
//!
//! 生命周期：Tauri setup 阶段 spawn；bind 失败优雅降级（warn 日志，门铃不可用，
//! 不影响应用启动）。serve 返回（极端情况）同样只记 warn。

use std::path::Path;
use std::sync::OnceLock;

use crate::commands::config::handoff;
use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tauri::AppHandle;

/// done/blocked 自动唤醒用 AppHandle（模式对齐 render/commands.rs 的 APP 静态）
static APP: OnceLock<AppHandle> = OnceLock::new();

/// POST /handoff 请求体
#[derive(Deserialize)]
struct HandoffPayload {
    id: String,
    status: String,
    summary: String,
    report_path: Option<String>,
}

/// POST /handoff/dispatch 请求体 —— Leader 派发前置：一次完成「建 brief + 置 status.json + 取契约」。
/// `brief` 允许为空（Leader 可后补）；`project` / `description` 可选，默认空。
#[derive(Deserialize)]
struct DispatchPayload {
    agent: String,
    task_id: String,
    brief: String,
    /// 产物子目录名（可选）：非空且合法时确保 {agent}/projects/{project}/ 存在，
    /// 对齐 read.md 约定「产物写 projects/{project}/」。
    #[serde(default)]
    project: Option<String>,
    /// agent 初始化描述（可选）：agent 目录未初始化时写入 read.md；缺省用默认描述。
    #[serde(default)]
    description: Option<String>,
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
    // report_path 被 push_event 消费，归组用克隆引用（字符串很短，开销可忽略）
    let report_path = payload.report_path.clone();
    match nuphus::handoff::push_event(
        &payload.id,
        &payload.status,
        &payload.summary,
        payload.report_path,
    ) {
        Ok(()) => {
            // 阶段 0：门铃事件归组 → 按 id 前缀匹配已初始化的 agent 目录并更新其
            // status.json（未命中/无目录静默跳过，绝不破坏既有门铃流程）。
            crate::commands::update_agent_status_from_doorbell(
                &payload.id,
                &payload.status,
                &payload.summary,
                report_path.as_deref(),
            );
            // 阶段 1（方案 v8 六章）：done/blocked 到达 → Leader 空闲时自动开一轮处理。
            // busy 预检在 try_spawn_leader_round 内；忙碌 → 事件留队列，轮次边界自然消化。
            if payload.status == "done" || payload.status == "blocked" {
                if let Some(app) = APP.get() {
                    let verb = if payload.status == "done" {
                        "已完成"
                    } else {
                        "受阻"
                    };
                    let report = report_path.as_deref().unwrap_or("（未提供）");
                    let message = format!(
                        "外部任务 {} {}，summary: {}，验收产物 report_path: {}",
                        payload.id, verb, payload.summary, report
                    );
                    crate::commands::process::try_spawn_leader_round(app.clone(), message);
                }
            }
            StatusCode::OK
        }
        // 参数非法（status 非枚举值 / id 或 summary 为空）→ 400
        Err(e) => {
            tracing::warn!("[Handoff] 拒绝非法事件: {e}");
            StatusCode::BAD_REQUEST
        }
    }
}

/// 派发端点（Leader 派发外部 Agent 的标准前置）。令牌错误 → 403（响应体不含任何令牌线索）；
/// agent/task_id/project 非法或落盘失败 → 400；成功 → 200 {"ok":true,"contract":"..."}。
/// 契约由 build_contract 生成，含 done 门铃 POST 示例、产物绝对路径、report_path。
async fn dispatch(
    headers: HeaderMap,
    Json(payload): Json<DispatchPayload>,
) -> (StatusCode, Json<serde_json::Value>) {
    let token_ok = headers
        .get("X-Handoff-Token")
        .and_then(|v| v.to_str().ok())
        .map(nuphus::handoff::verify_token)
        .unwrap_or(false);
    if !token_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "ok": false })),
        );
    }
    match dispatch_at_root(&handoff::handoff_root(), &payload) {
        Ok(contract) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "contract": contract })),
        ),
        Err(e) => {
            tracing::warn!("[Handoff] 派发拒绝: {e}");
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": e })),
            )
        }
    }
}

/// 派发核心（root 注入：HTTP handler 传 handoff_root()，测试传 tmp 根）。
/// 顺序：校验（先于任何落盘）→ 确保 agent 目录（未初始化则 init，幂等）→
/// 可选产物子目录 → ensure_handoff_at 写 brief + status.json(dispatched+task_id+dispatched_at)。
fn dispatch_at_root(root: &Path, payload: &DispatchPayload) -> Result<String, String> {
    handoff::validate_agent(&payload.agent)?;
    handoff::validate_task_id(&payload.task_id)?;
    // project 作为产物子目录名（可选）：仅 [a-zA-Z0-9_-]——先于任何落盘校验，杜绝路径穿越
    let project = payload.project.as_deref().filter(|p| !p.is_empty());
    if let Some(project) = project {
        if !project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("project 只能包含字母、数字、下划线、连字符".to_string());
        }
    }

    // 未初始化也补建（幂等）；read.md 用 body 的 description 或默认描述
    let description = payload
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("外部 Agent，按 Leader 派发的 brief 完成任务并通过门铃上报");
    handoff::init_agent_at(root, &payload.agent, description)?;

    // 可选产物子目录：对齐 read.md 约定「产物写 projects/{project}/」
    if let Some(project) = project {
        let project_dir = root.join(&payload.agent).join("projects").join(project);
        std::fs::create_dir_all(&project_dir).map_err(|e| format!("创建产物子目录失败: {e}"))?;
    }

    handoff::ensure_handoff_at(root, &payload.agent, &payload.task_id, &payload.brief)
}

/// 连通性自检，无需令牌
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "handoff-doorbell" }))
}

/// 构建路由（提取为独立函数：spawn 与集成测试共用同一路由定义）
fn create_router() -> Router {
    Router::new()
        .route("/handoff", post(post_handoff))
        .route("/handoff/dispatch", post(dispatch))
        .route("/handoff/health", get(health))
}

/// 在 tauri async runtime 上启动门铃 server。
/// 默认端口冲突 → 退化绑 0 由 OS 分配；再失败 → warn 后优雅降级。
pub fn spawn(app: AppHandle) {
    let _ = APP.set(app);
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

    /// 隔离测试根目录：每个用例独立 tmp 子目录，避免污染真实 .nuphus/handoff
    fn tmp_root(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join("nuphus-handoff-server-test");
        let dir = base.join(format!("{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn test_dispatch_end_to_end() {
        tokio_test::block_on(async {
            // HTTP 层（真实 axum server + reqwest，对齐 test_doorbell_end_to_end）：
            // 只覆盖鉴权/参数校验（403/400）——这些路径在任何落盘之前返回；
            // 成功路径的文件效果走 dispatch_at_root 注入 tmp 根，避免触碰真实 .nuphus/handoff。
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

            // 1. token 错误 → 403，响应体不含契约/令牌线索
            let r = client
                .post(format!("{base}/handoff/dispatch"))
                .header("X-Handoff-Token", "wrong-token")
                .json(&serde_json::json!({
                    "agent": "web_agent",
                    "task_id": "task-001",
                    "brief": "x",
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 403);
            let body = r.text().await.unwrap();
            assert!(!body.contains(&token), "403 响应不得泄露令牌");
            assert!(!body.contains("contract"), "403 不得携带契约");

            // 2. 非法 agent → 400（校验发生在任何落盘之前）
            let r = client
                .post(format!("{base}/handoff/dispatch"))
                .header("X-Handoff-Token", &token)
                .json(&serde_json::json!({
                    "agent": "../evil",
                    "task_id": "task-001",
                    "brief": "x",
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);

            // 3. 非法 task_id → 400
            let r = client
                .post(format!("{base}/handoff/dispatch"))
                .header("X-Handoff-Token", &token)
                .json(&serde_json::json!({
                    "agent": "web_agent",
                    "task_id": "a/b",
                    "brief": "x",
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);

            // 4. 非法 project → 400（路径安全，杜绝穿越）
            let r = client
                .post(format!("{base}/handoff/dispatch"))
                .header("X-Handoff-Token", &token)
                .json(&serde_json::json!({
                    "agent": "web_agent",
                    "task_id": "task-001",
                    "brief": "x",
                    "project": "../evil",
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);
        });

        // ── 成功路径：注入 tmp 根（走 dispatch_at_root，不触碰真实 .nuphus/handoff）──
        let root = tmp_root("dispatch");
        let payload = DispatchPayload {
            agent: "claude-code".to_string(),
            task_id: "task-001".to_string(),
            brief: "任务：重构登录页".to_string(),
            project: Some("web-redesign".to_string()),
            description: Some("负责前端编码".to_string()),
        };
        let contract = dispatch_at_root(&root, &payload).unwrap();
        let dir = root.join("claude-code");

        // agent 目录就绪：read.md 用 body 的 description（未初始化时 init 联动）
        assert!(dir.join("read.md").is_file());
        assert!(dir.join("memory.md").is_file());
        let read = std::fs::read_to_string(dir.join("read.md")).unwrap();
        assert!(read.contains("负责前端编码"));

        // brief 就绪
        assert_eq!(
            std::fs::read_to_string(dir.join("briefs").join("task-001-brief.md")).unwrap(),
            "任务：重构登录页"
        );

        // status.json：dispatched + task_id + dispatched_at；token 不落盘
        // （上板≠执行：in_progress 由外部 Agent 拉铃触发，此处仅置 dispatched）
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(status["state"], "dispatched");
        assert_eq!(status["task_id"], "task-001");
        assert!(status["dispatched_at"].is_string());
        let status_str = std::fs::read_to_string(dir.join("status.json")).unwrap();
        assert!(!status_str.contains("token"), "token 不得落 status.json");

        // 产物子目录就绪
        assert!(dir.join("projects").join("web-redesign").is_dir());

        // 契约含 CLI 上报示例（done/blocked）+ 门铃事件 id + 令牌行（门铃语义=交付上报，不含 ready）
        // cli_cmd 前缀为 current_exe 动态值，断言不依赖前缀
        assert!(contract.contains("task done --id claude-code::task-001"));
        assert!(contract.contains("task blocked --id claude-code::task-001"));
        assert!(!contract.contains("curl"), "契约不得再宣传 curl 上报");
        assert!(!contract.contains("\"status\":\"done\""));
        assert!(!contract.contains("\"status\":\"ready\""));
        assert!(contract.contains("claude-code::task-001"));
        assert!(contract.contains("令牌 token:"));

        // 幂等：二次派发不破坏既有结构
        let contract2 = dispatch_at_root(&root, &payload).unwrap();
        assert!(contract2.contains("claude-code::task-001"));
        assert!(dir.join("briefs").join("task-001-brief.md").is_file());
    }
}
