//! store.rs — WorkflowStore 持久化层
//!
//! 每个工作流独立 JSON 文件 + RwLock 缓存 + index.json 摘要索引。

use crate::workflow::types::{Action, LoopDef, Step, Workflow, WorkflowSummary};
use crate::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// 每个 workflow 的自有目录结构：
///
/// plugin/workflows/{id}/
/// ├── workflow.json        # 工作流定义（含 doc 字段）
/// ├── screenshots/         # 截图
/// └── anchors/             # 锚点数据
///
/// Workflow 持久化存储
pub struct WorkflowStore {
    /// 根目录 plugin/workflows/
    root: PathBuf,
    /// 内存缓存：workflow_id → Arc<Workflow>
    cache: RwLock<HashMap<String, Arc<Workflow>>>,
    /// 摘要缓存（列表用）
    summary_cache: RwLock<Vec<WorkflowSummary>>,
    /// 同一 run 内是否已备份：reset 于 run 开始，首次 save 置 true
    run_has_backup: Mutex<bool>,
}

impl WorkflowStore {
    /// 创建或打开存储
    pub fn new() -> Self {
        let root = Self::default_root();

        Self {
            root,
            cache: RwLock::new(HashMap::new()),
            summary_cache: RwLock::new(Vec::new()),
            run_has_backup: Mutex::new(false),
        }
    }

