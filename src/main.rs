//! Nuphus CLI — 入口（基于 ExecuteAgent）
//!
//! 用法:
//!   nuphus [任务描述]          — 单次执行模式（默认）
//!   nuphus --help              — 显示帮助

use nuphus::{config::load_registry, llm::ClientFactory, SubTaskRunner, ToolRegistry};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    nuphus::utils::init_logging();

    let args: Vec<String> = std::env::args().collect();

    // 解析 --help
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Nuphus CLI");
        println!();
        println!("用法:");
        println!("  nuphus [任务描述]              单次执行模式");
        println!("  nuphus --help                   显示此帮助");
        return;
    }

    run_single(args).await;
}

/// 单次执行模式（原有逻辑，不变）
async fn run_single(args: Vec<String>) {
    println!("Nuphus - 协同共生桌面助手\n");
    println!("========================\n");

    // 1. 加载模型配置
    let registry = load_registry()
        .expect("Failed to load model config (create config.toml or set API_KEY env vars)");
    let factory = ClientFactory::new(registry);

    // 2. 创建默认 LLM Client
    let llm = factory.create_main_client()
        .expect("Failed to create default LLM client");

    // 3. 初始化 Tools
    let tools = ToolRegistry::builtin();

    // 4. 读取任务
    let task = args.get(1)
        .cloned()
        .unwrap_or_else(|| "你好，介绍你自己".to_string());

    println!("任务: {}\n", task);

    use nuphus::agent::prompt;
    let tool_schemas = tools.render_tools_for_prompt();

    // 5. 构建纯静态 system prompt
    let system_prompt = prompt::build_exec_prompt(
        "cli",
        &tool_schemas,
        nuphus::agent::goal_types::GoalType::ScriptingExec,
        "", None,
        false, None,
    );

    // 6. 创建 SubTaskRunner，task 通过 user message 传入
    let mut agent = SubTaskRunner::new_free(
        llm.clone(),
        tools,
        system_prompt,
        task.clone(),
    );
    agent.session.push_user(task.clone());
    agent.set_context_window(nuphus::agent::goal_types::get_context_window(llm.model_name()));

    println!("\n执行中...\n");

    // 7. 执行
    let cancel_flag = AtomicBool::new(false);
    match agent.run_free(&cancel_flag).await {
        Ok((true, message, _)) => {
            println!("结果: {}", message);
        }
        Ok((false, message, _)) => {
            eprintln!("执行失败: {}", message);
        }
        Err(e) => {
            eprintln!("执行出错: {}", e);
        }
    }
}