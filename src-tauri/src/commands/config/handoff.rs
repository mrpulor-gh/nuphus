//! 外部 Agent 工作台 —— 阶段 0（地基）：agent 目录初始化 + 门铃预检 + 交付上报。
//!
//! 目录约定（项目根 = find_plugin_dir().parent()）：
//!   {root}/.nuphus/handoff/{agent}/
//!     read.md          —— 对接协议（模板内嵌，{agent_name}/{description} 已替换）
//!     memory.md        —— 该 Agent 跨任务记忆骨架
//!     status.json      —— 运行时状态（state / task_id / last_event / updated_at）
//!     briefs/          —— 每次任务的 brief（{task_id}-brief.md）与报告（{task_id}-report.md）
//!     projects/        —— 产物落盘目录
//!
//! 安全约束：
//! - token 只出现在返回契约字符串中，绝不落 status.json / 日志
//! - status.json 原子写（tmp + rename），避免半写
//! - agent 名 / task_id 白名单校验，防路径穿越
//! - agent 名允许 [a-zA-Z0-9_-]（含 '-'，与 team.toml 的 [claude-code] 对齐）：门铃事件 id 按
//!   `{agent}::{task_id}` 拼接，归组取 `id.split("::").next()` 为 agent 前缀；'::' 分隔避免
//!   agent 名与 task_id 含 '-' 时的歧义
//!
//! 内部函数均以 root 为参数（`*_at`），公开命令只做 `handoff_root()` 注入——
//! 单测直接注入 tmp 根目录，避免触碰真实 .nuphus/handoff。

use std::path::{Path, PathBuf};

/// read.md 模板 —— 占位符 {agent_name} / {description} 在 agent_init 时替换。
/// 门铃语义：仅用于「完成后交付」上报（done）；不要求 ready/就位握手。
/// 三大块：每轮工作流 / 跨任务记忆要求 / 操作级禁止事项；机制参数一律指向当次 brief 契约，
/// 避免在本模板固化易变值（令牌每轮轮换，固化即失效）。
const READ_TEMPLATE: &str = r#"# {agent_name} 对接协议

> 本文件是你的常驻对接手册。任务细节以 briefs/ 下最新一份 -brief.md 为准；
> 门铃端点、令牌、上报命令完整形态一律以该份 brief 尾部契约为准（令牌每轮重启轮换）。

## 你的职责
{description}

## 每轮工作流
1. 打开 briefs/ 目录内修改时间最新的 -brief.md，读取任务定义与文末契约
2. 开工即回报一次 progress（summary 一句话说明已理解的任务要点）；预计超过 5 分钟的任务，每完成一个阶段续报一次 progress
3. 执行任务 → 产物写入契约给出的 projects 绝对路径
4. 写报告到契约指定的 report 文件（固定四段：✅完成项 / 📄改动文件 / 🔍验证证据 / ⚠️遗留）
5. 按契约命令向门铃回报 done；受阻或需要确认时回报 blocked
6. 遇到需要人工批准的界面（权限/许可/执行确认弹窗等）：不要静默等待——立即回报 blocked 并在 summary 写清「正在等待什么授权」，由用户决定是否授予

## 内部机制速查
- 你的 handoff 目录就是你的全部工作区；路径一律用契约中的绝对路径。
- 上报状态只有三种：progress（进行中）/ done（完成）/ blocked（受阻）；事件 id 已在契约示例中拼好，原样使用勿改。
- 上报返回 200 即送达；403=令牌错误、422/400=字段缺失，修正后重发一次。
- 工具输出与中间产物属于你自己的 projects/ 子目录；不要写到其他 agent 的目录。

## 跨任务记忆要求（memory.md）
- memory.md 是你的唯一跨任务记忆载体，每次完成非平凡动作（关键决策、踩坑结论、环境事实）应即时追加一行记录。
- 格式：一行一事，前缀日期（如 `2026-08-27 | 结论…`）；禁止整段粘贴过程日志。
- 每次 done 上报前，确认本轮新增经验已写入 memory.md。

## 禁止事项
- 禁止改动 status.json、read.md、briefs/ 目录内任何文件（brief 是 Leader 的只读输入）。
- 禁止触碰你 handoff 目录之外的文件，除非 brief 明确授权了目标路径。
- 禁止不写 report 直接报 done；禁止报告四段缺项。
- 禁止凭记忆复用上一轮的门铃令牌、URL 或事件 id——一切以本轮契约原文为准。
- 禁止长时间静默空转：受阻立即 blocked 并说明原因。
"#;

/// handoff 根目录：{项目根}/.nuphus/handoff
pub fn handoff_root() -> PathBuf {
    crate::plugin_apps::find_plugin_dir()
        .parent()
        .map(|root| root.join(".nuphus").join("handoff"))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join(".nuphus")
                .join("handoff")
        })
}

