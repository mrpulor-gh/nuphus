//! compiler.rs — 工作流编译器
//!
//! 职责：执行前静态验证，把设计错误拦截在运行时之前。
//!
//! 验证项：
//! - 步骤非空、步骤 ID 全局唯一（断点续连依赖 ID 跳过已完成步骤）
//! - Tool 步骤：工具名非空、对注册表校验（提供工具表时）、params 非 null、必填参数齐全
//! - If / Until 条件：var/value 非空、regex 可编译、条件变量已被捕获（warning）
//! - Loop：for_each 变量名非空、items_var 已被捕获（warning，运行时缺失将静默空循环）
//! - Call：workflow_id 非空；目标存在性 + 循环调用链见 validate_calls（异步）
//! - Script：code 非空、runtime 白名单
//! - {{var}} 前向引用检查（warning 级：变量可能由运行时 inputs/params.json 注入）
//!
//! 编译产出：ValidationReport（errors 阻断执行，warnings 仅提示）

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::{Action, Condition, Step, Workflow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 编译报告（仅验证，不修改数据）
/// Serialize：供 wf_validate / wf_save 命令回传前端（画布 ProblemsPanel 消费）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub passed: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// 编译器（无状态，仅提供静态方法）
pub struct Compiler;

/// 校验上下文（递归遍历时携带）
struct Ctx<'a> {
    errors: Vec<String>,
    warnings: Vec<String>,
    /// 已出现的步骤 ID（查重）
    seen_ids: HashSet<String>,
    /// 按遍历顺序已被捕获的变量名
    captured: HashSet<String>,
    /// 工具注册表（None = 跳过工具名校验）
    tools: Option<&'a [crate::api::ToolDefinition]>,
    /// 模型注册表（None = 跳过 chat with.model 存在性校验；registry 未配置/为空时降级）
    models: Option<&'a crate::config::ModelRegistry>,
    /// 是否在 loop 内部（用于 break/continue 检查）
    in_loop: bool,
}

impl Ctx<'_> {
    /// 登记步骤 ID，重复即错误（断点续连按 ID 跳过，重复 ID 会误跳未执行步骤）
    fn register_id(&mut self, step: &Step) {
        let id = step.id();
        if id.is_empty() {
            self.errors.push(format!("步骤 '{}': id 为空", step.name()));
            return;
        }
        if !self.seen_ids.insert(id.clone()) {
            self.errors.push(format!("重复的步骤 ID: '{}'", id));
        }
    }

    /// 校验条件（V2 Condition 格式）
    fn validate_condition(&mut self, cond: &Condition, owner: &str) {
        match cond {
            Condition::Regex { regex } => {
                if !regex.is_empty() {
                    let pattern = resolve_var_ref_to_lit(&regex[0]);
                    if !pattern.is_empty() && regex::Regex::new(&pattern).is_err() {
                        self.errors.push(format!(
                            "步骤 '{}': regex 模式 '{}' 编译失败",
                            owner, pattern
                        ));
                    }
                }
                // 检查 VarRef 中的变量引用
                for r in regex {
                    check_var_ref(r, &self.captured, owner, &mut self.warnings);
                }
            }
            Condition::NotEmpty { not_empty } => {
                check_var_ref(not_empty, &self.captured, owner, &mut self.warnings);
                // NotEmpty: checks that var is non-empty — always valid at compile time
            }
            Condition::Empty { empty } => {
                check_var_ref(empty, &self.captured, owner, &mut self.warnings);
                // Empty: checks that var is empty — always valid at compile time
            }
            _ => {
                // Equals, NotEquals, Contains, StartsWith, Gt, Lt, Gte, Lte
                // All take a Vec<VarRef>
                let refs: Option<&Vec<crate::workflow::types::VarRef>> = match cond {
                    Condition::Equals { equals } => Some(equals),
                    Condition::NotEquals { not_equals } => Some(not_equals),
                    Condition::Contains { contains } => Some(contains),
                    Condition::StartsWith { starts_with } => Some(starts_with),
                    Condition::Gt { gt } => Some(gt),
                    Condition::Lt { lt } => Some(lt),
                    Condition::Gte { gte } => Some(gte),
                    Condition::Lte { lte } => Some(lte),
                    _ => None,
                };
                if let Some(refs) = refs {
                    for r in refs {
                        check_var_ref(r, &self.captured, owner, &mut self.warnings);
                    }
                }
            }
        }
    }

    /// 扫描 JSON 中的 {{var}} 引用，检查是否已被先前步骤捕获（warning 级）
    fn scan_refs(&mut self, v: &serde_json::Value, owner: &str) {
        static VAR_REF_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        match v {
            serde_json::Value::String(s) => {
                if !s.contains("{{") {
                    return;
                }
                let re = VAR_REF_RE.get_or_init(|| {
                    regex::Regex::new(r"\{\{\s*([A-Za-z_]\w*)")
                        .expect("var ref regex is statically valid")
                });
                let found: Vec<String> = re
                    .captures_iter(s)
                    .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
                    .filter(|name| {
                        !self.captured.contains(name)
                            && name != "params"
                            && !name.starts_with("ENV:")
                    })
                    .collect();
                for name in found {
                    self.warnings.push(format!(
                        "步骤 '{}': 引用变量 '{}' 尚未被先前步骤捕获（运行时可能由 inputs 注入）",
                        owner, name
                    ));
                }
            }
            serde_json::Value::Object(m) => {
                for val in m.values() {
                    self.scan_refs(val, owner);
                }
            }
            serde_json::Value::Array(a) => {
                for val in a {
                    self.scan_refs(val, owner);
                }
            }
            _ => {}
        }
    }
}

