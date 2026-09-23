//! Opt-in, real-product desktop smoke test; never runs as a unit test.
//!
//! cargo run -p nuphus --example desktop_workflow_smoke -- --run --mode enhanced
//! Add --config <providers.toml>, --mode normal, --timeout-secs 600 or
//! --output-root <directory> as needed. Default artifacts stay on the checkout drive.
//!
//! Uses WorkflowAgent -> work_agent tools -> WorkflowEngine, not a direct decision
//! API call. Opens only a newly created document in Notepad/TextEdit. Close unrelated
//! editor documents and stop other automation first. Ctrl+C cancels the run, but an
//! already dispatched native event may complete. No application is killed afterward.
//!
//! Workflow files, skill fixtures and memory files use an independent directory.
//! IMPORTANT: the public core database API has no path override; WorkflowAgent may
//! append this synthetic session to the normal Nuphus history database. This is not
//! an OS sandbox. The scope restriction is a task instruction, plus result checks.
//! No API keys, model text, desktop content or raw tool payloads are printed.

use anyhow::{bail, Context, Result};
use nuphus::{
    agent::events::{EventEmitter, NuphusEvent},
    automation_gate::{with_execution_owner, AutomationGate, HoldKind, ResourceClass},
    config::{self, ModelRegistry},
    llm::ClientFactory,
    permissions::ToolPermissions,
    runtime::WorkflowAgent,
    session::ContentBlock,
    tools::ToolRegistry,
    workflow::{store::WorkflowStore, types::RunStatus, WorkflowEngine},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct Progress {
    calls: Mutex<BTreeMap<String, usize>>,
    failed: Mutex<usize>,
}
impl EventEmitter for Progress {
    fn emit(&self, event: NuphusEvent) {
        if let NuphusEvent::ToolCallEnd {
            tool_name,
            success,
            duration_ms,
            ..
        } = event
        {
            *self
                .calls
                .lock()
                .unwrap()
                .entry(tool_name.clone())
                .or_default() += 1;
            if !success {
                *self.failed.lock().unwrap() += 1;
            }
            println!("tool={tool_name} success={success} duration_ms={duration_ms}");
        }
    }
}

struct Options {
    config: PathBuf,
    output_root: PathBuf,
    enhanced: bool,
    keyboard: bool,
    timeout: Duration,
}

fn options() -> Result<Option<Options>> {
    let mut args = std::env::args().skip(1);
    let mut run = false;
    let mut result = Options {
        config: dirs::config_dir()
            .context("找不到用户配置目录，请传 --config")?
            .join("nuphus/providers.toml"),
        output_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/desktop-workflow-smoke"),
        enhanced: true,
        keyboard: false,
        timeout: Duration::from_secs(600),
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => run = true,
            "--keyboard" => result.keyboard = true,
            "--mode" => {
                result.enhanced = match args.next().as_deref() {
                    Some("enhanced") => true,
                    Some("normal") => false,
                    _ => bail!("--mode 只能为 normal 或 enhanced"),
                };
            }
            "--config" => result.config = args.next().context("--config 缺少路径")?.into(),
            "--output-root" => {
                result.output_root = args.next().context("--output-root 缺少路径")?.into()
            }
            "--timeout-secs" => {
                let seconds: u64 = args
                    .next()
                    .context("--timeout-secs 缺少数值")?
                    .parse()
                    .context("超时必须为整数秒")?;
                if !(30..=1800).contains(&seconds) {
                    bail!("超时范围为 30–1800 秒");
                }
                result.timeout = Duration::from_secs(seconds);
            }
            "--help" | "-h" => {
                println!("--run --mode enhanced|normal [--keyboard] [--config providers.toml] [--output-root DIR] [--timeout-secs 600]");
                return Ok(None);
            }
            _ => bail!("未知参数；使用 --help 查看用法"),
        }
    }
    if !run {
        println!("默认不执行。请先停止其他自动化、关闭无关编辑器文档，再加 --run 显式启动真实桌面测试。测试可能在正常会话数据库追加合成记录。");
        return Ok(None);
    }
    if !cfg!(any(windows, target_os = "macos")) {
        bail!("此验收 example 暂只支持 Windows / macOS");
    }
    result.config = result.config.canonicalize().context("配置文件不存在")?;
    Ok(Some(result))
}

fn selected_model(registry: &ModelRegistry) -> Result<(String, Option<String>)> {
    let path = registry.source_path.as_ref().context("需要文件模型配置")?;
    let source = std::fs::read_to_string(path).context("无法读取模型配置")?;
    // Never propagate TOML diagnostics: they may include the credential source line.
    let doc: toml::Value = source
        .parse()
        .map_err(|_| anyhow::anyhow!("模型配置解析失败"))?;
    let workflow = doc
        .get("agent_models")
        .and_then(|v| v.get("workflow"))
        .and_then(toml::Value::as_str)
        .filter(|v| !v.is_empty());
    let Some(model) = workflow else {
        return Ok((registry.model.clone(), registry.last_model_provider_hint()));
    };
    let provider = doc
        .get("agent_models")
        .and_then(|v| v.get("workflow_provider"))
        .and_then(toml::Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| config::load_last_model_provider(path, model));
    Ok((model.to_owned(), provider))
}

