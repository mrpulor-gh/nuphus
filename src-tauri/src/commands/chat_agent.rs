//! chat_agent — Tauri Chat Agent CRUD 命令
//!
//! 对应 nuphus::workflow::chat_agent::ChatAgentStore 的静态方法。
//! 附加工 workflows 中 Action::Chat 步骤的内联配置查询/更新。

use nuphus::workflow::chat_agent::{ChatAgentConfig, ChatAgentStore};
use nuphus::workflow::types::{Action, ChatOpts, Step};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

// ── Inline types ──

/// 内联 ChatAgent 步骤配置（写入 ChatOpts；模型参数 + Agent 行为参数）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatAgentInlineConfig {
    // ── 模型参数 ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    // ── Agent 行为参数（对齐 ChatAgentConfig）──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
}

/// 工作流内联 ChatAgent 条目（带归属信息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineChatAgentEntry {
    pub workflow_id: String,
    pub workflow_name: String,
    pub step_id: String,
    pub step_name: String,
    pub config: ChatAgentInlineConfig,
}

// ── Helpers ──

fn chat_opts_to_inline_config(opts: &ChatOpts) -> ChatAgentInlineConfig {
    ChatAgentInlineConfig {
        model: opts.model.clone(),
        model_display: opts.model_display.clone(),
        temperature: opts.temperature,
        max_tokens: opts.max_tokens,
        system_prompt: opts.system_prompt.clone(),
        agent_id: opts.agent_id.clone(),
        persona: opts.persona.clone(),
        goal: opts.goal.clone(),
        constraints: opts.constraints.clone(),
        requirements: opts.requirements.clone(),
        knowledge: opts.knowledge.clone(),
        max_iterations: opts.max_iterations,
    }
}

fn apply_inline_config_to_opts(opts: &mut ChatOpts, config: &ChatAgentInlineConfig) {
    opts.model = config.model.clone();
    opts.model_display = config.model_display.clone();
    opts.temperature = config.temperature;
    opts.max_tokens = config.max_tokens;
    opts.system_prompt = config.system_prompt.clone();
    opts.agent_id = config.agent_id.clone();
    opts.persona = config.persona.clone();
    opts.goal = config.goal.clone();
    opts.constraints = config.constraints.clone();
    opts.requirements = config.requirements.clone();
    opts.knowledge = config.knowledge.clone();
    opts.max_iterations = config.max_iterations;
}

/// 递归遍历步骤树，对每个 Action::Chat 步骤调用 visit
fn visit_chat_steps(steps: &[Step], visit: &mut impl FnMut(&Step, &ChatOpts)) {
    for step in steps {
        match &step.action {
            Action::Chat { with, .. } => {
                visit(step, with);
            }
            Action::Seq { seq } => {
                visit_chat_steps(seq, visit);
            }
            Action::Loop { def } => {
                visit_chat_steps(&def.steps, visit);
            }
            Action::If { def } => {
                visit_chat_steps(&def.then, visit);
                visit_chat_steps(&def.else_branch, visit);
            }
            Action::Wait { auto, .. } => {
                visit_chat_steps(auto, visit);
            }
            _ => {}
        }
    }
}

/// 递归遍历步骤树，对每个 Action::Chat 步骤调用 visit_mut（传递 step_id + 可变借用 ChatOpts）
fn visit_chat_steps_mut(steps: &mut [Step], visit: &mut impl FnMut(&str, &mut ChatOpts)) {
    for step in steps {
        let sid = &step.id;
        match &mut step.action {
            Action::Chat { with, .. } => {
                visit(sid, with);
            }
            Action::Seq { seq } => {
                visit_chat_steps_mut(seq, visit);
            }
            Action::Loop { def } => {
                visit_chat_steps_mut(&mut def.steps, visit);
            }
            Action::If { def } => {
                visit_chat_steps_mut(&mut def.then, visit);
                visit_chat_steps_mut(&mut def.else_branch, visit);
            }
            Action::Wait { auto, .. } => {
                visit_chat_steps_mut(auto, visit);
            }
            _ => {}
        }
    }
}

// ── 全局 ChatAgent CRUD ──

#[tauri::command]
pub fn chat_agent_list() -> Result<Vec<ChatAgentConfig>, String> {
    Ok(ChatAgentStore::list())
}

#[tauri::command]
pub fn chat_agent_save(config: ChatAgentConfig) -> Result<ChatAgentConfig, String> {
    ChatAgentStore::save(&config)
}

#[tauri::command]
pub fn chat_agent_delete(name: String) -> Result<(), String> {
    ChatAgentStore::delete(&name)
}

#[tauri::command]
pub fn chat_agent_set_active(name: String) -> Result<ChatAgentConfig, String> {
    ChatAgentStore::set_active(&name)
}

#[tauri::command]
pub fn chat_agent_get_active() -> Result<Option<ChatAgentConfig>, String> {
    Ok(ChatAgentStore::get_active())
}

// ── 内联 ChatAgent（workflow 内的 Action::Chat 步骤配置）──

/// 查询某 workflow 中所有 Action::Chat 步骤的内联配置
#[tauri::command]
pub async fn chat_agent_list_inline(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<Vec<InlineChatAgentEntry>, String> {
    let engine = state.workflow_engine.read().await;
    let wf = engine
        .get_workflow(&workflow_id)
        .await
        .ok_or_else(|| format!("Workflow not found: {workflow_id}"))?;

    let mut entries = Vec::new();
    let wf_name = wf.name.clone();
    let wf_id = wf.id.clone();

    visit_chat_steps(&wf.steps, &mut |step, opts| {
        entries.push(InlineChatAgentEntry {
            workflow_id: wf_id.clone(),
            workflow_name: wf_name.clone(),
            step_id: step.id.clone(),
            step_name: step.name.clone(),
            config: chat_opts_to_inline_config(opts),
        });
    });

    Ok(entries)
}

/// 更新某 workflow 中指定 Action::Chat 步骤的内联配置
#[tauri::command]
pub async fn chat_agent_update_inline(
    state: State<'_, AppState>,
    workflow_id: String,
    step_id: String,
    config: ChatAgentInlineConfig,
) -> Result<(), String> {
    let engine = state.workflow_engine.read().await;
    let mut wf = engine
        .get_workflow(&workflow_id)
        .await
        .ok_or_else(|| format!("Workflow not found: {workflow_id}"))?;

    let mut found = false;
    let step_id_ref = &step_id;
    visit_chat_steps_mut(&mut wf.steps, &mut |sid, opts| {
        if sid == step_id_ref {
            apply_inline_config_to_opts(opts, &config);
            found = true;
        }
    });

    if !found {
        return Err(format!("Chat step not found: {step_id}"));
    }

    // 通过 store 直接保存（engine 无公开的 save_workflow，走 store）
    engine.store.save(&wf).await.map_err(|e| e.to_string())
}
