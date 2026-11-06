//! Refine 嵌套栈溢出 smoke（2026-11-06，STATUS_STACK_OVERFLOW 0xc00000fd）
//!
//! 用途：验证「in-loop refine 接力不再无限嵌套」——修复前，真实长会话
//! （948 消息 / 5.12MB / 粗估 140% 上下文窗口）上执行「接着干」会撑爆
//! tokio worker 栈 → 进程被杀（用户看到「闪退」）。
//!
//! 运行：
//!   cargo run -p nuphus --example refine_nesting_smoke
//!
//! 数据来源（可选）：环境变量 `NUPHUS_SMOKE_SESSION_ID` 指定本机 `nuphus.db`
//! 中的一个会话 id，只读其 `sessions.snapshot`。**不写任何数据**。
//! 未指定或数据库不可用时退化为「构造等规模合成会话」，保证可离线运行。
//!
//! 判定：
//!   ✅ 提炼轮次数 ≤ MAX_REFINE_RECURSION_DEPTH（嵌套深度有界）
//!   ✅ 递归深度 ≤ 上限（refine_depth 归零，不残留）
//!   ✅ 进程存活（未栈溢出）
//!   ❌ 修复前：提炼轮数会随嵌套层数增长 → 栈溢出/深度失控

use nuphus::{
    agent::AgentConfig,
    api::{ApiClient, AssistantEvent, MessageRequest, ProviderKind},
    runtime::{Runtime, RuntimeBuilder, RuntimeConfig},
    session::Session,
    tools::registry::{ToolDef, ToolRegistry},
    ToolResult,
};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 记录 LLM 调用次数的 mock——提炼轮每次调用都会返回非空摘要，
/// 因此**只要嵌套无界，调用次数就会无界增长**。
struct CountingLlm {
    calls: Arc<Mutex<usize>>,
}

#[async_trait::async_trait]
impl ApiClient for CountingLlm {
    async fn stream(&self, _request: MessageRequest) -> nuphus::Result<Vec<AssistantEvent>> {
        *self.calls.lock().unwrap() += 1;
        Ok(vec![
            AssistantEvent::TextDelta("摘要：本轮已完成的工作与待办清单。".to_string()),
            AssistantEvent::MessageStop,
        ])
    }

    fn model_name(&self) -> &str {
        "smoke-model"
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::MiniMax
    }

    fn provider_name(&self) -> &str {
        "smoke"
    }
}

/// 最小工具注册表（空即可——提炼轮是代码级禁工具的）。
fn empty_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(ToolDef {
        name: "noop".to_string(),
        description: "noop".to_string(),
        category: nuphus::permissions::ToolCategory::SystemAutomation,
        parameters: serde_json::json!({ "type": "object", "properties": {} }),
        executor: |_p, _c| Ok(ToolResult::success("noop")),
        depends_on: vec![],
    });
    registry
}

/// 崩溃会话的真实规模：948 消息 / 5.12MB（合成会话按此规模构造）。
///
/// 真实会话 id 由 `NUPHUS_SMOKE_SESSION_ID` 提供（不把个人会话标识写进仓库）；
/// 未设置时走等规模合成会话，脚本照样可离线跑。
fn crash_session_id() -> Option<String> {
    std::env::var("NUPHUS_SMOKE_SESSION_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
}

/// 尝试从本机 nuphus.db 读回指定会话的 snapshot（只读；仅 Windows）。
fn load_crash_session(session_id: &str) -> Option<Session> {
    let db = std::path::Path::new(&std::env::var("APPDATA").ok()?)
        .join("Nuphus")
        .join("nuphus.db");
    if !db.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let snap: Option<String> = conn
        .query_row(
            "SELECT snapshot FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .ok()?;
    let snap = snap?;
    serde_json::from_str::<serde_json::Value>(&snap)
        .ok()
        .and_then(|v| {
            // snapshot 是 Session 的序列化形态；直接尝试整体反序列化。
            serde_json::from_value::<Session>(v).ok()
        })
}

/// 离线退化：构造等规模合成会话（948 条消息）。
fn synthetic_session() -> Session {
    let mut session = Session::new();
    for i in 0..948 {
        session.push_user(format!("历史消息 {i}: 这是一次任务的中间产物记录。"));
    }
    session
}

#[tokio::main]
async fn main() {
    // ── 关键：真实崩溃发生在 **tokio worker 线程**（栈 2MB），不是主线程 ──
    // dump 栈顶第 2 帧 = tokio::task::spawn::spawn、第 4 帧 = tauri::async_runtime::spawn。
    // 主线程栈 1MB，worker 2MB——若只跑主线程，等于放宽了崩溃条件。
    // 这里显式把整个场景放进一个 **栈 2MB 的 worker 线程**，与生产条件对齐。
    let handle = std::thread::Builder::new()
        .name("refine-smoke-worker".to_string())
        .stack_size(2 * 1024 * 1024) // = tokio worker 默认栈
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("tokio runtime 构造应成功");
            rt.block_on(run_scenario())
        })
        .expect("worker 线程启动应成功");

    let ok = handle.join().unwrap_or(false);
    if !ok {
        std::process::exit(1);
    }
}