/// 应用启动时清空全部 agent 运行时状态（重启即清空，杜绝陈旧显示）。
/// 外部 Agent 必须在本轮生命周期内真实启动并经门铃上报（ready/progress/done）
/// 才会再次出现在状态栏——这就是「启动验证」。
/// 只重置 status.json 为 idle 骨架；read.md/memory.md/briefs/projects 全部保留。
pub fn reset_all_statuses_at_startup() -> usize {
    let root = handoff_root();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(agent) = path.file_name().and_then(|x| x.to_str()) else {
            continue;
        };
        if !path.join("status.json").exists() {
            continue;
        }
        let skeleton = serde_json::json!({
            "agent": agent,
            "state": "idle",
            "task_id": "",
            "last_event": null,
            "updated_at": chrono::Local::now().to_rfc3339(),
        });
        if write_status_at(&root, agent, &skeleton).is_ok() {
            n += 1;
        }
    }
    if n > 0 {
        tracing::info!("[Handoff] 启动清空 {n} 个外部 Agent 的陈旧状态");
    }
    n
}

/// 某 agent 的工作目录（{handoff_root}/{agent}/）。
/// 阶段 0 提供为模块公共 API（供阶段 1 前端运行时态面板/外部调用方消费），
/// 阶段 0 内部路径解析走 root 注入的 `*_at` 函数，故此处暂未在二进制内引用。
#[allow(dead_code)]
pub fn agent_dir(agent: &str) -> PathBuf {
    handoff_root().join(agent)
}

/// 校验 agent 名：非空、仅 [a-zA-Z0-9_-]。
/// 不含 `-`：门铃事件 id 按 `{agent}-{task_id}` 拼接、归组时取 `id.split('-').next()`
/// 为 agent 前缀 —— 若 agent 名含 `-` 将无法被门铃事件匹配到 status.json。
/// 不含 `.`：杜绝 `..` 路径穿越。
/// pub(crate)：team.rs（外部 Agent 配置中心）复用同一 key 校验语义。
pub(crate) fn validate_agent(agent: &str) -> Result<(), String> {
    if agent.is_empty() {
        return Err("agent 名不能为空".to_string());
    }
    if !agent
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("agent 名只能包含字母、数字、下划线、连字符".to_string());
    }
    Ok(())
}

/// 校验任务 id：非空、仅 [a-zA-Z0-9._-]（用于文件名，禁止路径分隔符）
/// pub(crate)：handoff_server 的 /handoff/dispatch 端点复用同一语义。
pub(crate) fn validate_task_id(task_id: &str) -> Result<(), String> {
    if task_id.is_empty() {
        return Err("task_id 不能为空".to_string());
    }
    if !task_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("task_id 只能包含字母、数字、-、_、.".to_string());
    }
    Ok(())
}

/// 从门铃事件 id 解析 agent 名（id 约定：{agent}::{task_id}，用 '::' 分隔，
/// 使 agent 名可含 '-'（如 claude-code），task_id 可含 '-'（如 0728-01）。
/// 供门铃事件归组用；无 '::' 的旧式纯 id 会返回整串，因不匹配任何 agent 目录而静默跳过。
pub fn agent_id_prefix(id: &str) -> Option<&str> {
    id.split("::").next().filter(|s| !s.is_empty())
}

/// 原子写 status.json：先写 .tmp 再 rename，避免半写残留。
/// 阶段 0 提供为模块公共 API（供阶段 1 前端运行时态面板/外部调用方消费），
/// 阶段 0 内部写入走 root 注入的 `write_status_at`，故此处暂未在二进制内引用。
#[allow(dead_code)]
pub fn write_status(agent: &str, status: &serde_json::Value) -> Result<(), String> {
    write_status_at(&handoff_root(), agent, status)
}

