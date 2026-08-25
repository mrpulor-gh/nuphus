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

/// read.md 模板 —— 占位符 {agent_name} / {description} 在 agent_init 时替换
/// 门铃语义：仅用于「完成后交付」上报（done）；不要求 ready/就位握手。
const READ_TEMPLATE: &str = r#"# {agent_name} 对接协议

## 你的职责
{description}

## 工作流程（每轮强制）
1. 读 read.md 与 briefs/{task_id}-brief.md 拿本任务
2. 执行 → 产物写 projects/{project}/ → 更新 memory.md
3. 完成后 POST 门铃 status:"done"   （携带 report_path 指向报告文件）

## 回传契约
门铃端点见 Leader 派发时下发的契约字符串（URL / token / report_path 约定）。产物必须写绝对路径。

## 边界/规则
- 只记稳定事实。
- 不靠视觉做常规验收；产物落盘是主证据。
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
    // 未初始化也补建目录（幂等），保证后续 brief/projects 可用
    std::fs::create_dir_all(dir.join("briefs"))
        .map_err(|e| format!("创建 briefs 目录失败: {e}"))?;
    std::fs::create_dir_all(dir.join("projects"))
        .map_err(|e| format!("创建 projects 目录失败: {e}"))?;

    let brief_path = dir.join("briefs").join(format!("{task_id}-brief.md"));
    std::fs::write(&brief_path, brief).map_err(|e| format!("写 brief 失败: {e}"))?;

    // 更新 status.json：保留已有对象字段，置 in_progress + task_id + dispatched_at；
    // 缺失/非对象则从骨架起
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
        obj.insert("state".to_string(), serde_json::json!("in_progress"));
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

/// 构建派发契约字符串（含门铃 URL / token / done POST 示例 / 产物路径 / report_path 约定）
/// pub(crate)：handoff_server 派发端点复用（ensure_handoff_at 内部已调用，开放供直接构造）。
pub(crate) fn build_contract(agent: &str, task_id: &str, dir: &Path) -> String {
    let info = nuphus::handoff::doorbell_info();
    let endpoint = format!("http://127.0.0.1:{}/handoff", info.port);
    let event_id = format!("{agent}::{task_id}");
    let projects_dir = dir.join("projects");
    let report_path = dir.join("briefs").join(format!("{task_id}-report.md"));

    let mut s = String::new();
    s.push_str("外部 Agent 交接契约\n");
    s.push_str("====================\n");
    s.push_str(&format!("agent: {agent}\n"));
    s.push_str(&format!("task_id: {task_id}\n"));
    s.push_str(&format!("门铃端点: {endpoint}\n"));
    if info.available {
        s.push_str(&format!("令牌: {}\n", info.token));
    } else {
        s.push_str("令牌: 门铃不可用（见末尾降级说明）\n");
    }
    s.push_str("\n上报示例（事件 id 必须以 agent 名为前缀，门铃据此归组到 status.json）:\n");
    s.push_str(&format!(
        "  done:  curl -X POST {endpoint} -H \"Content-Type: application/json\" -H \"X-Handoff-Token: {token}\" -d '{{\"id\":\"{event_id}\",\"status\":\"done\",\"summary\":\"任务完成\",\"report_path\":\"{report_path}\"}}'\n",
        token = info.token,
        report_path = report_path.to_string_lossy(),
    ));
    s.push_str(&format!(
        "\n产物落盘（绝对路径）: {}\n",
        projects_dir.to_string_lossy()
    ));
    s.push_str(&format!(
        "报告文件（绝对路径）: {}（报告写在 projects/ 内亦可，report_path 指向实际文件即可）\n",
        report_path.to_string_lossy()
    ));
    if !info.available {
        s.push_str("门铃不可用，请用 report 文件 + 回复标记。\n");
    }
    s
}

/// 门铃事件归组：按 id 前缀（{agent}-{task}）匹配已初始化的 agent 目录，
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
        "progress" => "in_progress",
        "ready" => "ready",
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
        // 门铃语义=交付上报：read.md 不再要求 ready 握手，仅含 done
        assert!(read.contains("status:\"done\""));
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
        assert_eq!(status["state"], "in_progress");
        assert_eq!(status["task_id"], "task-001");
        // 契约含门铃 URL / token / done POST 示例 / 产物路径（token 不落 status.json）
        assert!(
            contract.contains("http://127.0.0.1:/handoff")
                || contract.contains("http://127.0.0.1:18771/handoff")
        );
        assert!(contract.contains("\"status\":\"done\""));
        assert!(!contract.contains("\"status\":\"ready\""));
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
        // 派发任务 → task_id + in_progress 落盘（验证列表读到的是 status.json 实际内容）
        ensure_handoff_at(&root, "web_agent", "task-001", "任务：重构页面").unwrap();

        let statuses = list_agent_statuses_at(&root);
        assert_eq!(statuses.len(), 2);
        // 每个元素含 agent 字段；按名排序
        assert_eq!(statuses[0]["agent"], "claude-code");
        assert_eq!(statuses[1]["agent"], "web_agent");
        assert_eq!(statuses[1]["state"], "in_progress");
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

        // ready 命中 agent 前缀 → state=ready + last_event
        update_agent_status_from_doorbell_at(&root, "web_agent::task-001", "ready", "已就位", None);
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("web_agent").join("status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["state"], "ready");
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
}