async fn run_scenario() -> bool {
    let max_depth = nuphus::agent::distill::MAX_REFINE_RECURSION_DEPTH;
    println!("== refine 嵌套栈溢出 smoke ==");
    println!("MAX_REFINE_RECURSION_DEPTH = {max_depth}");

    // 1) 载入真实崩溃会话（或等规模合成会话）
    let mut session =
        match crash_session_id().and_then(|id| load_crash_session(&id).map(|s| (id, s))) {
            Some((id, s)) => {
                println!("载入真实会话 {id}: {} 条消息", s.len());
                s
            }
            None => {
                let s = synthetic_session();
                println!("未找到真实会话，使用合成会话: {} 条消息", s.len());
                s
            }
        };

    // 2) 把水位设成「远超 85% 硬线」——模拟崩溃现场的 140% 占用。
    //    api_input_tokens 是 react_loop 判 85% 硬线的官方口径来源。
    let cw: u64 = 1_048_576; // 1M 窗口（MiniMax-M3.1-Flash）
    session.api_input_tokens = (cw as f64 * 1.4) as u64;
    println!(
        "水位: {} / {} tokens = {:.1}%（> 85% 硬线）",
        session.api_input_tokens,
        cw,
        session.api_input_tokens as f64 / cw as f64 * 100.0
    );

    // 3) 构造 Runtime（真实代码路径）
    //    --real：用真实 providers.toml 里的真实模型（走真实 LLM 链路）。
    //    默认：mock LLM（离线可跑、可重复）。
    let real = std::env::args().any(|a| a == "--real");
    let calls = Arc::new(Mutex::new(0usize));
    let llm: Arc<dyn ApiClient> = if real {
        println!("使用真实模型（ClientFactory::live）");
        let factory = nuphus::llm::ClientFactory::live();
        factory
            .create_main_client()
            .expect("真实模型客户端创建应成功")
    } else {
        Arc::new(CountingLlm {
            calls: calls.clone(),
        })
    };
    let mut runtime: Runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(empty_tools())
        .config(RuntimeConfig {
            agent_config: AgentConfig {
                max_iterations: 3,
                enable_memory: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .expect("Runtime 构造应成功");
    runtime.set_session(session);
    runtime.agent_mut().refine_count = 0;
    runtime.agent_mut().ctx_hard_stop_done = false;
    runtime.agent_mut().refine_depth = 0;

    // 4) 走**完整执行路径**：Runtime::run → react_loop（85% 检查）→ refine_in_loop
    //    → resume → react_loop（再次检查）→ …… 这正是崩溃时的真实链路。
    //
    //    直接调 refine_in_loop 只覆盖了环的一环；run() 会额外经过
    //    push_user / 标注注入 / prompt 构建 / LLM 调用 / 事件发射，
    //    与生产的「接着干」路径一致。
    let cancel = AtomicBool::new(false);
    let output = runtime
        .run("接着干", &None, &cancel)
        .await
        .expect("run 应正常返回（不 panic、不栈溢出）");
    let llm_calls = *calls.lock().unwrap();

    println!("--- 结果 ---");
    println!(
        "run 返回: success={} message_len={}",
        output.success,
        output.message.len()
    );
    println!("LLM 调用次数: {llm_calls}");
    println!("refine_depth 残留: {}", runtime.agent().refine_depth);
    println!("refine_count: {}", runtime.agent().refine_count);
    println!("ctx_hard_stop_done: {}", runtime.agent().ctx_hard_stop_done);

    // 5) 判定
    //
    // 嵌套的**真实特征**是 `refine_count > 1`——refine_in_loop 被调用了不止一次。
    // （LLM 调用次数不是可靠指标：完整 run 路径下 = 1 次正常执行 + N 次提炼轮，
    //   N=1 时总数是 2，属正常而非嵌套。）
    let mut ok = true;
    if runtime.agent().refine_depth > max_depth {
        println!(
            "❌ 递归深度超限: {} > {max_depth}",
            runtime.agent().refine_depth
        );
        ok = false;
    }
    let refine_count = runtime.agent().refine_count;
    if refine_count > 1 {
        println!("❌ 发生嵌套接力（refine_count = {refine_count} > 1）——深度闸门未生效");
        ok = false;
    } else {
        println!("✅ 无嵌套接力（refine_count = {refine_count}，提炼只发生一次）");
    }
    if !runtime.agent().ctx_hard_stop_done {
        println!("ℹ️ ctx_hard_stop_done 未置位（本轮未触发 85% 硬止损）");
    }
    println!(
        "{} 进程存活（未栈溢出）；嵌套深度有界",
        if ok { "✅" } else { "❌" }
    );
    ok
}