fn write_status_at(root: &Path, agent: &str, status: &serde_json::Value) -> Result<(), String> {
    let path = root.join(agent).join("status.json");
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(status)
        .map_err(|e| format!("序列化 status.json 失败: {e}"))?;
    std::fs::write(&tmp, content).map_err(|e| format!("写 status.json tmp 失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("落盘 status.json 失败: {e}"))?;
    Ok(())
}

/// 读 status.json；不存在 / 不可解析 → None（调用方按语义处理）
fn read_status_at(root: &Path, agent: &str) -> Option<serde_json::Value> {
    let path = root.join(agent).join("status.json");
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 初始化 agent 工作目录（幂等：目录/文件已存在则补缺不覆盖）。
/// 返回该 agent 目录绝对路径。
#[tauri::command]
pub fn agent_init(agent: String, description: String) -> Result<String, String> {
    init_agent_at(&handoff_root(), &agent, &description)
        .map(|dir| dir.to_string_lossy().to_string())
}

/// 初始化 agent 工作目录（幂等：目录/文件已存在则补缺不覆盖）。
/// 返回该 agent 目录绝对路径。
/// pub(crate)：team.rs 保存新外部 Agent 时联动生成 handoff 目录。
pub(crate) fn init_agent_at(
    root: &Path,
    agent: &str,
    description: &str,
) -> Result<PathBuf, String> {
    validate_agent(agent)?;
    let dir = root.join(agent);
    std::fs::create_dir_all(dir.join("briefs"))
        .map_err(|e| format!("创建 briefs 目录失败: {e}"))?;
    std::fs::create_dir_all(dir.join("projects"))
        .map_err(|e| format!("创建 projects 目录失败: {e}"))?;

    let read_path = dir.join("read.md");
    if !read_path.exists() {
        let content = READ_TEMPLATE
            .replace("{agent_name}", agent)
            .replace("{description}", description);
        std::fs::write(&read_path, content).map_err(|e| format!("写 read.md 失败: {e}"))?;
    }

    let memory_path = dir.join("memory.md");
    if !memory_path.exists() {
        std::fs::write(&memory_path, format!("# {agent} 跨任务记忆\n\n"))
            .map_err(|e| format!("写 memory.md 失败: {e}"))?;
    }

    let status_path = dir.join("status.json");
    if !status_path.exists() {
        let status = serde_json::json!({
            "agent": agent,
            "state": "idle",
            "task_id": "",
            "last_event": null,
            "updated_at": chrono::Local::now().to_rfc3339(),
        });
        write_status_at(root, agent, &status)?;
    }

    Ok(dir)
}

/// 派发任务：写 brief + 更新 status.json 为 in_progress，返回回传契约字符串。
/// 契约含门铃 URL / token / done POST 示例 / 产物路径 / report_path 约定。
#[tauri::command]
pub fn handoff_ensure(agent: String, task_id: String, brief: String) -> Result<String, String> {
    ensure_handoff_at(&handoff_root(), &agent, &task_id, &brief)
}

/// 派发任务（root 注入）：写 brief + status.json 置 in_progress + task_id + dispatched_at，
/// 返回回传契约字符串。幂等：agent 目录未初始化也补建 briefs/projects。
/// pub(crate)：handoff_server 的 /handoff/dispatch 端点复用。
pub(crate) fn ensure_handoff_at(
    root: &Path,
    agent: &str,
    task_id: &str,
    brief: &str,
) -> Result<String, String> {
    validate_agent(agent)?;
    validate_task_id(task_id)?;
    let dir = root.join(agent);
    // 未初始化也补建目录与协议文件（幂等）。read.md 的 description 从 team.toml 取，
    // 让「手改 team.toml + dispatch 上板」路径与配置中心录入殊途同归（冒烟实测断层回归：
    // 旧逻辑 dispatch 只建空骨架，导致首次接手的 agent 没有可读协议文件）。
    if !dir.join("read.md").exists() {
        let desc = crate::commands::config::team::agent_config(agent)
            .ok()
            .flatten()
            .and_then(|m| {
                m.get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| format!("{agent}（职责描述未登记）"));
        init_agent_at(root, agent, &desc)?;
    }
    std::fs::create_dir_all(dir.join("briefs"))
        .map_err(|e| format!("创建 briefs 目录失败: {e}"))?;
    std::fs::create_dir_all(dir.join("projects"))
        .map_err(|e| format!("创建 projects 目录失败: {e}"))?;

    let brief_path = dir.join("briefs").join(format!("{task_id}-brief.md"));
    std::fs::write(&brief_path, brief).map_err(|e| format!("写 brief 失败: {e}"))?;

    // 更新 status.json：保留已有对象字段，置 dispatched + task_id + dispatched_at；
    // 缺失/非对象则从骨架起。
    // 语义纪律：上板 ≠ agent 开始执行。dispatched 仅表示任务已就绪待投递/待确认，
    // 真正的 in_progress 由外部 Agent 第一声拉铃（ready/progress 门铃）触发 ——
    // 否则状态栏会在指令尚未送达时就误亮「执行中」（冒烟实测回归）。
    let mut doc = match read_status_at(root, agent) {
        Some(d) if d.as_object().is_some() => d,
        _ => serde_json::json!({
            "agent": agent,
            "state": "idle",
            "task_id": "",
            "last_event": null
        }),
    };
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("state".to_string(), serde_json::json!("dispatched"));
        obj.insert("task_id".to_string(), serde_json::json!(task_id));
        obj.insert(
            "dispatched_at".to_string(),
            serde_json::json!(chrono::Local::now().to_rfc3339()),
        );
        obj.insert(
            "updated_at".to_string(),
            serde_json::json!(chrono::Local::now().to_rfc3339()),
        );
    }
    write_status_at(root, agent, &doc)?;

    Ok(build_contract(agent, task_id, &dir))
}

/// 查询 agent 当前状态；未初始化返回 {"state":"uninitialized"}
#[tauri::command]
pub fn agent_status(agent: String) -> Result<serde_json::Value, String> {
    Ok(status_at(&handoff_root(), &agent))
}

/// 列出所有已初始化 agent 的运行时状态（供外部 Agent 工作台状态面板消费）。
/// 遍历 handoff_root() 下的 agent 目录，读各自 status.json；
/// 目录不存在 → 空数组；单个目录无 status.json / 不可解析 → 跳过（不 panic）。
/// 返回按 agent 名排序的 status 数组（每个元素含 agent 字段）。
#[tauri::command]
pub fn list_agent_statuses() -> Result<Vec<serde_json::Value>, String> {
    Ok(list_agent_statuses_at(&handoff_root()))
}

fn list_agent_statuses_at(root: &Path) -> Vec<serde_json::Value> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut statuses = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(agent) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(status) = read_status_at(root, agent) {
            statuses.push(status);
        }
    }
    // 稳定输出：按 agent 名排序（保证面板顺序确定、单测可断言）
    statuses.sort_by(|a, b| {
        let an = a["agent"].as_str().unwrap_or_default();
        let bn = b["agent"].as_str().unwrap_or_default();
        an.cmp(bn)
    });
    statuses
}

fn status_at(root: &Path, agent: &str) -> serde_json::Value {
    read_status_at(root, agent).unwrap_or_else(|| serde_json::json!({ "state": "uninitialized" }))
}

/// 列出某 agent 的交付物：briefs/ 下的任务报告（`{task_id}-report.md` 约定）
/// + projects/ 下递归扫描的产物文件。每项含绝对路径 / 文件名 / 相对路径 /
/// kind（report|artifact）/ 字节大小 / 修改时间（rfc3339），按修改时间降序（最新在前）。
/// brief（任务书）是我们下发的内容，不算交付物，不列出。
#[tauri::command]
pub fn list_agent_deliverables(agent: String) -> Result<Vec<serde_json::Value>, String> {
    validate_agent(&agent)?;
    Ok(list_agent_deliverables_at(&handoff_root(), &agent))
}

/// 删除某 agent 的一个交付物文件（供交付物弹窗的删除入口调用）。
/// 安全边界见 delete_agent_deliverable_at：agent 名校验 + rel_path 双重防线。
#[tauri::command]
pub fn delete_agent_deliverable(agent: String, rel_path: String) -> Result<(), String> {
    validate_agent(&agent)?;
    delete_agent_deliverable_at(&handoff_root(), &agent, &rel_path)
}

/// 删除逻辑主体（root 可注入，供单测）：
/// 1. rel_path 逐组件校验——仅接受普通路径组件（拒绝 `..`/`.`/根/盘符前缀），
///    且首组件必须是 briefs 或 projects，与 list_agent_deliverables 的扫描范围
///    严格一致，永远删不到 status.json / memory.md 等核心 handoff 文件；
/// 2. 删除前对目标与 agent 目录做 canonicalize 前缀断言，防符号链接/junction
///    把删除目标引到 agent 目录之外。
fn delete_agent_deliverable_at(root: &Path, agent: &str, rel_path: &str) -> Result<(), String> {
    let rel = std::path::Path::new(rel_path);
    if rel.as_os_str().is_empty() {
        return Err("rel_path 不能为空".to_string());
    }
    let mut comps = rel.components();
    let first_ok = matches!(
        comps.next(),
        Some(std::path::Component::Normal(c)) if c == "briefs" || c == "projects"
    );
    if !first_ok {
        return Err("rel_path 必须位于 briefs/ 或 projects/ 下".to_string());
    }
    for comp in comps {
        if !matches!(comp, std::path::Component::Normal(_)) {
            return Err("rel_path 含非法路径组件".to_string());
        }
    }
    let dir = root.join(agent);
    let path = dir.join(rel);
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| format!("agent 目录不可访问: {e}"))?;
    let canon_path =
        std::fs::canonicalize(&path).map_err(|e| format!("目标不存在或不可访问: {e}"))?;
    if !canon_path.starts_with(&canon_dir) {
        return Err("目标越出 agent 目录，已拒绝删除".to_string());
    }
    if !canon_path.is_file() {
        return Err("目标不是文件".to_string());
    }
    std::fs::remove_file(&path).map_err(|e| format!("删除失败: {e}"))
}