fn copy_fixture(checkout: &Path, workspace: &Path, relative: &str) -> Result<()> {
    let destination = workspace.join(relative);
    std::fs::create_dir_all(destination.parent().context("无效 fixture 路径")?)?;
    std::fs::copy(checkout.join(relative), destination).context("无法复制工作流开发 fixture")?;
    Ok(())
}

fn open_document(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let mut command = std::process::Command::new("notepad.exe");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.args(["-a", "TextEdit"]);
        command
    };
    #[cfg(any(windows, target_os = "macos"))]
    {
        // This window is intentionally visible: the user is accepting a real UI test.
        command
            .arg(path)
            .spawn()
            .context("无法打开临时编辑器文档")?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        bail!("不支持此平台")
    }
}

fn decision_evidence(agent: &WorkflowAgent) -> (usize, usize) {
    let mut decision_calls = HashSet::new();
    let mut responses = 0;
    let mut handoffs = 0;
    for message in agent.session().messages() {
        for block in &message.content {
            match block {
                ContentBlock::ToolUse { id, name, .. } if name == "desktop_agent_step" => {
                    decision_calls.insert(id.clone());
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error: false,
                } if decision_calls.contains(tool_use_id) => {
                    if let Ok(value) = serde_json::from_str::<Value>(content) {
                        if value["decision"]["provider"] == "jev" {
                            responses += 1;
                        }
                        if value["status"] == "needs_primary_decision" {
                            handoffs += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (responses, handoffs)
}

fn workflow_evidence(value: &Value) -> (bool, bool) {
    match value {
        Value::Object(object) => {
            let tool = object.get("tool").and_then(Value::as_str).unwrap_or("");
            let mut desktop_action = matches!(
                tool,
                "desktop_semantic_action"
                    | "desktop_semantic_execute"
                    | "desktop_input"
                    | "desktop_mouse"
                    | "desktop_agent_step"
            );
            let mut bypass = object.contains_key("script")
                || matches!(tool, "Bash" | "Shell" | "Write" | "Edit" | "system_shell")
                || tool.starts_with("browser_");
            for nested in object.values() {
                let (action, forbidden) = workflow_evidence(nested);
                desktop_action |= action;
                bypass |= forbidden;
            }
            (desktop_action, bypass)
        }
        Value::Array(array) => array
            .iter()
            .map(workflow_evidence)
            .fold((false, false), |(a, b), (c, d)| (a || c, b || d)),
        _ => (false, false),
    }
}

fn direct_document_write(agent: &WorkflowAgent, filename: &str) -> bool {
    agent
        .session()
        .messages()
        .iter()
        .flat_map(|message| &message.content)
        .any(|block| {
            matches!(block, ContentBlock::ToolUse { name, input, .. }
            if matches!(name.as_str(), "Bash" | "Shell" | "system_shell")
            || (matches!(name.as_str(), "Write" | "Edit")
                && ["path", "file_path", "filename"].iter().any(|key|
                    input[*key].as_str().is_some_and(|path| path.ends_with(filename)))))
        })
}

fn static_window_handle(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "hwnd" && !value.as_str().is_some_and(|v| v.starts_with("{{")))
                || static_window_handle(value)
        }),
        Value::Array(array) => array.iter().any(static_window_handle),
        _ => false,
    }
}

async fn run(options: Options, workspace: PathBuf, registry: ModelRegistry) -> Result<()> {
    let (model, mut provider) = selected_model(&registry)?;
    if provider.is_none() {
        let candidates = registry.find_model_candidates(&model);
        if candidates.len() != 1 {
            bail!("工作流模型提供商不唯一或未配置，请先在应用内选择模型");
        }
        provider = Some(candidates[0].0.name.clone());
    }
    let factory = ClientFactory::new(registry);
    let llm = factory
        .create_client_for(provider.as_deref().unwrap(), &model)
        .map_err(|_| anyhow::anyhow!("无法创建已配置的工作流主模型客户端"))?;
    let mut tools = ToolRegistry::work_agent();
    tools.set_enhanced_mode(options.enhanced);
    let exposed = tools
        .get_schemas()
        .iter()
        .any(|s| s.function.name == "desktop_agent_step");
    anyhow::ensure!(exposed == options.enhanced, "增强模式工具暴露状态不一致");
    let gate = Arc::new(AutomationGate::new());
    tools.set_automation_gate(gate.clone());
    let mut engine = WorkflowEngine::new();
    engine.store = WorkflowStore::with_root(workspace.join("plugin/workflows"));
    engine.set_signals(tools.signals().clone());
    engine.set_client_factory(factory);
    engine.set_llm_client(llm.clone());
    engine.set_tools(Arc::new(tools.clone()));
    engine.init().await?;
    let engine = Arc::new(tokio::sync::RwLock::new(engine));
    let progress = Arc::new(Progress::default());
    let mut agent = WorkflowAgent::new(
        llm,
        tools,
        Some(progress.clone()),
        None,
        model,
        "用户".into(),
        "Nuphus".into(),
        ToolPermissions {
            file_access: true,
            web_search: false,
            system_automation: true,
        },
        0.95,
    );
    agent.set_workflow_engine(engine.clone());
    agent.set_enhanced_mode(options.enhanced);
    anyhow::ensure!(
        agent.enhanced_mode() == options.enhanced,
        "Agent 增强状态不一致"
    );

    let filename = format!("nuphus-smoke-{}.txt", uuid::Uuid::new_v4());
    let document = workspace.join(&filename);
    let expected = format!("Nuphus desktop workflow smoke {}", uuid::Uuid::new_v4());
    std::fs::write(&document, "Temporary Nuphus smoke document.\n")?;
    println!("即将打开临时文档并由真实 WorkflowAgent 开发、运行工作流；请勿同时操作鼠标键盘。mode={} artifacts={}", if options.enhanced { "enhanced" } else { "normal" }, workspace.display());
    open_document(&document)?;
    let prompt = format!(
        "进行一次真实桌面工作流验收。只操作刚打开的临时文本文档 {document}（窗口标题包含 {filename}），不能编辑或关闭其他文档、不能操作其他应用。\n\
         请用当前已配置的主模型完成工作流开发，不要只口头给方案。{mode}\n\
         {input_method}\n\
         目标：通过本地编辑器的真实桌面动作，把这个文档的全部内容替换为下面的一行测试文本并保存：\n{expected}\n\
         从应用/窗口发现与绑定开始，优先使用语义候选，必要时沿用产品现有鼠标/键盘能力。不要启动浏览器、不要发送消息、不要执行脚本/命令行；禁止使用 Write/Edit 等文件工具直接修改该 txt 来冒充桌面操作。文件写入仅用于本次工作流定义和参数、说明文档。\n\
         已有本地工作流开发资料位于 plugin/skills/builtin/workflow-design/SKILL.md 与 src/workflow/step_schema.json。\n\
         保存一个真实可重放工作流到 {store}/desktop-smoke/workflow.json，id 使用 desktop-smoke。固化本次成功返回的 workflow_step，幂等替换文本而非反复追加；键盘保存步骤使用 target_locator，不保存临时 hwnd。设置 timeout_secs 为 120，不创建定时任务。必须调用 workflow_validate，随后调用 workflow_run 实际执行一次；检查保存后的文档结果后立即结束，不清理文件、不关闭窗口、不额外检索项目源码。若阻塞，请报告真实原因，不要猜测成功。",
        document = document.display(),
        store = workspace.join("plugin/workflows").display(),
        mode = if options.enhanced { "增强模式已经开启：开发过程中必须实际使用 desktop_agent_step；若返回需要主模型接手或提供文本，再沿正常产品链路继续。" } else { "本轮为普通模式：由主模型使用 desktop_semantic_observe 与 desktop_semantic_execute，不能调用增强判断入口。" },
        input_method = if options.keyboard { "本轮回归测试键盘输入路径：语义定位编辑区并聚焦后，用 desktop_input 全选、输入和保存，不用 SetValue。所有键盘步骤复用同一份成功 locator，不手工添加已修改/未修改标题变体；本地应保留同次执行窗口绑定。" } else { "输入方式沿用本地可用能力。" },
    );
    let cancel = AtomicBool::new(false);
    let owner = format!("workflow-smoke:{}", uuid::Uuid::new_v4());
    let _lease = gate
        .try_acquire(
            ResourceClass::ExecutionBody,
            HoldKind::ExecutionBody,
            owner.clone(),
        )
        .map_err(|_| anyhow::anyhow!("自动化资源忙"))?;
    let output = {
        let images = None;
        let run = with_execution_owner(owner, agent.run(&prompt, &images, &cancel));
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => result.ok(),
            _ = tokio::time::sleep(options.timeout) => { cancel.store(true, Ordering::SeqCst); None },
            _ = tokio::signal::ctrl_c() => { cancel.store(true, Ordering::SeqCst); None },
        }
    };
    let engine = engine.read().await;
    if let Some(active) = engine.active_run_info() {
        engine.cancel_workflow(&active.workflow_id).await;
    }
    engine.store.load_all().await?;
    let workflow = engine.store.get("desktop-smoke").await;
    let run_success = workflow
        .as_ref()
        .and_then(|wf| wf.last_run())
        .is_some_and(|record| record.status == RunStatus::Success && !record.steps.is_empty());
    let (replays_desktop_action, workflow_bypass) = workflow
        .as_ref()
        .and_then(|wf| serde_json::to_value(&wf.steps).ok())
        .as_ref()
        .map(workflow_evidence)
        .unwrap_or((false, false));
    let direct_write = direct_document_write(&agent, &filename);
    let static_handle = workflow
        .as_ref()
        .and_then(|wf| serde_json::to_value(&wf.steps).ok())
        .as_ref()
        .is_some_and(static_window_handle);
    let text = std::fs::read_to_string(&document).unwrap_or_default();
    let file_matches = text.trim_start_matches('\u{feff}').trim() == expected;
    let (decision_responses, primary_handoffs) = decision_evidence(&agent);
    let agent_success = output.as_ref().is_some_and(|o| o.success);
    let pass = agent_success
        && run_success
        && file_matches
        && replays_desktop_action
        && !workflow_bypass
        && !direct_write
        && !static_handle
        && (!options.enhanced || decision_responses > 0);
    let report = json!({
        "mode": if options.enhanced { "enhanced" } else { "normal" },
        "keyboard_regression": options.keyboard,
        "passed": pass, "agent_success": agent_success,
        "workflow_run_success": run_success, "document_matches": file_matches,
        "replays_desktop_action": replays_desktop_action,
        "workflow_contains_bypass": workflow_bypass, "direct_document_write_or_shell": direct_write,
        "contains_static_window_handle": static_handle,
        "enhanced_decision_responses": decision_responses, "primary_handoffs": primary_handoffs,
        "cancelled_or_timed_out": cancel.load(Ordering::SeqCst),
        "tool_calls": *progress.calls.lock().unwrap(), "failed_tool_calls": *progress.failed.lock().unwrap(),
        "session_history_is_not_isolated": true,
    });
    let report = serde_json::to_string_pretty(&report)?;
    std::fs::write(workspace.join("smoke-report.json"), &report)?;
    println!("{report}");
    anyhow::ensure!(pass, "未通过验收；临时文档和工作流已保留，查看 smoke-report.json。不会把普通模型回退冒充增强模型成功。");
    Ok(())
}

fn main() -> Result<()> {
    let Some(options) = options()? else {
        return Ok(());
    };
    config::set_config_override(options.config.clone());
    let registry = config::load_registry().map_err(|_| anyhow::anyhow!("加载现有模型配置失败"))?;
    if options.enhanced && registry.jev.api_key.trim().is_empty() {
        bail!("增强判断模型没有可用密钥；请在应用内配置后再进行增强验收");
    }
    let workspace = options.output_root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&workspace)?;
    let workspace = workspace.canonicalize()?;
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for relative in [
        "src/workflow/step_schema.json",
        "plugin/skills/builtin/workflow-design/SKILL.md",
        "plugin/skills/builtin/workflow-design/skill.json",
    ] {
        copy_fixture(&checkout, &workspace, relative)?;
    }
    // Set only task-specific overrides, before starting runtime worker threads.
    std::env::set_var("NUPHUS_WORKSPACE", &workspace);
    std::env::set_var("NUPHUS_PLUGIN_DIR", workspace.join("plugin"));
    std::env::set_var("NUPHUS_MEMORY_DIR", workspace.join("memory"));
    std::env::set_var("NUPHUS_DATA_DIR", workspace.join("data"));
    std::env::set_current_dir(&workspace)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(options, workspace, registry));
    runtime.shutdown_timeout(Duration::from_secs(5));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_rejects_saved_handles_but_accepts_runtime_capture_and_stable_locator() {
        assert!(static_window_handle(&json!({"with": {"hwnd": 123}})));
        assert!(static_window_handle(
            &json!({"with": {"hwnd": "{params.window.hwnd}"}})
        ));
        assert!(!static_window_handle(
            &json!({"with": {"hwnd": "{{current_window.hwnd}}"}})
        ));
        assert!(!static_window_handle(
            &json!({"with": {"target_locator": "{params.locator}"}})
        ));
    }

    #[test]
    fn smoke_rejects_file_or_script_bypass_inside_nested_workflow() {
        let (desktop, bypass) = workflow_evidence(&json!([{"do": {"seq": [
            {"do": {"tool": "desktop_input"}},
            {"do": {"script": {"runtime": "python", "code": "..."}}}
        ]}}]));
        assert!(desktop && bypass);
        assert_eq!(
            workflow_evidence(&json!({"do": {"tool": "desktop_semantic_action"}})),
            (true, false)
        );
    }
}