    /// 默认根目录：项目根/plugin/workflows/
    /// CARGO_MANIFEST_DIR 指向 src/（nuphus lib crate），上溯一级到项目根
    fn default_root() -> PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let project_root = manifest.parent().unwrap_or(manifest);
        project_root.join("plugin").join("workflows")
    }

    /// 指定根目录创建（用于测试）
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            cache: RwLock::new(HashMap::new()),
            summary_cache: RwLock::new(Vec::new()),
            run_has_backup: Mutex::new(false),
        }
    }

    /// 工作流存储根目录
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 工作流专属目录
    pub fn workflow_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// 锚点目录
    pub fn anchors_dir(&self, id: &str) -> PathBuf {
        self.workflow_dir(id).join("anchors")
    }

    /// 导出 workflow.json（确定性执行定义）
    pub fn export_json(&self, workflow: &Workflow) -> String {
        #[derive(serde::Serialize)]
        struct ExportStep {
            name: String,
            description: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            tool: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            children: Vec<ExportStep>,
            #[serde(skip_serializing_if = "Option::is_none")]
            loop_def: Option<LoopDef>,
        }

        fn convert_step(step: &Step) -> ExportStep {
            match &step.action {
                Action::Tool { tool, with } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some(tool.clone()),
                    params: Some(with.clone()),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Seq { seq } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: None,
                    params: None,
                    children: seq.iter().map(convert_step).collect(),
                    loop_def: None,
                },
                Action::Loop { def } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("loop".to_string()),
                    params: None,
                    children: def.steps.iter().map(convert_step).collect(),
                    loop_def: Some(def.clone()),
                },
                Action::If { def } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("if".to_string()),
                    params: None,
                    children: {
                        let mut children = Vec::new();
                        children.push(ExportStep {
                            name: "then".into(),
                            description: String::new(),
                            tool: None,
                            params: None,
                            children: def.then.iter().map(convert_step).collect(),
                            loop_def: None,
                        });
                        if !def.else_branch.is_empty() {
                            children.push(ExportStep {
                                name: "else".into(),
                                description: String::new(),
                                tool: None,
                                params: None,
                                children: def.else_branch.iter().map(convert_step).collect(),
                                loop_def: None,
                            });
                        }
                        children
                    },
                    loop_def: None,
                },
                Action::Call { call, with } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some(format!("call:{}", call)),
                    params: Some(with.clone()),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Wait { wait, auto } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("wait".into()),
                    params: Some(serde_json::json!({ "prompt": wait })),
                    children: auto.iter().map(convert_step).collect(),
                    loop_def: None,
                },
                Action::Chat { chat, with: opts } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("chat_agent".into()),
                    params: Some(serde_json::json!({
                        "message": chat,
                        "screenshot": opts.screenshot,
                        "max_iterations": opts.max_steps,
                        "tools": opts.tools,
                        "knowledge": opts.knowledge,
                    })),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Script { script } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some(format!("script:{}", script.runtime)),
                    params: Some(serde_json::json!({
                        "runtime": script.runtime,
                        "code": script.code,
                    })),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Assert { assert } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("assert".into()),
                    params: Some(serde_json::json!({ "message": assert.message })),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Mcp { mcp } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some(format!("mcp:{}::{}", mcp.server, mcp.tool)),
                    params: Some(mcp.with.clone()),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Sleep { sleep } => ExportStep {
                    name: step.name.clone(),
                    description: step.description.clone(),
                    tool: Some("system_sleep".into()),
                    params: Some(serde_json::json!({ "seconds": sleep })),
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Break { .. } => ExportStep {
                    name: "_break".into(),
                    description: String::new(),
                    tool: Some("break".into()),
                    params: None,
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Continue { .. } => ExportStep {
                    name: "_continue".into(),
                    description: String::new(),
                    tool: Some("continue".into()),
                    params: None,
                    children: Vec::new(),
                    loop_def: None,
                },
                Action::Custom(_) => ExportStep {
                    name: step.name.clone(),
                    description: "custom step".into(),
                    tool: Some("custom".into()),
                    params: None,
                    children: Vec::new(),
                    loop_def: None,
                },
            }
        }

        let steps: Vec<ExportStep> = workflow.steps.iter().map(convert_step).collect();

        let output = serde_json::json!({
            "name": workflow.name,
            "version": 1,
            "created_at": workflow.created_at.unwrap_or_else(chrono::Utc::now).to_rfc3339(),
            "updated_at": workflow.updated_at.unwrap_or_else(chrono::Utc::now).to_rfc3339(),
            "parameters": {},
            "steps": steps,
        });

        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    }

    /// 导出 workflow.md（人类可读的 Markdown 文档）
    pub fn export_md(&self, workflow: &Workflow) -> String {
        fn format_step(step: &Step, depth: usize) -> String {
            let indent = "  ".repeat(depth);
            let header_prefix = if depth == 0 { "###" } else { "####" };
            let mut out = String::new();

            match &step.action {
                Action::Tool { tool, with } => {
                    out.push_str(&format!(
                        "{} {}. {} (tool: {})\n",
                        header_prefix, step.id, step.name, tool
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    let params =
                        serde_json::to_string_pretty(with).unwrap_or_else(|_| "null".into());
                    out.push_str(&format!("{}- 参数:\n{}{}\n", indent, indent, params));
                }
                Action::Seq { seq } => {
                    out.push_str(&format!(
                        "{} {}. {} (seq)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!("{}- 子步骤数: {}\n", indent, seq.len()));
                    for child in seq {
                        out.push_str(&format_step(child, depth + 1));
                    }
                }
                Action::Loop { def } => {
                    out.push_str(&format!(
                        "{} {}. {} (loop)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    if let Some(ref for_each) = def.for_each {
                        out.push_str(&format!(
                            "{}- for_each: {:?} as {}\n",
                            indent, for_each.items, for_each.item_var
                        ));
                    }
                    if let Some(repeat) = def.repeat {
                        out.push_str(&format!("{}- repeat: {}\n", indent, repeat));
                    }
                    out.push_str(&format!("{}- max: {}\n", indent, def.max));
                    out.push_str(&format!("{}- 子步骤数: {}\n", indent, def.steps.len()));
                    for child in &def.steps {
                        out.push_str(&format_step(child, depth + 1));
                    }
                }
                Action::If { def } => {
                    out.push_str(&format!(
                        "{} {}. {} (if)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!("{}- 条件: {:?}\n", indent, def.condition));
                    out.push_str(&format!("{}  then ({} steps):\n", indent, def.then.len()));
                    for child in &def.then {
                        out.push_str(&format_step(child, depth + 1));
                    }
                    if !def.else_branch.is_empty() {
                        out.push_str(&format!(
                            "{}  else ({} steps):\n",
                            indent,
                            def.else_branch.len()
                        ));
                        for child in &def.else_branch {
                            out.push_str(&format_step(child, depth + 1));
                        }
                    }
                }
                Action::Call { call, with } => {
                    out.push_str(&format!(
                        "{} {}. {} (call: {})\n",
                        header_prefix, step.id, step.name, call
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    let params =
                        serde_json::to_string_pretty(with).unwrap_or_else(|_| "null".into());
                    if params != "null" && params != "{}" {
                        out.push_str(&format!("{}- 参数:\n{}{}\n", indent, indent, params));
                    }
                }
                Action::Wait { wait, auto } => {
                    out.push_str(&format!(
                        "{} {}. {} (wait)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!("{}- 提示词: {}\n", indent, wait));
                    if !auto.is_empty() {
                        out.push_str(&format!("{}- 自动步骤数: {}\n", indent, auto.len()));
                        for child in auto {
                            out.push_str(&format_step(child, depth + 1));
                        }
                    }
                }
                Action::Chat { chat, with: opts } => {
                    out.push_str(&format!(
                        "{} {}. {} (chat_agent)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!("{}- 消息: {}\n", indent, chat));
                    out.push_str(&format!("{}- 截图: {}\n", indent, opts.screenshot));
                    if let Some(ms) = opts.max_steps {
                        out.push_str(&format!("{}- 最大步数: {}\n", indent, ms));
                    }
                    if let Some(ref tools) = opts.tools {
                        out.push_str(&format!("{}- 工具: {:?}\n", indent, tools));
                    }
                    if let Some(ref knowledge) = opts.knowledge {
                        out.push_str(&format!("{}- 知识库: {:?}\n", indent, knowledge));
                    }
                }
                Action::Script { script } => {
                    out.push_str(&format!(
                        "{} {}. {} (script: {})\n",
                        header_prefix, step.id, step.name, script.runtime
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!(
                        "{}- 代码:\n```{}\n{}\n```\n",
                        indent, script.runtime, script.code
                    ));
                }
                Action::Assert { assert } => {
                    out.push_str(&format!(
                        "{} {}. {} (assert)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    out.push_str(&format!("{}- 条件: {:?}\n", indent, assert.condition));
                    if let Some(ref msg) = assert.message {
                        out.push_str(&format!("{}- 失败信息: {}\n", indent, msg));
                    }
                }
                Action::Mcp { mcp } => {
                    out.push_str(&format!(
                        "{} {}. {} (mcp: {}::{})\n",
                        header_prefix, step.id, step.name, mcp.server, mcp.tool
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                    let params =
                        serde_json::to_string_pretty(&mcp.with).unwrap_or_else(|_| "null".into());
                    if params != "null" && params != "{}" {
                        out.push_str(&format!("{}- 参数:\n{}{}\n", indent, indent, params));
                    }
                }
                Action::Sleep { sleep } => {
                    out.push_str(&format!(
                        "{} {}. {} (sleep: {}s)\n",
                        header_prefix, step.id, step.name, sleep
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                }
                Action::Break { .. } => {
                    out.push_str(&format!("{} {}. break\n", header_prefix, step.id));
                }
                Action::Continue { .. } => {
                    out.push_str(&format!("{} {}. continue\n", header_prefix, step.id));
                }
                Action::Custom(_) => {
                    out.push_str(&format!(
                        "{} {}. {} (custom)\n",
                        header_prefix, step.id, step.name
                    ));
                    if !step.description.is_empty() {
                        out.push_str(&format!("{}- 描述: {}\n", indent, step.description));
                    }
                }
            }
            out.push('\n');
            out
        }

        let mut md = String::new();

        // Title + description
        md.push_str(&format!("# {}\n", workflow.name));
        if let Some(ref doc) = workflow.doc {
            md.push_str(&format!("{}\n", doc));
        }
        md.push('\n');

        // Parameters section (extracted from step captures)
        let params: Vec<&str> = workflow
            .steps
            .iter()
            .filter_map(|s| s.capture.as_deref())
            .collect();
        if !params.is_empty() {
            md.push_str("## 参数\n");
            for p in &params {
                md.push_str(&format!("- {}\n", p));
            }
            md.push('\n');
        }

        // Steps section
        md.push_str("## 步骤\n");
        if workflow.steps.is_empty() {
            md.push_str("_无步骤_\n");
        } else {
            for step in &workflow.steps {
                md.push_str(&format_step(step, 0));
            }
        }

        md
    }

    // ── 目录管理 ──

    /// 确保工作流的子目录全部存在
    pub async fn ensure_dirs(&self, id: &str) -> Result<()> {
        tokio::fs::create_dir_all(self.anchors_dir(id)).await?;
        // screenshots 目录由 process_screenshot 创建
        tokio::fs::create_dir_all(self.workflow_dir(id).join("screenshots")).await?;
        Ok(())
    }

    /// 保存工作流说明文档（guide.md）
    pub async fn save_guide_file(&self, workflow_id: &str, content: &str) -> Result<()> {
        let path = self.workflow_dir(workflow_id).join("guide.md");
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    // 读取工作流脚本文件（.wf.json）

    // ── 启动加载 ──

    /// 启动时加载所有工作流到缓存
    /// 1. 优先从 index.json 加载（反序列化失败则 fallback 扫描目录）
    /// 2. 单个 workflow.json 反序列化失败时保留摘要（列表可见，执行时报错）
    pub async fn load_all(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.root).await?;

        let index_path = self.root.join("index.json");
        let summaries: Vec<WorkflowSummary> =
            if tokio::fs::try_exists(&index_path).await.unwrap_or(false) {
                let data = tokio::fs::read_to_string(&index_path).await?;
                match serde_json::from_str::<Vec<WorkflowSummary>>(&data) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("index.json 反序列化失败 ({}), fallback 扫描目录", e);
                        self.scan_dirs()
                    }
                }
            } else {
                Vec::new()
            };

        let mut cache = self.cache.write().await;
        cache.clear(); // 热加载：先清空旧缓存再重新加载
        for summary in &summaries {
            let wf_path = self.root.join(&summary.id).join("workflow.json");
            if tokio::fs::try_exists(&wf_path).await.unwrap_or(false) {
                let wf_data = tokio::fs::read_to_string(&wf_path).await?;
                match serde_json::from_str::<Workflow>(&wf_data) {
                    Ok(wf) => {
                        cache.insert(summary.id.clone(), Arc::new(wf));
                    }
                    Err(e) => {
                        tracing::error!("[store] 工作流 '{}' 反序列化失败: {e}", summary.id);
                    }
                }
            }
        }
        *self.summary_cache.write().await = summaries;

        Ok(())
    }

    /// 扫描目录生成摘要（index.json 损坏时的 fallback）
    fn scan_dirs(&self) -> Vec<WorkflowSummary> {
        let mut summaries = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return summaries,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let wf_file = path.join("workflow.json");
            if !wf_file.exists() {
                continue;
            }
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name.is_empty() {
                continue;
            }
            if let Ok(data) = std::fs::read_to_string(&wf_file) {
                if let Ok(wf) = serde_json::from_str::<Workflow>(&data) {
                    summaries.push(WorkflowSummary::from(&wf));
                }
            }
        }
        summaries
    }

    // ── 读取 ──

    /// 列出所有工作流摘要（从缓存读；缓存为空时自动 load）
    pub async fn list(&self) -> Vec<WorkflowSummary> {
        {
            let cache = self.summary_cache.read().await;
            if !cache.is_empty() {
                return cache.clone();
            }
        }
        // 缓存为空，尝试从磁盘加载
        let _ = self.load_all().await;
        self.summary_cache.read().await.clone()
    }

    /// 获取单个工作流（缓存 → 回源文件）
    pub async fn get(&self, id: &str) -> Option<Workflow> {
        // 先查缓存
        if let Some(wf) = self.cache.read().await.get(id) {
            return Some(wf.as_ref().clone());
        }

        // 缓存未命中，从磁盘加载
        let wf_path = self.root.join(id).join("workflow.json");
        let data = tokio::fs::read_to_string(&wf_path).await.ok()?;
        let mut wf: Workflow = match serde_json::from_str(&data) {
            Ok(wf) => wf,
            Err(e) => {
                tracing::warn!(
                    "Failed to deserialize workflow '{}' from {}: {e}",
                    id,
                    wf_path.display()
                );
                return None;
            }
        };

        // 旧数据缺少 created_at/updated_at 时补填当前时间
        if wf.created_at.is_none() {
            wf.created_at = Some(chrono::Utc::now());
        }
        if wf.updated_at.is_none() {
            wf.updated_at = Some(chrono::Utc::now());
        }

        // 回填缓存 + 同步 summary_cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(id.to_string(), Arc::new(wf.clone()));
        }
        {
            let mut sc = self.summary_cache.write().await;
            if let Some(pos) = sc.iter().position(|s| s.id == id) {
                sc[pos] = WorkflowSummary::from(&wf);
            } else {
                sc.push(WorkflowSummary::from(&wf));
            }
        }
        Some(wf)
    }

    // ── 写入 ──

    /// 标记一次新的 workflow run 开始，重置备份标志。
    /// 调用方：execute_v2 入口处。
    pub async fn begin_run_backup_window(&self) {
        *self.run_has_backup.lock().await = false;
    }

    /// 保存工作流（写锁 → 序列化 → 写磁盘 → 更新缓存）
    pub async fn save(&self, wf: &Workflow) -> Result<()> {
        let id = wf.id.clone();
        let wf_dir = self.root.join(&id);
        tokio::fs::create_dir_all(&wf_dir).await?;

        // 序列化
        let json = serde_json::to_string_pretty(wf)?;

        let final_path = wf_dir.join("workflow.json");

        // 版本备份：同一 run 内只备份一次（由 begin_run_backup_window 控制）
        let mut has_backup = self.run_has_backup.lock().await;
        if !*has_backup && tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
            let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
            let backup_path = wf_dir.join(format!("workflow.{}.json", ts));
            let _ = tokio::fs::rename(&final_path, &backup_path).await;
            *has_backup = true;
        }
        drop(has_backup);

        // 原子写入（先写临时文件再 rename）
        let tmp_path = wf_dir.join("workflow.json.tmp");
        tokio::fs::write(&tmp_path, &json).await?;
        tokio::fs::rename(&tmp_path, &final_path).await?;

        // ── 清理旧备份：保留最近 3 个 ──
        self.prune_backups(&wf_dir, 3).await;

        // 更新缓存
        self.cache.write().await.insert(id, Arc::new(wf.clone()));

        // 更新 index.json
        self.flush_index().await?;

        Ok(())
    }

    /// 清理旧备份文件，保留最近 `max` 个
    async fn prune_backups(&self, wf_dir: &Path, max: usize) {
        let mut backups: Vec<String> = match tokio::fs::read_dir(wf_dir).await {
            Ok(mut entries) => {
                let mut names = Vec::new();
                loop {
                    match entries.next_entry().await {
                        Ok(Some(entry)) => {
                            let name = entry.file_name().to_string_lossy().to_string();
                            // 匹配 backup 文件：workflow.{ts}.json
                            if name.starts_with("workflow.")
                                && name.ends_with(".json")
                                && name != "workflow.json"
                            {
                                names.push(name);
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                names
            }
            Err(_) => return,
        };

        if backups.len() > max {
            // 按文件名字典序排序（时间戳格式保证字典序 = 时间顺序）
            backups.sort();
            let to_delete = backups.len() - max;
            for old in backups.iter().take(to_delete) {
                let _ = tokio::fs::remove_file(wf_dir.join(old)).await;
                tracing::debug!("Pruned old backup: {}", old);
            }
        }
    }

    /// 删除工作流
    pub async fn delete(&self, id: &str) -> Result<()> {
        // 删目录
        let wf_dir = self.root.join(id);
        if tokio::fs::try_exists(&wf_dir).await.unwrap_or(false) {
            tokio::fs::remove_dir_all(&wf_dir).await?;
        }

        // 删缓存
        self.cache.write().await.remove(id);

        // 删除旧 index.json 以绕过 flush_index 空缓存保护，确保索引与缓存一致
        let _ = tokio::fs::remove_file(self.root.join("index.json")).await;

        // 更新 index.json
        self.flush_index().await?;

        Ok(())
    }

    // ── 索引维护 ──

    /// 刷新 index.json（从缓存重建摘要）
    pub async fn flush_index(&self) -> Result<()> {
        let cache = self.cache.read().await;
        // Guard: 缓存为空且 index.json 已存在时，保留磁盘索引不被空缓存覆盖
        if cache.is_empty() {
            let index_path = self.root.join("index.json");
            if tokio::fs::try_exists(&index_path).await.unwrap_or(false) {
                tracing::debug!("flush_index: cache empty, preserving existing index.json");
                return Ok(());
            }
        }
        let summaries: Vec<WorkflowSummary> = cache
            .values()
            .map(|wf| WorkflowSummary::from(wf.as_ref()))
            .collect();

        let json = serde_json::to_string_pretty(&summaries)?;
        let tmp_path = self.root.join("index.json.tmp");
        let final_path = self.root.join("index.json");
        tokio::fs::write(&tmp_path, &json).await?;
        tokio::fs::rename(&tmp_path, &final_path).await?;

        *self.summary_cache.write().await = summaries;

        Ok(())
    }
}

impl Default for WorkflowStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 创建临时目录用于测试
    fn tmp_root() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nuphus_workflow_test_{}", uuid::Uuid::new_v4()));
        path
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let root = tmp_root();
        let store = WorkflowStore::with_root(root.clone());

        let wf = Workflow::new("测试工作流");
        store.save(&wf).await.unwrap();

        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "测试工作流");

        // 清理
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_get_and_delete() {
        let root = tmp_root();
        let store = WorkflowStore::with_root(root.clone());

        let wf = Workflow::new("待删除");
        let id = wf.id.clone();
        store.save(&wf).await.unwrap();

        let loaded = store.get(&id).await;
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "待删除");

        store.delete(&id).await.unwrap();
        let list = store.list().await;
        assert_eq!(list.len(), 0);

        // 清理
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