fn list_agent_deliverables_at(root: &Path, agent: &str) -> Vec<serde_json::Value> {
    let dir = root.join(agent);
    let mut out = Vec::new();

    // 任务报告：briefs/*-report.md
    if let Ok(entries) = std::fs::read_dir(dir.join("briefs")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_report = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("-report.md"))
                .unwrap_or(false);
            if is_report {
                push_deliverable(&mut out, &dir, &path, "report");
            }
        }
    }

    // 产物：projects/** 递归
    collect_project_files(&dir.join("projects"), &dir, &mut out);

    // 最新在前；时间相同按相对路径排序保证输出稳定
    out.sort_by(|a, b| {
        let am = a["modified"].as_str().unwrap_or_default();
        let bm = b["modified"].as_str().unwrap_or_default();
        bm.cmp(am).then_with(|| {
            a["rel_path"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["rel_path"].as_str().unwrap_or_default())
        })
    });
    out
}

fn push_deliverable(out: &mut Vec<serde_json::Value>, dir: &Path, path: &Path, kind: &str) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let rel = path
        .strip_prefix(dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| name.to_string());
    let modified = meta
        .modified()
        .map(chrono::DateTime::<chrono::Utc>::from)
        .map(|t| t.with_timezone(&chrono::Local).to_rfc3339())
        .unwrap_or_default();
    out.push(serde_json::json!({
        "path": path.to_string_lossy(),
        "name": name,
        "rel_path": rel,
        "kind": kind,
        "size": meta.len(),
        "modified": modified,
    }));
}