/// 检查 VarRef 中的变量引用是否已被捕获
fn check_var_ref(
    r: &crate::workflow::types::VarRef,
    captured: &HashSet<String>,
    owner: &str,
    warnings: &mut Vec<String>,
) {
    if let crate::workflow::types::VarRef::Var { var } = r {
        if !captured.contains(var) && var != "_index" && !var.starts_with("ENV:") {
            warnings.push(format!(
                "条件步骤 '{}': 变量 '{}' 尚未被先前步骤捕获，求值将为 false（运行时可能由 inputs 注入）",
                owner, var
            ));
        }
    }
}

/// 从 VarRef 提取字面量（用于 regex 校验等）
fn resolve_var_ref_to_lit(r: &crate::workflow::types::VarRef) -> String {
    match r {
        crate::workflow::types::VarRef::Lit(s) => s.clone(),
        crate::workflow::types::VarRef::Var { .. } => String::new(),
    }
}

impl Compiler {
    /// 基础校验（不包含工具注册表校验）
    pub fn validate_workflow(workflow: &Workflow) -> ValidationReport {
        Self::validate_workflow_with_tools(workflow, &[])
    }

    /// 完整校验（含工具注册表校验）
    pub fn validate_workflow_with_tools(
        workflow: &Workflow,
        tools: &[crate::api::ToolDefinition],
    ) -> ValidationReport {
        // chat with.model 存在性校验用的模型注册表：加载失败或无 provider 时降级跳过
        // （裸模型名 fallback 是合法语义，校验仅作提示，不阻断）
        let registry = crate::config::load_registry()
            .ok()
            .filter(|r| !r.providers.is_empty());
        let mut ctx = Ctx {
            errors: Vec::new(),
            warnings: Vec::new(),
            seen_ids: HashSet::new(),
            captured: HashSet::new(),
            tools: if tools.is_empty() { None } else { Some(tools) },
            models: registry.as_ref(),
            in_loop: false,
        };

        if workflow.steps.is_empty() {
            ctx.warnings.push("工作流没有任何步骤".to_string());
            return ValidationReport {
                passed: true,
                warnings: ctx.warnings,
                errors: ctx.errors,
            };
        }

        for step in &workflow.steps {
            Self::validate_step(step, &mut ctx);
        }

        ValidationReport {
            passed: ctx.errors.is_empty(),
            warnings: ctx.warnings,
            errors: ctx.errors,
        }
    }

