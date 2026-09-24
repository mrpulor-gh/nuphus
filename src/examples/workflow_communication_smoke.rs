//! Opt-in live WorkflowAgent -> WorkflowEngine communication regression.
//! `cargo run -p nuphus --example workflow_communication_smoke -- --run`
//! Only fixture files may be read/written. No shell, browser, or desktop tools.
//! Credentials are read from the existing application configuration, never printed.
//! Like the desktop smoke example, the synthetic session may append to the normal
//! history database; workflow and file artifacts are isolated under target/.
use anyhow::{ensure, Context, Result};
use nuphus::{
    agent::events::{EventEmitter, NuphusEvent},
    config,
    llm::ClientFactory,
    permissions::ToolPermissions,
    runtime::WorkflowAgent,
    tools::{
        registry::{ToolCtx, ToolDef},
        ToolRegistry,
    },
    workflow::{store::WorkflowStore, types::RunStatus, WorkflowEngine},
    ToolResult,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static WRITE: OnceLock<ToolDef> = OnceLock::new();
static READ: OnceLock<ToolDef> = OnceLock::new();

fn fixture_path(params: &Value) -> std::result::Result<(), String> {
    let path = PathBuf::from(params["path"].as_str().ok_or("path required")?);
    let root = ROOT.get().ok_or("fixture not initialized")?;
    let allowed = [
        "result.txt",
        "plugin/workflows/communication-smoke/workflow.json",
        "plugin/workflows/communication-smoke/params.json",
        "plugin/workflows/communication-smoke/guide.md",
        "plugin/workflows/index.json",
        "src/workflow/step_schema.json",
    ];
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if !allowed.iter().any(|relative| path == root.join(relative)) {
        return Err("Only explicitly listed smoke fixture files are available".into());
    }
    Ok(())
}
fn fixture_write(params: &Value, ctx: &ToolCtx) -> std::result::Result<ToolResult, String> {
    fixture_path(params)?;
    (WRITE.get().unwrap().executor)(params, ctx)
}
fn fixture_read(params: &Value, ctx: &ToolCtx) -> std::result::Result<ToolResult, String> {
    fixture_path(params)?;
    (READ.get().unwrap().executor)(params, ctx)
}

struct Recorder {
    started: Instant,
    events: Mutex<Vec<Value>>,
}
impl EventEmitter for Recorder {
    fn emit(&self, event: NuphusEvent) {
        let ms = self.started.elapsed().as_millis();
        let row = match event {
            NuphusEvent::AssistantProgress {
                text, message_id, ..
            } => {
                println!("progress {ms}ms: {text}");
                json!({"kind":"progress","ms":ms,"message_id":message_id,"text":text})
            }
            NuphusEvent::ToolCallStart { tool_name, .. } => {
                json!({"kind":"tool","ms":ms,"tool":tool_name})
            }
            NuphusEvent::ExecutionCompleted { .. } => json!({"kind":"completed","ms":ms}),
            _ => return,
        };
        self.events.lock().unwrap().push(row);
    }
}

async fn run(
    root: PathBuf,
    llm: Arc<dyn nuphus::api::ApiClient>,
    factory: ClientFactory,
) -> Result<()> {
    let full = ToolRegistry::work_agent();
    let mut tools = ToolRegistry::new();
    for name in [
        "workflow_report_progress",
        "workflow_validate",
        "workflow_run",
        "skill_read",
        "Read",
        "Write",
    ] {
        let mut tool = full.get(name).context("missing production tool")?.clone();
        if name == "Write" {
            WRITE.set(tool.clone()).unwrap();
            tool.executor = fixture_write;
        } else if name == "Read" {
            READ.set(tool.clone()).unwrap();
            tool.executor = fixture_read;
        }
        tools.register(tool);
    }
    let mut engine = WorkflowEngine::new();
    engine.store = WorkflowStore::with_root(root.join("plugin/workflows"));
    engine.set_signals(tools.signals().clone());
    engine.set_tools(Arc::new(tools.clone()));
    engine.set_client_factory(factory);
    engine.set_llm_client(llm.clone());
    engine.init().await?;
    let engine = Arc::new(tokio::sync::RwLock::new(engine));
    let recorder = Arc::new(Recorder {
        started: Instant::now(),
        events: Mutex::new(vec![]),
    });
    let mut agent = WorkflowAgent::new(
        llm.clone(),
        tools,
        Some(recorder.clone()),
        None,
        llm.model_name().into(),
        "用户".into(),
        "Nuphus".into(),
        ToolPermissions {
            file_access: true,
            web_search: false,
            system_automation: false,
        },
        0.95,
    );
    agent.set_workflow_engine(engine.clone());
    let prompt = format!(
        "请创建并实际运行一个临时文件工作流：id=communication-smoke，使用 Write 工具将 Nuphus communication smoke 写入 {}，再用 Read 读回验证。工作流文件保存到 {}；可按需写同目录 params.json、guide.md 和上级 index.json，schema 位于 {}。只使用这些临时文件和现有工具，完成一次真实运行后结束，不操作其他应用或文件。",
        root.join("result.txt").display(), root.join("plugin/workflows/communication-smoke/workflow.json").display(), root.join("src/workflow/step_schema.json").display()
    );
    let cancel = AtomicBool::new(false);
    let result =
        tokio::time::timeout(Duration::from_secs(300), agent.run(&prompt, &None, &cancel)).await;
    cancel.store(true, Ordering::SeqCst);
    let success = result.ok().and_then(Result::ok).is_some_and(|o| o.success);
    let engine = engine.read().await;
    engine.store.load_all().await?;
    let workflow = engine.store.get("communication-smoke").await;
    let ran = workflow
        .as_ref()
        .and_then(|w| w.last_run())
        .is_some_and(|r| r.status == RunStatus::Success);
    let file_ok = std::fs::read_to_string(root.join("result.txt"))
        .is_ok_and(|s| s.trim() == "Nuphus communication smoke");
    let events = recorder.events.lock().unwrap();
    let first_progress = events.iter().position(|e| e["kind"] == "progress");
    let first_tool = events.iter().position(|e| e["kind"] == "tool");
    let count = events.iter().filter(|e| e["kind"] == "progress").count();
    let opening = matches!((first_progress,first_tool),(Some(p),Some(t)) if p<t);
    let pass = success && ran && file_ok && opening && count >= 2;
    let report = json!({"pass":pass,"agent_success":success,"workflow_ran":ran,"file_verified":file_ok,
        "opening_before_tools":opening,"progress_messages":count,"events":*events,
        "session_history_is_not_isolated":true});
    std::fs::write(
        root.join("communication-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("pass={pass} workflow_ran={ran} file_verified={file_ok} opening={opening} progress_messages={count}");
    ensure!(pass, "communication smoke did not meet acceptance criteria");
    Ok(())
}

fn main() -> Result<()> {
    if !std::env::args().any(|a| a == "--run") {
        println!("Opt-in: add --run. Uses configured deepseek-flash; synthetic session may append to application history. No desktop operations.");
        return Ok(());
    }
    let registry =
        config::load_registry().map_err(|_| anyhow::anyhow!("Cannot load model configuration"))?;
    let candidates = registry.find_model_candidates("deepseek-flash");
    ensure!(
        candidates.len() == 1,
        "deepseek-flash must have one unambiguous configured provider"
    );
    let provider = candidates[0].0.name.clone();
    let factory = ClientFactory::new(registry);
    let llm = factory
        .create_client_for(&provider, "deepseek-flash")
        .map_err(|_| anyhow::anyhow!("Cannot create configured model client"))?;
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let root = checkout
        .join("target/communication-smoke")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("plugin/workflows/communication-smoke"))?;
    let root = std::fs::canonicalize(root)?;
    #[cfg(windows)]
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
    for relative in [
        "src/workflow/step_schema.json",
        "plugin/skills/builtin/workflow-design/SKILL.md",
        "plugin/skills/builtin/workflow-design/skill.json",
    ] {
        let destination = root.join(relative);
        std::fs::create_dir_all(destination.parent().unwrap())?;
        std::fs::copy(checkout.join(relative), destination)?;
    }
    ROOT.set(root.clone()).unwrap();
    std::env::set_var("NUPHUS_WORKSPACE", &root);
    std::env::set_var("NUPHUS_PLUGIN_DIR", root.join("plugin"));
    std::env::set_var("NUPHUS_MEMORY_DIR", root.join("memory"));
    std::env::set_var("NUPHUS_DATA_DIR", root.join("data"));
    std::env::set_current_dir(&root)?;
    println!("artifacts={}", root.display());
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(root, llm, factory))
}