fn collect_project_files(dir: &Path, base: &Path, out: &mut Vec<serde_json::Value>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // 防御性深度上限，避免异常目录树拖垮 UI
            let depth = path
                .strip_prefix(base)
                .map(|p| p.components().count())
                .unwrap_or(0);
            if depth < 8 {
                collect_project_files(&path, base, out);
            }
        } else {
            push_deliverable(out, base, &path, "artifact");
        }
    }
}

/// 构建派发契约字符串（含门铃 URL / token / 上报 CLI 示例 / 产物路径 / report_path 约定）
/// 上报通道唯一化：CLI（nuphus task done/blocked）——curl 已从契约移除（GBK 编码坑 + 错误反馈脆弱）。
/// pub(crate)：handoff_server 派发端点复用（ensure_handoff_at 内部已调用，开放供直接构造）。
pub(crate) fn build_contract(agent: &str, task_id: &str, dir: &Path) -> String {
    let info = nuphus::handoff::doorbell_info();
    let endpoint = format!("http://127.0.0.1:{}/handoff", info.port);
    let event_id = format!("{agent}::{task_id}");
    let projects_dir = dir.join("projects");
    let report_path = dir.join("briefs").join(format!("{task_id}-report.md"));
    // 上报 CLI 自解析：与桌面壳同目录的 nuphus-task.exe（PATH 不可依赖——实测 target\debug 外启动即失联）；
    // 解析不到时退回裸命令名，由 agent 按 read.md 的排障指引自查。
    let cli_cmd = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let sibling = exe.parent()?.join("nuphus-task.exe");
            if sibling.is_file() {
                Some(sibling.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "nuphus task".to_string());

    let mut s = String::new();
    s.push_str("外部 Agent 交接契约（handoff contract）\n");
    s.push_str("====================\n");
    // [1] 身份与凭证：首屏取齐调用所需
    s.push_str(&format!("agent: {agent}\n"));
    s.push_str(&format!("task_id: {task_id}\n"));
    s.push_str(&format!(
        "门铃端点 doorbell endpoint (回调 webhook): {endpoint}\n"
    ));
    if info.available {
        s.push_str(&format!("令牌 token: {}\n", info.token));
    } else {
        s.push_str("令牌 token: 门铃不可用（见 [4] 降级说明）\n");
    }
    // [2] 上报命令：三态齐全、绝对路径、复制改参即可用（先给能直接跑的，再讲规则）
    s.push_str("\n[2] 上报命令（CLI 与桌面主程序是两个东西；命令含绝对路径可整行复制执行；\n     事件 id 已拼好必须原样使用，门铃按其前缀归组更新状态栏）:\n");
    s.push_str(&format!(
        "  progress: {cli_cmd} task progress --id {event_id} --token <令牌见上> --summary \"开工确认：<一句话要点>\"\n",
    ));
    s.push_str(&format!(
        "  done:     {cli_cmd} task done --id {event_id} --token <令牌见上> --summary \"任务完成\" --report \"{report_path}\"\n",
        report_path = report_path.to_string_lossy(),
    ));
    s.push_str(&format!(
        "  blocked:  {cli_cmd} task blocked --id {event_id} --token <令牌见上> --reason \"等待确认\"\n"
    ));
    // [3] 工作区路径约定
    s.push_str("\n[3] 工作区路径（均为绝对路径）:\n");
    s.push_str(&format!(
        "  产物落盘 artifacts: {}\n",
        projects_dir.to_string_lossy()
    ));
    s.push_str(&format!(
        "  报告文件 report: {}（写在 projects/ 内亦可，report_path 指向实际文件即可）\n",
        report_path.to_string_lossy()
    ));
    // [4] 行为纪律：三态语义与时序要求
    s.push_str("\n[4] 上报纪律:\n");
    s.push_str("  - 开工即发一次 progress（summary 一句话说明已理解的任务要点）；\n");
    s.push_str("  - 预计超过 5 分钟的任务，每完成一个阶段续报一次 progress；\n");
    s.push_str("  - 完成才发 done、受阻立即发 blocked，禁止长时间静默空转。\n");
    // [5] 红线：最高频踩坑反例，独立段落确保可见性
    s.push_str("\n[5] 红线:\n");
    s.push_str(&format!(
        "  - 上报只用本契约给出的 CLI（{cli_cmd}）；Nuphus 桌面主程序 nuphus.exe 不是上报 CLI——运行它会拉起新的桌面实例；禁止自行搜索或启动任何 Nuphus 可执行文件。\n",
        cli_cmd = cli_cmd,
    ));
    if !info.available {
        s.push_str("[4a] 降级说明: 门铃不可用时，将结果写入 report 文件并在回复末尾输出 handoff 标记，待 Leader 提取。\n");
    }
    s
}

/// 门铃事件归组：按事件 id 前缀（格式 `{agent}::{task_id}`，以 '::' 分隔）匹配已初始化的 agent 目录，
/// 命中则更新其 status.json 的 state / last_event / updated_at；
/// 未命中或无目录 → 静默跳过（不报错，避免破坏既有门铃流程）。
///
/// 状态映射：progress→in_progress / done→done / blocked→blocked。
/// ready 保留兼容解析（旧 Agent 可能仍上报），但产品流程（read.md/契约）不再要求 ready。
pub fn update_agent_status_from_doorbell(
    id: &str,
    status: &str,
    summary: &str,
    report_path: Option<&str>,
) {
    update_agent_status_from_doorbell_at(&handoff_root(), id, status, summary, report_path);
}

fn update_agent_status_from_doorbell_at(
    root: &Path,
    id: &str,
    status: &str,
    summary: &str,
    report_path: Option<&str>,
) {
    let Some(agent) = agent_id_prefix(id) else {
        return;
    };
    let state = match status {
        // ready/progress 都是外部 Agent 的「开始确认」拉铃 → 才是真正的执行中
        "ready" | "progress" => "in_progress",
        "done" => "done",
        "blocked" => "blocked",
        _ => return, // 未知状态：不落盘（与 push_event 校验语义一致）
    };
    let mut doc = match read_status_at(root, agent) {
        Some(d) => d,
        None => return, // 未初始化 / 无 status.json → 静默跳过
    };
    let obj = match doc.as_object_mut() {
        Some(o) => o,
        None => return, // status.json 不是对象 → 静默跳过，不覆盖
    };
    obj.insert("state".to_string(), serde_json::json!(state));
    obj.insert(
        "last_event".to_string(),
        serde_json::json!({
            "status": status,
            "summary": summary,
            "report_path": report_path,
            "ts": chrono::Local::now().to_rfc3339(),
        }),
    );
    obj.insert(
        "updated_at".to_string(),
        serde_json::json!(chrono::Local::now().to_rfc3339()),
    );
    if let Err(e) = write_status_at(root, agent, &doc) {
        tracing::warn!("[Handoff] 更新 agent[{agent}] status.json 失败（不影响门铃流程）: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 隔离测试根目录：每个用例独立 tmp 子目录，避免污染真实 .nuphus/handoff
    fn tmp_root(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join("nuphus-handoff-test");
        let dir = base.join(format!("{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn test_agent_init_creates_structure_idempotent() {
        let root = tmp_root("init");
        let dir = init_agent_at(&root, "web_agent", "负责网页任务").unwrap();
        assert!(dir.join("briefs").is_dir());
        assert!(dir.join("projects").is_dir());
        let read = std::fs::read_to_string(dir.join("read.md")).unwrap();
        assert!(read.contains("# web_agent 对接协议"));
        assert!(read.contains("负责网页任务"));
        // 门铃语义=交付上报：read.md 用文字描述上报状态（progress/done/blocked），无 JSON 字面示例
        assert!(read.contains("done"));
        assert!(read.contains("progress"));
        assert!(!read.contains("status:\"done\""));
        assert!(!read.contains("status:\"ready\""));
        let memory = std::fs::read_to_string(dir.join("memory.md")).unwrap();
        assert!(memory.starts_with("# web_agent 跨任务记忆"));
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(status["agent"], "web_agent");
        assert_eq!(status["state"], "idle");
        assert_eq!(status["last_event"], serde_json::Value::Null);

        // 幂等：二次调用不覆盖已有 read.md / memory.md / status.json
        let read_before = std::fs::read_to_string(dir.join("read.md")).unwrap();
        init_agent_at(&root, "web_agent", "新的描述").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("read.md")).unwrap(),
            read_before
        );
        assert!(!read_before.contains("新的描述"));
        let status2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(status2["state"], "idle");
    }

    #[test]
    fn test_agent_init_rejects_unsafe_name() {
        let root = tmp_root("unsafe");
        assert!(init_agent_at(&root, "../evil", "x").is_err());
        assert!(init_agent_at(&root, "a/b", "x").is_err());
        assert!(init_agent_at(&root, "", "x").is_err());
        assert!(init_agent_at(&root, "a:b", "x").is_err()); // ':' 是 id '::' 分隔符，禁用于 agent 名
        assert!(!root.join("..").join("evil").exists());
        // 含 '-' 的 agent 名（如 claude-code）合法，与 team.toml 命名对齐
        assert!(init_agent_at(&root, "claude-code", "x").is_ok());
    }

    #[test]
    fn test_handoff_ensure_writes_brief_and_status() {
        let root = tmp_root("ensure");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let contract = ensure_handoff_at(&root, "web_agent", "task-001", "任务：重构页面").unwrap();
        let dir = root.join("web_agent");
        assert!(dir.join("briefs").join("task-001-brief.md").is_file());
        assert_eq!(
            std::fs::read_to_string(dir.join("briefs").join("task-001-brief.md")).unwrap(),
            "任务：重构页面"
        );
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        // 上板≠执行：派发仅置 dispatched（in_progress 由外部 Agent 拉铃触发）
        assert_eq!(status["state"], "dispatched");
        assert_eq!(status["task_id"], "task-001");
        // 契约含门铃 URL / token / CLI 上报示例 / 产物路径（token 不落 status.json）
        assert!(
            contract.contains("http://127.0.0.1:/handoff")
                || contract.contains("http://127.0.0.1:18771/handoff")
        );
        // 上报通道唯一化：CLI 示例（done/blocked），curl 已从契约移除
        assert!(contract.contains("nuphus task done --id web_agent::task-001"));
        assert!(contract.contains("nuphus task blocked --id web_agent::task-001"));
        assert!(!contract.contains("curl"), "契约不得再宣传 curl 上报");
        assert!(!contract.contains("\"status\":\"done\""));
        assert!(contract.contains("web_agent::task-001"));
        let status_str = std::fs::read_to_string(dir.join("status.json")).unwrap();
        assert!(!status_str.contains("token"), "token 不得落 status.json");
    }

    #[test]
    fn test_agent_status_uninitialized() {
        let root = tmp_root("status");
        assert_eq!(status_at(&root, "ghost")["state"], "uninitialized");
    }

    #[test]
    fn test_list_agent_statuses() {
        let root = tmp_root("list");
        // 注入两个已初始化 agent（含 '-' 命名，与 team.toml 对齐）
        init_agent_at(&root, "web_agent", "网页任务").unwrap();
        init_agent_at(&root, "claude-code", "编码任务").unwrap();
        // 派发任务 → task_id + dispatched 落盘（验证列表读到的是 status.json 实际内容）
        ensure_handoff_at(&root, "web_agent", "task-001", "任务：重构页面").unwrap();

        let statuses = list_agent_statuses_at(&root);
        assert_eq!(statuses.len(), 2);
        // 每个元素含 agent 字段；按名排序
        assert_eq!(statuses[0]["agent"], "claude-code");
        assert_eq!(statuses[1]["agent"], "web_agent");
        assert_eq!(statuses[1]["state"], "dispatched");
        assert_eq!(statuses[1]["task_id"], "task-001");

        // 无 status.json 的目录 → 跳过，不影响其余
        std::fs::create_dir_all(root.join("ghost")).unwrap();
        assert_eq!(list_agent_statuses_at(&root).len(), 2);

        // 根目录不存在 → 空数组，不报错
        assert!(list_agent_statuses_at(&tmp_root("nope")).is_empty());
    }

    #[test]
    fn test_doorbell_grouping_updates_status_and_skips_unknown() {
        let root = tmp_root("group");
        init_agent_at(&root, "web_agent", "desc").unwrap();

        // ready 命中 agent 前缀 → state=in_progress（ready/progress 都是「开始确认」拉铃）+ last_event 保留原始值
        update_agent_status_from_doorbell_at(&root, "web_agent::task-001", "ready", "已就位", None);
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("web_agent").join("status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["state"], "in_progress");
        assert_eq!(status["last_event"]["status"], "ready");

        // done 映射 + report_path
        update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-001",
            "done",
            "完成",
            Some("C:/report.md"),
        );
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("web_agent").join("status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["state"], "done");
        assert_eq!(status["last_event"]["report_path"], "C:/report.md");

        // progress → in_progress
        update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-001",
            "progress",
            "一半",
            None,
        );
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("web_agent").join("status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["state"], "in_progress");

        // 未初始化 agent → 静默跳过，不 panic 不创建目录
        update_agent_status_from_doorbell_at(&root, "ghost-task", "done", "x", None);
        assert!(!root.join("ghost-task").exists());

        // 无前缀 / 未知状态 → 静默跳过
        update_agent_status_from_doorbell_at(&root, "-only", "done", "x", None);
        update_agent_status_from_doorbell_at(&root, "web_agent::task-001", "running", "x", None);
    }

    #[test]
    fn test_agent_id_prefix() {
        assert_eq!(agent_id_prefix("web_agent::task-1"), Some("web_agent"));
        assert_eq!(agent_id_prefix("claude-code::0728-01"), Some("claude-code"));
        assert_eq!(agent_id_prefix("web_agent"), Some("web_agent")); // 无 '::' 退化为整串，不匹配目录即跳过
        assert_eq!(agent_id_prefix(""), None);
        assert_eq!(agent_id_prefix("::foo"), None);
    }

    /// 用 std FileTimes 固定 mtime，保证排序断言确定性
    fn set_mtime(path: &Path, secs: u64) {
        let f = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open for set_mtime");
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        f.set_times(std::fs::FileTimes::new().set_modified(t))
            .expect("set_times");
    }

    #[test]
    fn test_list_agent_deliverables_scans_reports_and_projects() {
        let root = tmp_root("deliver");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-001", "任务").unwrap();
        let dir = root.join("web_agent");
        // 报告 + 嵌套产物 + 平铺产物；brief 是任务书不算交付物
        std::fs::write(dir.join("briefs").join("task-001-report.md"), "# 报告").unwrap();
        set_mtime(
            &dir.join("briefs").join("task-001-report.md"),
            1_800_000_000,
        );
        std::fs::create_dir_all(dir.join("projects").join("smoke3")).unwrap();
        std::fs::write(dir.join("projects").join("smoke3").join("smoke3.txt"), "ok").unwrap();
        set_mtime(
            &dir.join("projects").join("smoke3").join("smoke3.txt"),
            1_800_000_100,
        );
        std::fs::write(dir.join("projects").join("out.json"), "{}").unwrap();

        let list = list_agent_deliverables_at(&root, "web_agent");
        assert_eq!(list.len(), 3, "报告 1 + 产物 2，brief 不计入");
        assert!(!list
            .iter()
            .any(|d| d["name"].as_str().unwrap().contains("-brief")));

        // 最新在前：mtime 更晚的产物排第一
        assert_eq!(list[0]["name"], "smoke3.txt");
        assert_eq!(list[0]["kind"], "artifact");
        assert_eq!(list[0]["rel_path"], "projects/smoke3/smoke3.txt");
        let report = list.iter().find(|d| d["kind"] == "report").unwrap();
        assert_eq!(report["name"], "task-001-report.md");
        assert_eq!(report["rel_path"], "briefs/task-001-report.md");
        assert_eq!(report["size"], 8); // "# 报告" 的 UTF-8 字节数
        assert!(report["path"]
            .as_str()
            .unwrap()
            .contains("task-001-report.md"));

        // 未初始化 agent → 空列表；根不存在 → 空列表
        assert!(list_agent_deliverables_at(&root, "ghost").is_empty());
        assert!(list_agent_deliverables_at(&tmp_root("nope-deliver"), "web_agent").is_empty());
    }

    #[test]
    fn test_delete_agent_deliverable_security() {
        let root = tmp_root("deliver-del");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-001", "任务").unwrap();
        let dir = root.join("web_agent");
        std::fs::write(dir.join("briefs").join("task-001-report.md"), "# 报告").unwrap();
        std::fs::create_dir_all(dir.join("projects").join("sub")).unwrap();
        std::fs::write(dir.join("projects").join("sub").join("out.json"), "{}").unwrap();

        // 正常删除：嵌套产物（正斜杠跨平台，Windows/Linux 均解析为分隔符）
        delete_agent_deliverable_at(&root, "web_agent", "projects/sub/out.json")
            .expect("合法产物应可删除");
        assert!(!dir.join("projects").join("sub").join("out.json").exists());

        // 正常删除：报告
        delete_agent_deliverable_at(&root, "web_agent", "briefs/task-001-report.md")
            .expect("报告应可删除");
        assert!(!dir.join("briefs").join("task-001-report.md").exists());

        // 路径穿越拒绝（.. 组件）
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        let err = delete_agent_deliverable_at(&root, "web_agent", "../../secret.txt")
            .expect_err("穿越必须被拒");
        assert!(
            err.contains("非法") || err.contains("briefs"),
            "意外错误: {err}"
        );
        assert!(root.join("secret.txt").exists(), "文件不应被误删");

        // 首组件非 briefs/projects 拒绝 → 核心文件受保护
        // （删除在首组件校验处即被拒，先于任何 fs 操作，所以无需断言文件仍存在）
        let err = delete_agent_deliverable_at(&root, "web_agent", "status.json")
            .expect_err("核心文件必须被拒");
        assert!(err.contains("briefs"));
        let err =
            delete_agent_deliverable_at(&root, "web_agent", "memory.md").expect_err("必须被拒");
        assert!(err.contains("briefs"));

        // 空 rel_path / 不存在的目标 / 目录型目标
        assert!(delete_agent_deliverable_at(&root, "web_agent", "").is_err());
        assert!(delete_agent_deliverable_at(&root, "web_agent", "projects/ghost.json").is_err());
        assert!(
            delete_agent_deliverable_at(&root, "web_agent", "projects/nonexist-dir").is_err(),
            "canonicalize 失败的目录也应报错"
        );

        // 未知 agent（目录不存在）→ 报错而非 panic
        assert!(delete_agent_deliverable_at(&root, "ghost_agent", "briefs/x.md").is_err());

        // 路径分隔符统一解析（正斜杠在 Windows/Linux 均为合法分隔符）
        std::fs::write(dir.join("projects").join("a.txt"), "1").unwrap();
        delete_agent_deliverable_at(&root, "web_agent", "projects/a.txt").unwrap();
    }
}
