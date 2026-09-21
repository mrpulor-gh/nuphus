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
//! - inputs 声明 ↔ 引用一致性：引用 {{inputs.x}} 未声明 → error；声明但未被引用 → warning
//!
//! 编译产出：ValidationReport（errors 阻断执行，warnings 仅提示）

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::{Action, Condition, Step, Workflow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

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
    /// 声明的外部输入名（workflow.inputs，声明顺序）
    declared_inputs: Vec<String>,
    /// 步骤模板/条件中实际引用到的输入名（用于「声明未被引用」warning）
    referenced_inputs: BTreeSet<String>,
    /// 已出现的未声明输入引用（按名字去重，finalize 时统一报 error）
    missing_inputs: BTreeSet<String>,
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

    /// 登记一处 `inputs.<name>` 引用（步骤模板与条件 VarRef 均经此路径）。
    /// 未声明的引用先收集去重，遍历结束后由 finalize_inputs 统一报 error。
    fn note_input_ref(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        if self.declared_inputs.iter().any(|d| d == name) {
            self.referenced_inputs.insert(name.to_string());
        } else {
            self.missing_inputs.insert(name.to_string());
        }
    }

    /// 汇总 inputs 校验：未声明引用 → error；声明但全程未被引用 → warning
    fn finalize_inputs(&mut self) {
        for name in &self.missing_inputs {
            self.errors.push(format!(
                "未声明的输入引用 {{{{inputs.{}}}}}（请在 workflow.inputs 声明）",
                name
            ));
        }
        for name in &self.declared_inputs {
            if !self.referenced_inputs.contains(name) {
                self.warnings
                    .push(format!("输入 {} 已声明但未被任何步骤引用", name));
            }
        }
    }

    /// 检查 VarRef 中的变量引用。
    /// `inputs.<name>` → 登记声明一致性（未声明由 finalize_inputs 报 error）；
    /// 其余变量沿用前向引用 warning 语义。
    fn check_var_ref(&mut self, r: &crate::workflow::types::VarRef, owner: &str) {
        if let crate::workflow::types::VarRef::Var { var } = r {
            if let Some(name) = var.strip_prefix("inputs.") {
                if let Some(first) = name.split('.').next() {
                    self.note_input_ref(first);
                }
                return;
            }
            if !self.captured.contains(var) && var != "_index" && !var.starts_with("ENV:") {
                self.warnings.push(format!(
                    "条件步骤 '{}': 变量 '{}' 尚未被先前步骤捕获，求值将为 false（运行时可能由 inputs 注入）",
                    owner, var
                ));
            }
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
                    self.check_var_ref(r, owner);
                }
            }
            Condition::NotEmpty { not_empty } => {
                self.check_var_ref(not_empty, owner);
                // NotEmpty: checks that var is non-empty — always valid at compile time
            }
            Condition::Empty { empty } => {
                self.check_var_ref(empty, owner);
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
                        self.check_var_ref(r, owner);
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
                        // params / inputs 为运行时注入的命名空间，前向引用 warning 不适用
                        // （inputs 的声明一致性由 finalize_inputs 单独校验）
                        !self.captured.contains(name)
                            && name != "params"
                            && name != "inputs"
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

/// 收集步骤模板中引用的输入名：扫描 `{{inputs.<name>}}`（含管道写法 `{{inputs.x | ...}}`）。
///
/// 覆盖整棵步骤树：`seq / loop.do / if.then / if.else / wait.auto` 等嵌套子步骤随 Step
/// 序列化一并在内，无需再手写逐变体遍历（新步骤类型/字段自动纳入）。
fn collect_template_input_refs(steps: &[Step], out: &mut BTreeSet<String>) {
    static INPUT_REF_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = INPUT_REF_RE.get_or_init(|| {
        regex::Regex::new(r"\{\{\s*inputs\.([A-Za-z_][A-Za-z0-9_]*)")
            .expect("input ref regex is statically valid")
    });
    for step in steps {
        // 步骤为纯数据类型，序列化不会失败；万一失败则跳过（不阻断校验）
        if let Ok(v) = serde_json::to_value(step) {
            collect_input_refs_in_value(&v, re, out);
        }
    }
}

/// 递归扫描 JSON 字符串中的 `{{inputs.<name>}}` 引用
fn collect_input_refs_in_value(
    v: &serde_json::Value,
    re: &regex::Regex,
    out: &mut BTreeSet<String>,
) {
    match v {
        serde_json::Value::String(s) => {
            for cap in re.captures_iter(s) {
                if let Some(m) = cap.get(1) {
                    out.insert(m.as_str().to_string());
                }
            }
        }
        serde_json::Value::Object(m) => {
            for val in m.values() {
                collect_input_refs_in_value(val, re, out);
            }
        }
        serde_json::Value::Array(a) => {
            for val in a {
                collect_input_refs_in_value(val, re, out);
            }
        }
        _ => {}
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
            declared_inputs: workflow.inputs.iter().map(|i| i.name.clone()).collect(),
            referenced_inputs: BTreeSet::new(),
            missing_inputs: BTreeSet::new(),
        };

        ctx.errors
            .extend(crate::workflow::inputs::validate_specs(&workflow.inputs));

        // inputs 声明一致性：先扫步骤模板，条件 VarRef 在遍历中就地登记
        let mut template_refs = BTreeSet::new();
        collect_template_input_refs(&workflow.steps, &mut template_refs);
        for name in template_refs {
            ctx.note_input_ref(&name);
        }

        if workflow.steps.is_empty() {
            ctx.warnings.push("工作流没有任何步骤".to_string());
            ctx.finalize_inputs();
            return ValidationReport {
                passed: ctx.errors.is_empty(),
                warnings: ctx.warnings,
                errors: ctx.errors,
            };
        }

        for step in &workflow.steps {
            Self::validate_step(step, &mut ctx);
        }
        ctx.finalize_inputs();

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
                    ctx.check_var_ref(&fe.items, &step.name);
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
                // 显式 provider+model 必须存在；旧数据无 provider 时仅允许唯一候选。
                if let (Some(model_id), Some(registry)) = (&opts.model, ctx.models) {
                    if let Some(provider) = &opts.provider {
                        if registry
                            .find_model_for_provider(provider, model_id)
                            .is_none()
                        {
                            ctx.errors.push(format!(
                                "Chat step '{}': 模型 '{}' 不存在于 provider '{}'",
                                step.name, model_id, provider
                            ));
                        }
                    } else {
                        match registry.find_model_candidates(model_id).len() {
                            0 => ctx.warnings.push(format!(
                                "Chat step '{}': 模型 '{}' 不在 registry 中，执行时无法精确路由",
                                step.name, model_id
                            )),
                            1 => {}
                            count => ctx.errors.push(format!(
                                "Chat step '{}': 模型 '{}' 在 registry 中有 {} 个 provider 候选，必须指定 provider",
                                step.name, model_id, count
                            )),
                        }
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
        #[derive(Clone)]
        struct CallSite {
            target: String,
            owner: String,
            with: serde_json::Value,
        }

        fn collect_calls(steps: &[Step], calls: &mut Vec<CallSite>) {
            for step in steps {
                match &step.action {
                    Action::Call { call, with } => {
                        calls.push(CallSite {
                            target: call.clone(),
                            owner: step.name.clone(),
                            with: with.clone(),
                        });
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
            let mut calls = Vec::new();
            collect_calls(&wf.steps, &mut calls);
            for call in calls {
                let target = call.target;
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
        let mut sites = Vec::new();
        collect_calls(&workflow.steps, &mut sites);
        for site in &sites {
            let Some(target) = store.get(&site.target).await else {
                errors.push(format!(
                    "Call step '{}': 目标工作流 '{}' 不存在",
                    site.owner, site.target
                ));
                continue;
            };
            let with = if site.with.is_null() {
                serde_json::Map::new()
            } else if let Some(with) = site.with.as_object() {
                with.clone()
            } else {
                errors.push(format!("Call step '{}': with 必须是对象", site.owner));
                continue;
            };
            let input_map = match with.get("inputs") {
                None => serde_json::Map::new(),
                Some(value) if value.is_object() => value.as_object().cloned().unwrap_or_default(),
                Some(_) => {
                    errors.push(format!(
                        "Call step '{}': with.inputs 必须是对象",
                        site.owner
                    ));
                    continue;
                }
            };
            if let Some(outputs) = with.get("outputs") {
                match outputs.as_object() {
                    None => errors.push(format!(
                        "Call step '{}': with.outputs 必须是对象",
                        site.owner
                    )),
                    Some(map) => {
                        for (name, parent) in map {
                            if !parent.is_string() {
                                errors.push(format!(
                                    "Call step '{}': with.outputs.{} 必须映射到父变量名字符串",
                                    site.owner, name
                                ));
                            }
                        }
                    }
                }
            }
            for spec in &target.inputs {
                match input_map.get(&spec.name) {
                    None if spec.required && spec.default.is_none() => errors.push(format!(
                        "Call step '{}': 子工作流 '{}' 缺少必填输入映射 '{}'",
                        site.owner, target.name, spec.name
                    )),
                    Some(value)
                        if !value.as_str().is_some_and(|text| text.contains("{{"))
                            && !crate::workflow::inputs::value_matches_kind(spec.kind, value) =>
                    {
                        errors.push(format!(
                            "Call step '{}': 输入映射 '{}' 类型不符合子工作流声明",
                            site.owner, spec.name
                        ));
                    }
                    _ => {}
                }
            }
        }
        let mut path = vec![workflow.id.clone()];
        dfs(&workflow.id, store, &mut path, &mut errors, 0).await;
        errors
    }
}