    fn validate_step(step: &Step, ctx: &mut Ctx) {
        ctx.register_id(step);

        if step.name.is_empty() {
            ctx.errors
                .push(format!("步骤 '{}': name 不能为空", step.id()));
            return;
        }

        match &step.action {
            Action::Tool { tool, with } => {
                if tool.is_empty() {
                    ctx.errors
                        .push(format!("Tool step '{}': tool is empty", step.name));
                }
                if with.is_null() {
                    ctx.errors
                        .push(format!("Tool step '{}': params 不能为 null", step.name));
                } else if let Some(tools) = ctx.tools {
                    match tools.iter().find(|t| t.function.name == *tool) {
                        None => ctx.errors.push(format!(
                            "Tool step '{}': 工具 '{}' 不在注册表中（拼写错误？）",
                            step.name, tool
                        )),
                        Some(def) => {
                            if let Some(required) = def
                                .function
                                .parameters
                                .get("required")
                                .and_then(|r| r.as_array())
                            {
                                match with.as_object() {
                                    Some(obj) => {
                                        for r in required {
                                            if let Some(key) = r.as_str() {
                                                if !obj.contains_key(key) {
                                                    ctx.errors.push(format!(
                                                        "Tool step '{}' ({}): 缺少必填参数 '{}'",
                                                        step.name, tool, key
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        if !required.is_empty() {
                                            ctx.errors.push(format!(
                                                "Tool step '{}' ({}): params 必须是对象（需要 {:?}）",
                                                step.name, tool, required
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                ctx.scan_refs(with, &step.name);
                if let Some(cap) = &step.capture {
                    ctx.captured.insert(cap.clone());
                }
            }
            Action::Seq { seq } => {
                if seq.is_empty() {
                    ctx.warnings.push(format!(
                        "Seq step '{}' ({}): 无子步骤",
                        step.name,
                        step.id()
                    ));
                }
                for sub in seq {
                    Self::validate_step(sub, ctx);
                }
            }
            Action::Loop { def } => {
                if def.for_each.is_none() && def.repeat.is_none() && def.until.is_none() {
                    ctx.errors.push(format!(
                        "Loop step '{}' ({}): 缺少 for_each / repeat / until",
                        step.name,
                        step.id()
                    ));
                }
                if let Some(ref fe) = def.for_each {
                    if fe.item_var.is_empty() {
                        ctx.errors.push(format!(
                            "Loop step '{}': for_each 的 item_var 不能为空",
                            step.name
                        ));
                    }
                    check_var_ref(&fe.items, &ctx.captured, &step.name, &mut ctx.warnings);
                }
                if let Some(ref until) = def.until {
                    ctx.validate_condition(until, &step.name);
                }
                let was_loop = ctx.in_loop;
                ctx.in_loop = true;
                for sub in &def.steps {
                    Self::validate_step(sub, ctx);
                }
                ctx.in_loop = was_loop;
            }
            Action::If { def } => {
                ctx.validate_condition(&def.condition, &step.name);
                for sub in &def.then {
                    Self::validate_step(sub, ctx);
                }
                for sub in &def.else_branch {
                    Self::validate_step(sub, ctx);
                }
            }
            Action::Call { call, with } => {
                if call.is_empty() {
                    ctx.errors.push(format!(
                        "Call step '{}' ({}): workflow_id 不能为空",
                        step.name,
                        step.id()
                    ));
                }
                ctx.scan_refs(with, &step.name);
            }
            Action::Wait { wait, auto } => {
                if wait.is_empty() && auto.is_empty() {
                    ctx.warnings.push(format!(
                        "Wait step '{}': prompt 和 auto 均为空，将立即通过",
                        step.name
                    ));
                }
                for sub in auto {
                    Self::validate_step(sub, ctx);
                }
            }
            Action::Chat { chat, with: opts } => {
                if chat.is_empty() {
                    ctx.errors
                        .push(format!("Chat step '{}': message 不能为空", step.name));
                }
                // with.model 优先按 registry 模型 ID 路由；不在 registry 时回退裸模型名（warning 提示）
                if let (Some(model_id), Some(registry)) = (&opts.model, ctx.models) {
                    if registry.find_model(model_id).is_none() {
                        ctx.warnings.push(format!(
                            "Chat step '{}': 模型 '{}' 不在 registry 中，执行时将回退为主模型客户端（裸模型名）",
                            step.name, model_id
                        ));
                    }
                }
                if let Some(ref knowledge) = opts.knowledge {
                    for path in knowledge {
                        if !std::path::Path::new(path).exists() {
                            ctx.warnings.push(format!(
                                "Chat step '{}': 知识库文件不存在: {}",
                                step.name, path
                            ));
                        }
                    }
                }
            }
            Action::Script { script } => {
                if script.code.is_empty() {
                    ctx.errors
                        .push(format!("Script step '{}': code 不能为空", step.name));
                }
                const VALID_RUNTIMES: &[&str] = &["python", "node", "ahk", "pwsh"];
                if !VALID_RUNTIMES.contains(&script.runtime.as_str()) {
                    ctx.errors.push(format!(
                        "Script step '{}': 不支持的 runtime '{}'（支持: {:?}）",
                        step.name, script.runtime, VALID_RUNTIMES
                    ));
                }
            }
            Action::Assert { assert } => {
                ctx.validate_condition(&assert.condition, &step.name);
            }
            Action::Mcp { mcp } => {
                if mcp.server.is_empty() {
                    ctx.errors
                        .push(format!("Mcp step '{}': server 不能为空", step.name));
                }
                if mcp.tool.is_empty() {
                    ctx.errors
                        .push(format!("Mcp step '{}': tool 不能为空", step.name));
                }
            }
            Action::Sleep { sleep } => {
                if *sleep <= 0.0 {
                    ctx.errors.push(format!(
                        "Sleep step '{}': sleep 必须 > 0 (got {})",
                        step.name, sleep
                    ));
                }
            }
            Action::Break { .. } | Action::Continue { .. } => {
                if !ctx.in_loop {
                    ctx.errors.push(format!(
                        "步骤 '{}': break/continue 只能在 loop 内部使用",
                        step.name
                    ));
                }
            }
            Action::Custom(_) => {
                ctx.warnings
                    .push(format!("步骤 '{}': custom 类型，跳过类型校验", step.name));
            }
        }
    }

    /// Claim: chained call detection → static deadlock prevention
    pub async fn validate_calls(workflow: &Workflow, store: &WorkflowStore) -> Vec<String> {
        fn collect_calls(steps: &[Step], calls: &mut HashSet<String>) {
            for step in steps {
                match &step.action {
                    Action::Call { call, .. } => {
                        calls.insert(call.clone());
                    }
                    Action::Seq { seq } => collect_calls(seq, calls),
                    Action::Loop { def } => collect_calls(&def.steps, calls),
                    Action::If { def } => {
                        collect_calls(&def.then, calls);
                        collect_calls(&def.else_branch, calls);
                    }
                    Action::Wait { auto, .. } => collect_calls(auto, calls),
                    _ => {}
                }
            }
        }

        async fn dfs(
            wf_id: &str,
            store: &WorkflowStore,
            path: &mut Vec<String>,
            errors: &mut Vec<String>,
            depth: u32,
        ) {
            const MAX_CALL_DEPTH: u32 = 10;
            if depth > MAX_CALL_DEPTH {
                return;
            }
            let Some(wf) = store.get(wf_id).await else {
                return;
            };
            let mut calls = HashSet::new();
            collect_calls(&wf.steps, &mut calls);
            for target in calls {
                if path.contains(&target) {
                    errors.push(format!(
                        "检测到循环调用链: {} → {}",
                        path.join(" → "),
                        target
                    ));
                    continue;
                }
                path.push(target.clone());
                Box::pin(dfs(&target, store, path, errors, depth + 1)).await;
                path.pop();
            }
        }

        let mut errors = Vec::new();
        let mut path = vec![workflow.id.clone()];
        dfs(&workflow.id, store, &mut path, &mut errors, 0).await;
        errors
    }
}
