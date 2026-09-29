//! 外部 Agent 工作台 —— 阶段 0（地基）：agent 目录初始化 + 门铃预检 + 交付上报。
//!
//! 目录约定（项目根 = plugin_root().parent()，即 workspace_root()）：
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
- 禁止擅自改动目标仓库的 git 历史：commit / push / tag / reset / checkout / switch / rebase / stash 一律不做；
  代码改动留在工作区，由 Leader 审核后统一提交（仅当本轮 brief 显式授权提交、并写明目标分支时才可提交）。
- 禁止不写 report 直接报 done；禁止报告四段缺项。
- 禁止凭记忆复用上一轮的门铃令牌、URL 或事件 id——一切以本轮契约原文为准。
- 禁止长时间静默空转：受阻立即 blocked 并说明原因。
"#;

/// handoff 根目录：{项目根}/.nuphus/handoff —— **唯一权威推导**。
///
/// 派生规则纯函数见 `nuphus::utils::handoff_root_from`：
/// 开发机（plugin 根在源码检出内）→ `<repo>/.nuphus/handoff`（零迁移）；
/// 发布版（plugin 根落在 nuphus_data_dir() 之下）→ `nuphus_data_dir()/handoff`。
pub fn handoff_root() -> PathBuf {
    nuphus::utils::handoff_root_from(
        &nuphus::utils::plugin_root(),
        &nuphus::utils::nuphus_data_dir(),
    )
}

/// 旧 handoff 根 → 新根的一次性迁移汇总（**agent 目录级**粒度）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HandoffMigrationReport {
    /// 从旧根拷入新根的 agent 名（按来源顺序）
    pub copied: Vec<String>,
    /// 新根已有同名 agent 目录 → 保留新根、弃用旧副本的 agent 名
    pub skipped: Vec<String>,
    /// (agent 名, 错误信息)；单个 agent 失败不影响其余迁移
    pub failed: Vec<(String, String)>,
}

impl HandoffMigrationReport {
    /// 无任何实质动作（没迁、没跳、没失败）。
    pub fn is_empty(&self) -> bool {
        self.copied.is_empty() && self.skipped.is_empty() && self.failed.is_empty()
    }
}

/// 迁移核心（root 注入，可单测）：把多个候选旧根下的 agent 目录合并进 target。
///
/// 粒度是 **agent 目录级**：target 已有该 agent 目录即跳过——绝不能整根跳过，
/// 否则首次启动建出新根后，迁移逻辑永远不会再触发。
/// 同一 agent 在多个来源都出现时，按 `sources` 顺序首个胜出。
/// 来源目录不存在 / 不可读 → 视为无此来源，不报错。
pub fn migrate_handoff_agents_into(target: &Path, sources: &[PathBuf]) -> HandoffMigrationReport {
    let mut report = HandoffMigrationReport::default();
    for source in sources {
        let Ok(entries) = std::fs::read_dir(source) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue; // 只迁 agent 目录，散落文件不处理
            }
            let Some(agent) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let dest = target.join(&agent);
            if dest.exists() {
                report.skipped.push(agent);
                continue;
            }
            // 第一次真实拷贝时才确保 target 存在（无内容可迁不凭空建新根）
            if let Err(e) = std::fs::create_dir_all(target) {
                report
                    .failed
                    .push((agent, format!("创建 handoff 根 {target:?} 失败: {e}")));
                continue;
            }
            match copy_dir_recursive(&path, &dest) {
                Ok(()) => report.copied.push(agent),
                Err(e) => {
                    let msg = format!("拷贝 {agent} 目录失败: {e}");
                    report.failed.push((agent, msg));
                }
            }
        }
    }
    report
}

/// 递归拷贝目录内容（不存在目标层级时逐级创建）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target_path)?;
        } else {
            std::fs::copy(entry.path(), &target_path)?;
        }
    }
    Ok(())
}

/// 启动期接线：候选旧根 → 新 handoff 根的一次性合并迁移（详见 `migrate_handoff_agents_into`）。
///
/// 旧根候选覆盖历史全部布局：
/// ① `<exe_dir>/.nuphus/handoff`（exe 同级）
/// ② `<cwd>/.nuphus/handoff`（发布版 cwd 常 ≠ exe_dir）
/// ③ `nuphus_data_dir()/.nuphus/handoff`（历史版本嵌进 data_dir 的嵌套坑）
///
/// 必须在 `reset_all_statuses_at_startup()` **之前**调用：后者按新根清零
/// status.json，先迁移才能让迁过来的真实状态参与本轮生命周期判定。
pub fn migrate_legacy_handoff_roots() -> HandoffMigrationReport {
    let data_dir = nuphus::utils::nuphus_data_dir();
    let target = nuphus::utils::handoff_root_from(&nuphus::utils::plugin_root(), &data_dir);

    let mut sources: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            sources.push(dir.join(".nuphus").join("handoff"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        sources.push(cwd.join(".nuphus").join("handoff"));
    }
    sources.push(data_dir.join(".nuphus").join("handoff"));

    // 去重（cwd == exe_dir 时同一路径出现两次）+ 排除目标自身（自拷贝）
    let mut uniq: Vec<PathBuf> = Vec::new();
    for s in sources {
        if s != target && !uniq.contains(&s) {
            uniq.push(s);
        }
    }

    let report = migrate_handoff_agents_into(&target, &uniq);
    if !report.is_empty() {
        tracing::info!(
            "[Handoff] 旧根合并迁移: target={:?}, copied={:?}, skipped={:?}, failed={:?}",
            target,
            report.copied,
            report.skipped,
            report.failed
        );
    }
    report
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

/// 从门铃事件 id 解析事件自带的 task_id（id 约定 `{agent}::{task_id}`）。
/// 缺 `::` 分隔、task_id 段为空 → None（旧式纯 id 无从比对归属，见
/// [`update_agent_status_from_doorbell_at`] 的一致性闸处置）。
fn event_task_id(id: &str) -> Option<&str> {
    id.split("::")
        .nth(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// status.json 写锁：把同进程内所有 agent 的 status.json 写临界区串行化。
/// 门铃事件（handoff_server 的 axum handler）与派发编排（ext_agent）分属不同线程，
/// 共用固定 tmp 名时并发写会互相覆盖、甚至 rename 找不到 tmp 而丢整次写入。
static STATUS_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// status.json 临时文件序号（配合 pid）：让并发写各写各的 tmp，同进程/跨进程都不撞名。
static STATUS_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 取 status.json 写锁。锁中毒也照常放行——临界区内只做「序列化 + 写文件 + rename」，
/// 没有需要回滚的进程内中间态， poisoned 仅说明某线程曾在临界区内 panic。
///
/// 注意：持锁期间**禁止**再调用 [`write_status_at`]（std Mutex 不可重入，会自死锁）；
/// 已在临界区内时改用 [`write_status_locked`]。
fn status_write_lock() -> std::sync::MutexGuard<'static, ()> {
    STATUS_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 原子写 status.json：先写 tmp 再 rename，避免半写残留与并发互踩。
/// 阶段 0 提供为模块公共 API（供阶段 1 前端运行时态面板/外部调用方消费），
/// 阶段 0 内部写入走 root 注入的 `write_status_at`，故此处暂未在二进制内引用。
#[allow(dead_code)]
pub fn write_status(agent: &str, status: &serde_json::Value) -> Result<(), String> {
    write_status_at(&handoff_root(), agent, status)
}

/// 原子写 status.json（自带写锁）。tmp 名带 pid + 自增序号：同一 agent 目录下多个
/// 并发写各持一份 tmp，不会出现「对方已 rename 走、自己 rename 落空」的丢写。
fn write_status_at(root: &Path, agent: &str, status: &serde_json::Value) -> Result<(), String> {
    let _guard = status_write_lock();
    write_status_locked(root, agent, status)
}

/// [`write_status_at`] 的锁内变体：调用方已通过 [`status_write_lock`] 持有写锁时用它，
/// 以便把「读—改—写」整段纳入同一临界区（否则锁只保护 rename 前的一瞬，仍会丢更新）。
/// tmp 名刻意不沿用仓库里常见的固定 `<file>.json.tmp`：status.json 是同进程内唯一被
/// 两类线程（门铃 HTTP handler / 派发编排）并发读改写 + 跨进程也可能并写的文件，
/// 固定 tmp 名会互相覆盖、或对方已 rename 走导致自己 rename 落空而整次丢写。
fn write_status_locked(root: &Path, agent: &str, status: &serde_json::Value) -> Result<(), String> {
    let path = root.join(agent).join("status.json");
    let content = serde_json::to_string_pretty(status)
        .map_err(|e| format!("序列化 status.json 失败: {e}"))?;
    let tmp = path.with_file_name(format!(
        "status.json.{}.{}.tmp",
        std::process::id(),
        STATUS_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&tmp, content).map_err(|e| format!("写 status.json tmp 失败: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp); // rename 失败不留垃圾 tmp
        return Err(format!("落盘 status.json 失败: {e}"));
    }
    Ok(())
}

/// 读 status.json；不存在 / 不可解析 → None（调用方按语义处理）。
/// pub(crate)：ext_agent 派发前的在途闸预检要读它——闸门判据必须看真实看板，
/// 不能凭调用方（LLM）自述「我以为 agent 空闲」。
pub(crate) fn read_status_at(root: &Path, agent: &str) -> Option<serde_json::Value> {
    let path = root.join(agent).join("status.json");
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 在途闸（派发前预检，纯判定）：板上 state 为 `dispatched`/`in_progress` **且**板上
/// task_id 与本轮新任务不同 → 返回阻止原因（Some）；否则放行（None）。
///
/// 为什么建这道闸：`agent_dispatch` 原本不看在板状态就直接上板——agent 还在跑 task_A
/// 时派 task_B，两个任务会在同一个 TUI 里交错执行（键入互相插入、门铃归属混乱；
/// task_id 一致性闸只能保状态不串，保不了终端执行不互相干扰）。
/// 放行边界（保守，不挡正当路径）：
/// - 同一 task_id 的重派/续派放行——失败重试、契约刷新都走这条路（brief 覆盖是幂等语义）；
/// - 板上 task_id 为空（骨架/异常态）放行——新上板会顺带修复看板；
/// - doc 缺失/不可解析放行——无法判定在途时不挡路，由上板路径自愈。
pub(crate) fn in_flight_block_reason(
    doc: Option<&serde_json::Value>,
    new_task_id: &str,
) -> Option<String> {
    let doc = doc?;
    let obj = doc.as_object()?;
    let state = obj
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if state != "dispatched" && state != "in_progress" {
        return None;
    }
    let board = obj
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if board == new_task_id {
        return None; // 同 task_id 重派/续派放行
    }
    Some(format!(
        "agent 有在途任务未到终态（task_id={board}，state={state}）——拒绝派发 {new_task_id}，\
         避免两个任务在同一终端交错执行。\n\
         出口：① 等在途任务的门铃终态（done/blocked）到达后再派；\n\
         ② 若确认在途任务已死（终端空闲、无执行痕迹），用原 task_id「{board}」重新派发刷新看板\
         （新 brief 覆盖旧板，agent 按新契约上报）。\n\
         禁令：禁止手改 status.json 绕闸。"
    ))
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
pub fn handoff_ensure(
    agent: String,
    task_id: String,
    brief: String,
    workspace: Option<String>,
) -> Result<String, String> {
    ensure_handoff_at(
        &handoff_root(),
        &agent,
        &task_id,
        &brief,
        workspace.as_deref(),
    )
}

// ── 派发审计：目标工作区的 git HEAD 基线 ──────────────────────────────────────

/// 完工审计结论：派发基线 HEAD 与完工时 HEAD 不一致，说明本轮工作区里出现了提交
/// （外部 Agent 自己提交，或另有写手动了同一个 worktree）。
/// `changed` 为真时由门铃路径提示 Leader 先复核再验收。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadAudit {
    pub workspace: String,
    pub base_head: String,
    pub head: String,
    pub branch: String,
    pub changed: bool,
}

/// 在 `dir` 上执行一条 git 查询（直接 spawn，不经 shell）。
/// 非 git 仓库 / git 未安装 / 命令失败 / 空输出 → None，调用方一律按「无基线」降级。
fn git_query(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 工作区当前 HEAD（sha, 分支名）。非 git 仓库 / git 不可用 → None。
fn git_head(dir: &Path) -> Option<(String, String)> {
    let head = git_query(dir, &["rev-parse", "HEAD"])?;
    let branch =
        git_query(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "-".to_string());
    Some((head, branch))
}

/// 由 status.json 算完工审计结论：`workspace` 与 `base_head` 必须同时存在且可解析，
/// 且该工作区当前仍是可读的 git 仓库。任一不满足 → None（无基线，不做审计）。
fn head_audit(status: &serde_json::Value) -> Option<HeadAudit> {
    let workspace = status.get("workspace")?.as_str()?.to_string();
    let base_head = status.get("base_head")?.as_str()?.to_string();
    let (head, branch) = git_head(Path::new(&workspace))?;
    Some(HeadAudit {
        changed: head != base_head,
        workspace,
        base_head,
        head,
        branch,
    })
}

/// 短 sha（前 8 位），人读提示用。
pub(crate) fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// 派发任务（root 注入）：写 brief + status.json 置 in_progress + task_id + dispatched_at，
/// 返回回传契约字符串。幂等：agent 目录未初始化也补建 briefs/projects。
/// pub(crate)：handoff_server 的 /handoff/dispatch 端点复用。
/// `workspace`：可选。外部 Agent 被授权改动的目标工作区（绝对路径）。给出时在 status.json
/// 记下 `workspace` 与派发时刻的 git HEAD（`base_head`/`base_branch`），供完工时比对。
/// 「外部 Agent 擅自提交」无法从应用侧强制阻止（它是独立进程、git 凭据不受本应用约束），
/// 但可以把「事后翻 git log 才发现」变成「完工即知」。
pub(crate) fn ensure_handoff_at(
    root: &Path,
    agent: &str,
    task_id: &str,
    brief: &str,
    workspace: Option<&str>,
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
        // 新一轮派发清掉上一轮的失败痕迹：否则上一轮 error 的 error_reason 会留在
        // 一个 state=dispatched 的新 record 旁边，读文件的人无法判断现在到底失没失败
        // （与下面「清掉上一轮审计结论」同一纪律）。
        obj.remove("error_reason");
        obj.remove("error_task_id");
        obj.remove("error_at");
        // 派发基线：目标工作区 + 当下 HEAD。非 git 目录 / git 不可用 / 未声明 workspace
        // → 不记（宁可缺失，不可错记）；新一轮派发清掉上一轮的审计结论。
        match workspace.map(str::trim).filter(|s| !s.is_empty()) {
            Some(ws) => {
                let (head, branch) = match git_head(Path::new(ws)) {
                    Some((h, b)) => (serde_json::json!(h), serde_json::json!(b)),
                    None => (serde_json::Value::Null, serde_json::Value::Null),
                };
                obj.insert("workspace".to_string(), serde_json::json!(ws));
                obj.insert("base_head".to_string(), head);
                obj.insert("base_branch".to_string(), branch);
                obj.remove("head_check");
            }
            None => {
                obj.remove("workspace");
                obj.remove("base_head");
                obj.remove("base_branch");
                obj.remove("head_check");
            }
        }
    }
    write_status_at(root, agent, &doc)?;

    Ok(build_contract(agent, task_id, &dir))
}

/// 派发失败 → 给该 agent 落 error 态，打破「state 永久停在 dispatched」的幽灵
/// （前端 `ExternalAgentsStatusBar` 早把 `error` 映射成 is-error，只差没人写它）。
///
/// `reason` 必须人类可读且能指认失败环节，让用户判断是「进程没起来 / 窗口没捕获 /
/// 输入没进去」中的哪一种（前端暂不展示原因，人打开 status.json 即可定位）。
///
/// 语义纪律：
/// - 上板**前**失败（agent 未登记、brief 写不进、workspace 不是目录）同样要落 —— 否则
///   状态栏还停在上一轮 state，用户看不到本轮派发根本没成功；
/// - 只动 state/error 相关字段，绝不覆盖别的轮次：`task_id` 仅在「当前无在途任务」
///   （空/缺失）时补记本次失败的任务，失败任务自身记在 `error_task_id` 里；
/// - 落 error 本身失败只 warn，绝不因此改变派发的错误返回。
pub(crate) fn mark_agent_error_at(root: &Path, agent: &str, task_id: Option<&str>, reason: &str) {
    if let Err(e) = validate_agent(agent) {
        tracing::warn!("[Handoff] 落 error 态被拒：agent 名非法（{e}）");
        return;
    }
    // 目录可能还不存在（agent 从未初始化/未登记也得能看到失败），补建后再写
    if let Err(e) = std::fs::create_dir_all(root.join(agent)) {
        tracing::warn!("[Handoff] 落 error 态失败：无法创建 agent[{agent}] 目录: {e}");
        return;
    }
    let task = task_id.map(str::trim).filter(|s| !s.is_empty());
    let _guard = status_write_lock();
    let mut doc = match read_status_at(root, agent) {
        Some(d) if d.as_object().is_some() => d,
        _ => serde_json::json!({
            "agent": agent,
            "state": "idle",
            "task_id": "",
            "last_event": null
        }),
    };
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    let has_task = obj
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    if !has_task {
        // 无在途任务才补 task_id：有在途任务时它是另一轮的归属，不能在本轮失败里被改写
        obj.insert("task_id".to_string(), serde_json::json!(task.unwrap_or("")));
    }
    let now = chrono::Local::now().to_rfc3339();
    obj.insert("state".to_string(), serde_json::json!("error"));
    obj.insert("error_reason".to_string(), serde_json::json!(reason));
    obj.insert("error_task_id".to_string(), serde_json::json!(task));
    obj.insert("error_at".to_string(), serde_json::json!(now.clone()));
    obj.insert("updated_at".to_string(), serde_json::json!(now));
    if let Err(e) = write_status_locked(root, agent, &doc) {
        tracing::warn!("[Handoff] 落 agent[{agent}] error 态失败: {e}");
    }
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
///
/// 每项附 `hidden`（用户是否已从列表栏移出）——显示态由后端共享集标注
/// （应用生命周期内保持、重启即净），前端不再自持移出状态。
#[tauri::command]
pub fn list_agent_statuses(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let hidden = nuphus::state::SignalState::hidden_ext_agents(&state.signals);
    let mut list = list_agent_statuses_at(&handoff_root());
    for item in list.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let agent = obj
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        obj.insert(
            "hidden".to_string(),
            serde_json::json!(hidden.contains_key(&agent)),
        );
    }
    Ok(list)
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

/// 用户把外部 Agent 从列表栏移出 → ①显示态写入后端共享集（应用生命周期内保持，
/// 重启即净——前端 UI 显示跟随应用生命周期，遵循用户选择）；②往「下一条提示」
/// 注入位写一句，由下一个轮次边界带进 agent 上下文（只进上下文、界面不显示）。
///
/// 目的：让 agent 知道「这个外部 Agent 已被用户暂时移出，后续需用户显式指定才可调用」，
/// 避免它继续自动派发/重试。显示层操作，不动配置与 team.toml。
#[tauri::command]
pub fn notify_ext_agent_removed(
    agent: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let agent = agent.trim().to_string();
    validate_agent(&agent)?;
    // 显示态归应用生命周期：写入进程内共享集（src/state.rs）。
    // 撤销只有两条显式路径：时刻更晚的新门铃活动 / 配置中心保存。
    nuphus::state::SignalState::hide_ext_agent(
        &state.signals,
        &agent,
        chrono::Utc::now().timestamp_millis(),
    );
    nuphus::state::SignalState::push_notice(
        &state.signals,
        format!("[外部 Agent 列表栏] 用户已把外部 Agent「{agent}」移出：后续需用户显式指定才可调用，不要自动派发或重试它。"),
    );
    Ok(())
}

/// 门铃事件到达后的显示态撤销：事件时刻**严格晚于**移出时刻 → 撤销「从列表移出」
/// （新一轮真实调用 = 用户的新选择，条目应回到列表栏）。无 status / 无 last_event /
/// 未移出 → 无操作。纯判定在 [`nuphus::state::SignalState::event_revives_hidden`]。
pub fn revive_hidden_ext_agent_on_event(signals: &nuphus::state::SharedSignals, id: &str) {
    let Some(agent) = agent_id_prefix(id) else {
        return;
    };
    let Some(hidden_since) = nuphus::state::SignalState::hidden_ext_agent_since(signals, agent)
    else {
        return;
    };
    let event_ts = read_status_at(&handoff_root(), agent)
        .and_then(|d| d.get("last_event")?.get("ts")?.as_str().map(str::to_string))
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.timestamp_millis());
    if let Some(ts) = event_ts {
        if nuphus::state::SignalState::event_revives_hidden(hidden_since, ts) {
            nuphus::state::SignalState::unhide_ext_agent(signals, agent);
        }
    }
}

/// 注销外部 Agent 时清运行时态：**只删 status.json**（briefs/projects/memory.md/
/// read.md 全留——追溯链不断）。状态栏唯一边据即 status.json，删之即从列表消失
/// 且重启不复活（reset 只重置 status.json，文件没了无从重置）。
/// 目录/文件不存在 → Ok(())（幂等）。
pub fn clear_runtime_state_at(root: &Path, agent: &str) -> Result<(), String> {
    validate_agent(agent)?;
    let path = root.join(agent).join("status.json");
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除 status.json 失败: {e}")),
    }
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
    // 上报 CLI 自解析：桌面壳自身即上报入口（桌面主程序内置 `task` 子命令，方案 A）。
    // 不再做 sibling 探测 / PATH 回退 / 平台文件名猜测——旧旁车 bin nuphus-task 从未
    // 进入发布产物，安装版与 npm 用户的机器上它并不存在，探测必败、回退不可执行。
    // 路径加双引号防空格撕裂；Windows 反斜杠原样保留：外部 Agent 在 PowerShell/cmd
    // 里整行复制执行，反斜杠无需转义。
    let cli_cmd = match std::env::current_exe() {
        Ok(exe) => format!("\"{}\" task", exe.display()),
        // current_exe() 仅在可执行文件运行中被删除等极端场景失败；此时退化为命令名
        // 本体以保持契约命令形态完整（无任何探测逻辑）。
        Err(_) => "\"nuphus\" task".to_string(),
    };

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
    s.push_str("\n[2] 上报命令（上报 CLI 即桌面主程序自带的 task 子命令，无需另装任何工具；命令含绝对路径可整行复制执行；\n     事件 id 已拼好必须原样使用，门铃按其前缀归组更新状态栏）:\n");
    s.push_str(&format!(
        "  progress: {cli_cmd} progress --id {event_id} --token <令牌见上> --summary \"开工确认：<一句话要点>\"\n",
    ));
    s.push_str(&format!(
        "  done:     {cli_cmd} done --id {event_id} --token <令牌见上> --summary \"任务完成\" --report \"{report_path}\"\n",
        report_path = report_path.to_string_lossy(),
    ));
    s.push_str(&format!(
        "  blocked:  {cli_cmd} blocked --id {event_id} --token <令牌见上> --reason \"等待确认\"\n"
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
        "  - 上报只用本契约给出的 CLI（{cli_cmd}）——该命令即桌面主程序的 task 子命令，不会拉起桌面窗口；禁止裸跑桌面主程序（无 task 子命令会拉起新桌面实例），也禁止自行搜索或启动其它 Nuphus 可执行文件。\n",
        cli_cmd = cli_cmd,
    ));
    s.push_str(
        "  - 禁止擅自改动目标仓库的 git 历史（commit / push / tag / reset / checkout / switch / rebase / stash）：改动留在工作区，由 Leader 审核后统一提交。仅当本 brief 显式授权提交、并写明目标分支时才可执行。\n",
    );
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
///
/// ## task_id 一致性闸
/// status.json 的 `task_id` 是「当前在板上的是哪一轮任务」，由 [`ensure_handoff_at`] 在上板时写入。
/// 上一轮的迟到事件（task_A 的 progress/done 晚于 task_B 上板到达）若直接落盘，就会把 task_A 的
/// 结论写到 task_B 的 state/task_id 上，并拿 task_B 的 `base_head` 基线跑完工审计——命中后
/// 经 handoff_server 推出「本轮出现了不是你派发的提交」的误报，连唤醒指令都指错任务。
/// 因此：事件自带 task_id 与板上 task_id **不一致时**只追加 `last_event`（保留事件原值）+
/// warn，不动 state/task_id、不跑 head_audit、不返回审计结论。事件本身绝不丢弃——done/blocked
/// 是唯一唤醒源，唤醒只认事件 status 与 status.json 无关，保守落盘不影响它进队。
pub fn update_agent_status_from_doorbell(
    id: &str,
    status: &str,
    summary: &str,
    report_path: Option<&str>,
) -> Option<HeadAudit> {
    update_agent_status_from_doorbell_at(&handoff_root(), id, status, summary, report_path)
}

/**
 * Once a task is assigned, only matching reports may update its state or trigger an audit.
 * Unassigned agents still accept legacy events; unmatched reports remain in last_event.
 */
fn update_agent_status_from_doorbell_at(
    root: &Path,
    id: &str,
    status: &str,
    summary: &str,
    report_path: Option<&str>,
) -> Option<HeadAudit> {
    let agent = agent_id_prefix(id)?;
    let state = match status {
        // ready/progress 都是外部 Agent 的「开始确认」拉铃 → 才是真正的执行中
        "ready" | "progress" => "in_progress",
        "done" => "done",
        "blocked" => "blocked",
        _ => return None, // 未知状态：不落盘（与 push_event 校验语义一致）
    };
    // 整段「读—判—改—写」在同一把写锁内完成：门铃 handler 与派发编排分属不同线程，
    // 只锁写不锁读会先读到旧 doc、再把旧 task_id/基线写回去（丢更新）。
    // 代价是完工审计的 git 查询也在锁内（done/blocked 才走，事件低频，可接受）。
    let _guard = status_write_lock();
    // 未初始化 / 无 status.json / 不是对象 → 静默跳过，不覆盖
    let mut doc = read_status_at(root, agent)?;
    doc.as_object()?;

    let event_task = event_task_id(id);
    let board_task = doc
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let stale = board_task.is_some() && event_task != board_task;
    if stale {
        // 跨任务迟到事件：事件是真的（可能是上一轮的真实完工），所以 last_event 照记并
        // 保留事件自带 task_id 原值供追溯；但 state/task_id 属于板上那一轮，不能改。
        tracing::warn!(
            "[Handoff] 忽略跨任务迟到事件的 state 变更：agent[{agent}] 板上 task_id={}，事件 {} 属于 task_id={}（只记 last_event，不改 state/task_id、不跑完工审计）",
            board_task.unwrap_or_default(),
            id,
            event_task.unwrap_or_default()
        );
        let obj = doc.as_object_mut()?;
        obj.insert(
            "last_event".to_string(),
            serde_json::json!({
                "status": status,
                "summary": summary,
                "report_path": report_path,
                "task_id": event_task,
                "ts": chrono::Local::now().to_rfc3339(),
            }),
        );
        obj.insert(
            "updated_at".to_string(),
            serde_json::json!(chrono::Local::now().to_rfc3339()),
        );
        if let Err(e) = write_status_locked(root, agent, &doc) {
            tracing::warn!("[Handoff] 追加 agent[{agent}] last_event 失败（不影响门铃流程）: {e}");
        }
        return None;
    }

    // 完工审计：只在 done/blocked 上做（progress 只是开工确认，此时工作区还没动）。
    let audit = if matches!(status, "done" | "blocked") {
        head_audit(&doc)
    } else {
        None
    };
    let obj = doc.as_object_mut()?;
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
    if let Some(a) = &audit {
        obj.insert(
            "head_check".to_string(),
            serde_json::json!({
                "changed": a.changed,
                "workspace": a.workspace,
                "base_head": a.base_head,
                "head": a.head,
                "branch": a.branch,
                "ts": chrono::Local::now().to_rfc3339(),
            }),
        );
    }
    obj.insert(
        "updated_at".to_string(),
        serde_json::json!(chrono::Local::now().to_rfc3339()),
    );
    if let Err(e) = write_status_locked(root, agent, &doc) {
        tracing::warn!("[Handoff] 更新 agent[{agent}] status.json 失败（不影响门铃流程）: {e}");
    }
    audit
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

    fn doc(state: &str, task_id: &str) -> serde_json::Value {
        serde_json::json!({"agent": "a", "state": state, "task_id": task_id, "last_event": null})
    }

    #[test]
    fn test_in_flight_gate_blocks_different_task() {
        let d = doc("in_progress", "task-a");
        let reason = in_flight_block_reason(Some(&d), "task-b").expect("在途不同任务必须阻止");
        assert!(reason.contains("task-a"), "原因须指认在途任务: {reason}");
        assert!(reason.contains("task-b"), "原因须指认被拒任务: {reason}");
        // dispatched 同样阻止（上板未投/未确认也是在途）
        assert!(in_flight_block_reason(Some(&doc("dispatched", "task-a")), "task-b").is_some());
    }

    #[test]
    fn test_in_flight_gate_allows_legit_paths() {
        // 同 task_id 重派/续派放行（失败重试与契约刷新的唯一出口）
        assert!(in_flight_block_reason(Some(&doc("in_progress", "task-a")), "task-a").is_none());
        // 终态后放行
        for s in ["done", "blocked", "error", "idle"] {
            assert!(
                in_flight_block_reason(Some(&doc(s, "task-a")), "task-b").is_none(),
                "终态 {s} 不应阻止新派发"
            );
        }
        // 板上 task_id 为空（骨架/异常态）放行——新上板顺带修复看板
        assert!(in_flight_block_reason(Some(&doc("in_progress", "")), "task-b").is_none());
        assert!(in_flight_block_reason(Some(&doc("in_progress", "  ")), "task-b").is_none());
        // 无 status / 非对象 doc：无法判定在途，不挡路
        assert!(in_flight_block_reason(None, "task-b").is_none());
        assert!(in_flight_block_reason(Some(&serde_json::json!("oops")), "task-b").is_none());
    }

    /// 注销清运行时态：status.json 删光，briefs/report/memory/read.md 全留
    /// （L0 配置与 L1 运行时态联动注销，追溯链不断）；幂等。
    #[test]
    fn test_clear_runtime_state_keeps_traceability() {
        let root = tmp_root("clear-runtime");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-001", "brief", None).unwrap();
        assert!(root.join("web_agent").join("status.json").exists());

        clear_runtime_state_at(&root, "web_agent").unwrap();
        assert!(!root.join("web_agent").join("status.json").exists());
        // 追溯链资产一动没动
        assert!(root
            .join("web_agent")
            .join("briefs")
            .join("task-001-brief.md")
            .exists());
        assert!(root.join("web_agent").join("read.md").exists());

        // 幂等：再删/删不存在的 agent 都不报错
        clear_runtime_state_at(&root, "web_agent").unwrap();
        clear_runtime_state_at(&root, "no_such_agent").unwrap();
    }

    /// 跑一条 git 命令；成功返回 trim 后的 stdout。环境无 git / 命令失败 → None。
    fn git_run(dir: &Path, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// 建一个临时 git 仓库并提交一次；返回 (工作区绝对路径, 首个 commit sha)。
    /// 环境没有 git → None（调用方跳过该用例并在输出里说明，不伪造通过）。
    fn tmp_git_workspace(name: &str) -> Option<(String, String)> {
        let repo = tmp_root(name);
        std::fs::create_dir_all(&repo).ok()?;
        git_run(&repo, &["init", "-q"])?;
        git_run(&repo, &["config", "user.email", "t@example.com"])?;
        git_run(&repo, &["config", "user.name", "t"])?;
        git_run(&repo, &["commit", "-q", "--allow-empty", "-m", "init"])?;
        let head = git_run(&repo, &["rev-parse", "HEAD"])?;
        Some((repo.to_string_lossy().to_string(), head))
    }

    fn status_json(root: &Path, agent: &str) -> serde_json::Value {
        let p = root.join(agent).join("status.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// 派发声明 workspace → status.json 记基线；完工时 HEAD 变了 → 审计命中。
    #[test]
    fn test_dispatch_baseline_detects_undelegated_commit() {
        let root = tmp_root("audit");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let Some((ws, base)) = tmp_git_workspace("audit-repo") else {
            eprintln!("跳过：环境无 git");
            return;
        };

        ensure_handoff_at(&root, "web_agent", "task-001", "任务", Some(&ws)).unwrap();
        let status = status_json(&root, "web_agent");
        assert_eq!(status["workspace"], serde_json::json!(ws));
        assert_eq!(status["base_head"], serde_json::json!(base));
        assert!(status["base_branch"].as_str().is_some());

        // 开工确认不是完工：progress 不做审计
        assert!(update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-001",
            "progress",
            "开工",
            None
        )
        .is_none());

        // 模拟「外部 Agent 自己提交了一个 commit」
        assert!(git_run(
            Path::new(&ws),
            &["commit", "-q", "--allow-empty", "-m", "agent self commit"]
        )
        .is_some());
        let audit = update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-001",
            "done",
            "完成",
            None,
        )
        .expect("声明过 workspace 的完工必须产出审计结论");
        assert!(audit.changed, "HEAD 变了必须报 changed");
        assert_ne!(audit.head, audit.base_head);
        assert_eq!(audit.workspace, ws);

        let status = status_json(&root, "web_agent");
        assert_eq!(status["head_check"]["changed"], serde_json::json!(true));
        assert_eq!(status["head_check"]["base_head"], serde_json::json!(base));
    }

    /// 未声明 workspace / 非 git 目录 → 不审计（宁缺勿错），不产生 head_check。
    #[test]
    fn test_dispatch_audit_degrades_without_baseline() {
        let root = tmp_root("audit-degrade");
        init_agent_at(&root, "web_agent", "desc").unwrap();

        // ① 未声明 workspace
        ensure_handoff_at(&root, "web_agent", "task-001", "任务", None).unwrap();
        let status = status_json(&root, "web_agent");
        assert!(status.get("workspace").is_none());
        assert!(update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-001",
            "done",
            "完成",
            None
        )
        .is_none());
        assert!(status_json(&root, "web_agent").get("head_check").is_none());

        // ② 声明了 workspace，但目标不是 git 仓库 → 记路径、不记基线、不审计
        let plain = tmp_root("audit-plain");
        std::fs::create_dir_all(&plain).unwrap();
        let ws = plain.to_string_lossy().to_string();
        ensure_handoff_at(&root, "web_agent", "task-002", "任务", Some(&ws)).unwrap();
        let status = status_json(&root, "web_agent");
        assert_eq!(status["workspace"], serde_json::json!(ws));
        assert_eq!(status["base_head"], serde_json::Value::Null);
        assert!(update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-002",
            "done",
            "完成",
            None
        )
        .is_none());
    }

    /// 新一轮派发清掉上一轮的审计结论，避免旧结论误导验收。
    #[test]
    fn test_redispatch_clears_previous_audit() {
        let root = tmp_root("audit-redispatch");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let Some((ws, _)) = tmp_git_workspace("audit-repo2") else {
            eprintln!("跳过：环境无 git");
            return;
        };

        ensure_handoff_at(&root, "web_agent", "task-001", "任务", Some(&ws)).unwrap();
        update_agent_status_from_doorbell_at(&root, "web_agent::task-001", "done", "完成", None);
        assert!(status_json(&root, "web_agent").get("head_check").is_some());

        ensure_handoff_at(&root, "web_agent", "task-002", "任务", None).unwrap();
        let status = status_json(&root, "web_agent");
        assert!(
            status.get("head_check").is_none(),
            "新派发必须清掉旧审计结论"
        );
        assert!(status.get("workspace").is_none());
        assert!(status.get("base_head").is_none());
        assert_eq!(status["state"], serde_json::json!("dispatched"));
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
        // 红线：不得擅自改动目标仓库的 git 历史（代码改动留工作区，Leader 审核后统一提交）
        assert!(read.contains("禁止擅自改动目标仓库的 git 历史"));
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
        let contract =
            ensure_handoff_at(&root, "web_agent", "task-001", "任务：重构页面", None).unwrap();
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
        // 上报通道唯一化：CLI 示例（done/blocked）——cli_cmd 前缀为 current_exe 动态值，断言不依赖前缀
        assert!(contract.contains("task done --id web_agent::task-001"));
        assert!(contract.contains("task blocked --id web_agent::task-001"));
        assert!(!contract.contains("curl"), "契约不得再宣传 curl 上报");
        assert!(!contract.contains("\"status\":\"done\""));
        assert!(contract.contains("web_agent::task-001"));
        // 红线：外部 Agent 不得擅自改动 git 历史（改动留工作区，Leader 审核后统一提交）
        assert!(contract.contains("禁止擅自改动目标仓库的 git 历史"));
        let status_str = std::fs::read_to_string(dir.join("status.json")).unwrap();
        assert!(!status_str.contains("token"), "token 不得落 status.json");
    }

    #[test]
    fn test_agent_status_uninitialized() {
        let root = tmp_root("status");
        assert_eq!(status_at(&root, "ghost")["state"], "uninitialized");
    }

    /// 多源合并 + agent 目录级跳过：目标已有该 agent 目录必须原样保留
    /// （绝不「整根存在即跳过」——那会导致建出新根后迁移永不触发）。
    #[test]
    fn test_migrate_handoff_merges_sources_with_agent_level_skip() {
        let base = tmp_root("migrate");
        let target = base.join("target");
        let src1 = base.join("src1");
        let src2 = base.join("src2");

        // src1：web_agent（含子目录递归内容）+ claude-code + 散落文件
        std::fs::create_dir_all(src1.join("web_agent").join("briefs")).unwrap();
        std::fs::write(src1.join("web_agent").join("read.md"), "src1 web").unwrap();
        std::fs::write(
            src1.join("web_agent").join("briefs").join("t1-brief.md"),
            "b1",
        )
        .unwrap();
        std::fs::create_dir_all(src1.join("claude-code")).unwrap();
        std::fs::write(src1.join("claude-code").join("read.md"), "src1 cc").unwrap();
        std::fs::write(src1.join("loose-file.md"), "x").unwrap();

        // src2：web_agent（旧副本，必须被跳过）+ gemini
        std::fs::create_dir_all(src2.join("web_agent")).unwrap();
        std::fs::write(src2.join("web_agent").join("read.md"), "src2 web").unwrap();
        std::fs::create_dir_all(src2.join("gemini")).unwrap();
        std::fs::write(src2.join("gemini").join("status.json"), "{}").unwrap();

        // target 已有 claude-code（新根数据优先，旧副本不得覆盖）
        std::fs::create_dir_all(target.join("claude-code")).unwrap();
        std::fs::write(target.join("claude-code").join("read.md"), "target cc").unwrap();

        let report = migrate_handoff_agents_into(&target, &[src1.clone(), src2.clone()]);

        // read_dir 顺序不保证，两个汇总都按「排序后比较集合」断言。
        // 同一 agent 在多个来源出现时首个 copy、其余 skip：因此 src2 的 web_agent
        // 旧副本**也会进 skipped**——这正是「不覆盖已有数据」的期望行为（见下方
        // 「src2 同名副本被跳过不覆盖」的断言），不是漏记。
        let mut copied = report.copied.clone();
        copied.sort();
        assert_eq!(
            copied,
            vec!["gemini".to_string(), "web_agent".to_string()],
            "copied 应含两个真实拷入的 agent"
        );
        let mut skipped = report.skipped.clone();
        skipped.sort();
        assert_eq!(
            skipped,
            vec!["claude-code".to_string(), "web_agent".to_string()],
            "目标已有的 agent 与后到的同名旧副本，都必须逐个跳过并记录"
        );
        assert!(report.failed.is_empty());

        // web_agent 来自 src1（含子目录递归拷贝）；src2 同名副本被跳过不覆盖
        assert_eq!(
            std::fs::read_to_string(target.join("web_agent").join("read.md")).unwrap(),
            "src1 web"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("web_agent").join("briefs").join("t1-brief.md"))
                .unwrap(),
            "b1"
        );
        // claude-code 保留目标版本（agent 目录级跳过的核心语义）
        assert_eq!(
            std::fs::read_to_string(target.join("claude-code").join("read.md")).unwrap(),
            "target cc"
        );
        assert!(target.join("gemini").join("status.json").is_file());
        assert!(
            !target.join("loose-file.md").exists(),
            "散落文件不属于 agent 目录"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 全部来源不存在 → 空汇总，且不得凭空创建新根。
    #[test]
    fn test_migrate_handoff_tolerates_missing_sources() {
        let base = tmp_root("migrate-empty");
        let target = base.join("target");
        let report =
            migrate_handoff_agents_into(&target, &[base.join("nope1"), base.join("nope2")]);
        assert!(report.is_empty());
        assert!(!target.exists(), "无内容可迁时不得创建空的新根");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_list_agent_statuses() {
        let root = tmp_root("list");
        // 注入两个已初始化 agent（含 '-' 命名，与 team.toml 对齐）
        init_agent_at(&root, "web_agent", "网页任务").unwrap();
        init_agent_at(&root, "claude-code", "编码任务").unwrap();
        // 派发任务 → task_id + dispatched 落盘（验证列表读到的是 status.json 实际内容）
        ensure_handoff_at(&root, "web_agent", "task-001", "任务：重构页面", None).unwrap();

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
    fn test_doorbell_keeps_assigned_task_state_for_unattributed_events() {
        let root = tmp_root("superseded-events");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-001", "旧任务", None).unwrap();
        ensure_handoff_at(&root, "web_agent", "task-002", "当前任务", None).unwrap();
        let before = status_json(&root, "web_agent");

        for event_id in [
            "web_agent::task-001",
            "web_agent",
            "web_agent::",
            "web_agent::   ",
        ] {
            for event_status in ["ready", "progress", "done", "blocked"] {
                assert!(update_agent_status_from_doorbell_at(
                    &root,
                    event_id,
                    event_status,
                    "旧任务迟到的上报",
                    Some("C:/报告/旧任务.md"),
                )
                .is_none());
                let status = status_json(&root, "web_agent");
                assert_eq!(
                    status["state"], before["state"],
                    "{event_id} {event_status}"
                );
                assert_eq!(status["task_id"], before["task_id"]);
                assert_eq!(status["head_check"], before["head_check"]);
                assert_eq!(status["last_event"]["status"], event_status);
                assert_eq!(status["last_event"]["summary"], "旧任务迟到的上报");
                assert_eq!(status["last_event"]["report_path"], "C:/报告/旧任务.md");
                let expected_task = if event_id == "web_agent::task-001" {
                    serde_json::json!("task-001")
                } else {
                    serde_json::Value::Null
                };
                assert_eq!(status["last_event"]["task_id"], expected_task);
            }
        }

        update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-002",
            "progress",
            "当前任务开始",
            None,
        );
        let progress = status_json(&root, "web_agent");
        assert_eq!(progress["task_id"], "task-002");
        assert_eq!(progress["state"], "in_progress");
        assert_eq!(progress["last_event"]["summary"], "当前任务开始");

        update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-002",
            "done",
            "当前任务完成",
            Some("C:/报告/当前任务.md"),
        );
        let done = status_json(&root, "web_agent");
        assert_eq!(done["task_id"], "task-002");
        assert_eq!(done["state"], "done");
        assert_eq!(done["last_event"]["summary"], "当前任务完成");
        assert_eq!(done["last_event"]["report_path"], "C:/报告/当前任务.md");
    }

    #[test]
    fn test_doorbell_does_not_audit_unattributed_events() {
        let root = tmp_root("superseded-audit");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let Some((ws, _)) = tmp_git_workspace("superseded-audit-repo") else {
            eprintln!("跳过：环境无 git");
            return;
        };
        ensure_handoff_at(&root, "web_agent", "task-001", "旧任务", None).unwrap();
        ensure_handoff_at(&root, "web_agent", "task-002", "当前任务", Some(&ws)).unwrap();
        assert!(git_run(
            Path::new(&ws),
            &["commit", "-q", "--allow-empty", "-m", "current task change"]
        )
        .is_some());
        let before = status_json(&root, "web_agent");

        for event_id in [
            "web_agent::task-001",
            "web_agent",
            "web_agent::",
            "web_agent::   ",
        ] {
            for event_status in ["done", "blocked"] {
                assert!(
                    update_agent_status_from_doorbell_at(
                        &root,
                        event_id,
                        event_status,
                        "无法归属当前任务的终态",
                        None,
                    )
                    .is_none(),
                    "{event_id} {event_status}"
                );
                let status = status_json(&root, "web_agent");
                for field in [
                    "state",
                    "task_id",
                    "workspace",
                    "base_head",
                    "base_branch",
                    "head_check",
                ] {
                    assert_eq!(
                        status[field], before[field],
                        "{event_id} {event_status} {field}"
                    );
                }
                assert_eq!(status["last_event"]["status"], event_status);
            }
        }

        let audit = update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-002",
            "done",
            "当前任务完成",
            None,
        )
        .expect("当前任务仍应使用自身派发基线审计");
        assert!(audit.changed);
        assert_eq!(
            status_json(&root, "web_agent")["head_check"]["changed"],
            true
        );
    }

    #[test]
    fn test_doorbell_grouping_updates_status_and_skips_unknown() {
        let root = tmp_root("group");
        init_agent_at(&root, "web_agent", "desc").unwrap();

        update_agent_status_from_doorbell_at(&root, "web_agent", "ready", "旧式就位事件", None);
        let status = status_json(&root, "web_agent");
        assert_eq!(status["task_id"], "");
        assert_eq!(status["state"], "in_progress");
        assert_eq!(status["last_event"]["summary"], "旧式就位事件");

        // ready 命中 agent 前缀 → state=in_progress（ready/progress 都是「开始确认」拉铃）+ last_event 保留原始值
        update_agent_status_from_doorbell_at(&root, "web_agent::task-001", "ready", "已就位", None);
        let status: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("web_agent").join("status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["task_id"], "");
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

    /// 一致性闸（回归 issue #69 第 2 项①）：上一轮的迟到 done 不得把结论写到下一轮头上。
    /// 同时守住 ②：不一致时 last_event 照记（保留事件自带 task_id）、不跑 head_audit。
    #[test]
    fn test_stale_cross_task_doorbell_is_isolated_from_current_task() {
        let root = tmp_root("stale-task");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let Some((ws, base)) = tmp_git_workspace("stale-repo") else {
            eprintln!("跳过：环境无 git");
            return;
        };

        // 轮次 A 上板（记基线 base）
        ensure_handoff_at(&root, "web_agent", "task-A", "任务A", Some(&ws)).unwrap();
        assert_eq!(
            status_json(&root, "web_agent")["base_head"],
            serde_json::json!(base)
        );

        // A 执行期间工作区多出一个 commit（A 的真实产出）
        assert!(git_run(
            Path::new(&ws),
            &["commit", "-q", "--allow-empty", "-m", "A work"]
        )
        .is_some());
        let head_a = git_run(Path::new(&ws), &["rev-parse", "HEAD"]).unwrap();

        // 轮次 B 上板：基线重新快照为 head_a，板上 task_id 变成 task-B
        ensure_handoff_at(&root, "web_agent", "task-B", "任务B", Some(&ws)).unwrap();
        let status = status_json(&root, "web_agent");
        assert_eq!(status["task_id"], "task-B");
        assert_eq!(status["base_head"], serde_json::json!(head_a));

        // B 执行期间又出一个 commit（相对 B 的基线也是「变了」）
        assert!(git_run(
            Path::new(&ws),
            &["commit", "-q", "--allow-empty", "-m", "B work"]
        )
        .is_some());

        // A 的 done 迟到到达：不得返回审计结论（否则 handoff_server 会推「本轮出现了
        // 不是你派发的提交」的误报），不得改 B 的 state/task_id，不得写 head_check
        let audit = update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-A",
            "done",
            "A 完成（迟到）",
            Some("C:/a-report.md"),
        );
        assert!(audit.is_none(), "跨任务迟到事件不得产出完工审计结论");
        let status = status_json(&root, "web_agent");
        assert_eq!(
            status["state"], "dispatched",
            "不允许用上一轮的结论覆盖 B 的 state"
        );
        assert_eq!(status["task_id"], "task-B", "不允许改写板上 task_id");
        assert!(
            status.get("head_check").is_none(),
            "不得用 B 的基线审 A 的事件"
        );
        assert_eq!(status["last_event"]["status"], "done", "事件本身不许被丢弃");
        assert_eq!(
            status["last_event"]["task_id"], "task-A",
            "last_event 必须保留事件自带的 task_id 原值以便追溯"
        );
        assert_eq!(status["last_event"]["report_path"], "C:/a-report.md");

        // B 自己的 done 按时到达：正常路径零变化（审计照跑、state 照改、唤醒照常）
        let audit = update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-B",
            "done",
            "B 完成",
            None,
        )
        .expect("本轮任务的完工必须正常审计");
        assert!(audit.changed, "B 的基线 head_a 之后确实有新提交");
        assert_eq!(audit.workspace, ws);
        let status = status_json(&root, "web_agent");
        assert_eq!(status["state"], "done");
        assert_eq!(status["task_id"], "task-B");
        assert_eq!(status["head_check"]["changed"], serde_json::json!(true));
    }

    /// 上板后失败落 error 态（回归 issue #69 第 2 项②）：打破 dispatched 永久幽灵，
    /// 且 error 不是终态——同一任务的门铃照常恢复 state。
    #[test]
    fn test_mark_agent_error_state_and_recovery() {
        let root = tmp_root("error-state");

        // ① agent 从未初始化/未登记（上板前失败）也要能看到 error
        mark_agent_error_at(
            &root,
            "ghost_agent",
            Some("task-001"),
            "agent「ghost_agent」未在 team.toml 登记，请先在外部 Agent 配置中心登记",
        );
        let status = status_json(&root, "ghost_agent");
        assert_eq!(status["state"], "error");
        assert_eq!(
            status["task_id"], "task-001",
            "无在途任务时补记本次失败的任务"
        );
        assert_eq!(status["error_task_id"], "task-001");
        assert!(status["error_reason"]
            .as_str()
            .unwrap()
            .contains("未在 team.toml 登记"));
        assert!(status["error_at"].as_str().is_some());

        // ② 上板后失败：另一轮在途任务的 task_id 不被本轮失败改写
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-A", "任务A", None).unwrap();
        mark_agent_error_at(
            &root,
            "web_agent",
            Some("task-B"),
            "投递失败（第 3 步 desktop_input）：hwnd 已失效",
        );
        let status = status_json(&root, "web_agent");
        assert_eq!(status["state"], "error");
        assert_eq!(status["task_id"], "task-A", "不能覆盖另一轮在途任务的归属");
        assert_eq!(status["error_task_id"], "task-B");
        assert!(status["error_reason"]
            .as_str()
            .unwrap()
            .contains("desktop_input"));
        assert!(
            status.get("dispatched_at").is_some(),
            "error 态不清空其他字段"
        );

        // ③ 非法 agent 名 → 拒绝落盘，不 panic、不建目录
        mark_agent_error_at(&root, "../evil", Some("t"), "x");
        assert!(!root.parent().unwrap().join("evil").exists());

        // ④ error 不是终态：同任务的后续门铃照常改写 state（接管 SOP 补投递后的恢复路径）
        assert!(update_agent_status_from_doorbell_at(
            &root,
            "web_agent::task-A",
            "done",
            "完成",
            None
        )
        .is_none());
        assert_eq!(status_json(&root, "web_agent")["state"], "done");
        assert_eq!(
            status_json(&root, "web_agent")["task_id"],
            "task-A",
            "一致时不改 task_id"
        );

        // ⑤ 重新派发清掉上一轮的失败痕迹（与「清掉上一轮审计结论」同一纪律）：
        // 否则 error_reason 会留在一个 state=dispatched 的新 record 旁边误导读文件的人
        ensure_handoff_at(&root, "web_agent", "task-B", "任务B", None).unwrap();
        let status = status_json(&root, "web_agent");
        assert_eq!(status["state"], "dispatched");
        assert_eq!(status["task_id"], "task-B");
        assert!(
            status.get("error_reason").is_none(),
            "新一轮派发必须清掉上一轮 error 痕迹"
        );
        assert!(status.get("error_at").is_none());
    }

    /// 并发写 status.json 不丢数据（回归 issue #69 附录 A）：每个写入都成功、最终文件是
    /// 某一次写入的完整快照、不留 tmp 残留。
    #[test]
    fn test_concurrent_status_writes_do_not_lose_data() {
        let root = tmp_root("concurrent-write");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        let n = 8u64;
        let mut handles = Vec::new();
        for i in 0..n {
            let root = root.clone();
            handles.push(std::thread::spawn(move || {
                let payload = serde_json::json!({
                    "agent": "web_agent",
                    "state": "in_progress",
                    "task_id": format!("task-{i}"),
                    "seq": i,
                });
                write_status_at(&root, "web_agent", &payload)
            }));
        }
        for h in handles {
            assert!(h.join().unwrap().is_ok(), "并发写不允许丢写");
        }
        let final_doc = status_json(&root, "web_agent");
        assert!(
            (0..n).any(|i| final_doc["seq"] == serde_json::json!(i)),
            "最终内容必须是某一次写入的完整快照，实际: {final_doc}"
        );
        let leftovers: Vec<String> = std::fs::read_dir(root.join("web_agent"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "rename 完成后不得残留 tmp: {leftovers:?}"
        );
    }

    /// 并发门铃事件（读—改—写整段在锁内）不产生半写/混合内容。
    #[test]
    fn test_concurrent_doorbell_events_keep_status_json_intact() {
        let root = tmp_root("concurrent-doorbell");
        init_agent_at(&root, "web_agent", "desc").unwrap();
        ensure_handoff_at(&root, "web_agent", "task-001", "任务", None).unwrap();
        let n = 8u64;
        let mut handles = Vec::new();
        for i in 0..n {
            let root = root.clone();
            handles.push(std::thread::spawn(move || {
                update_agent_status_from_doorbell_at(
                    &root,
                    "web_agent::task-001",
                    "progress",
                    &format!("进展 {i}"),
                    None,
                )
            }));
        }
        for h in handles {
            assert!(h.join().unwrap().is_none(), "progress 不产出审计结论");
        }
        let status = status_json(&root, "web_agent");
        assert_eq!(status["state"], "in_progress");
        let summaries: Vec<String> = (0..n).map(|i| format!("进展 {i}")).collect();
        assert!(
            summaries.contains(
                &status["last_event"]["summary"]
                    .as_str()
                    .unwrap()
                    .to_string()
            ),
            "last_event 必须是某一条事件的完整记录"
        );
    }

    #[test]
    fn test_agent_id_prefix() {
        assert_eq!(agent_id_prefix("web_agent::task-1"), Some("web_agent"));
        assert_eq!(agent_id_prefix("claude-code::0728-01"), Some("claude-code"));
        assert_eq!(agent_id_prefix("web_agent"), Some("web_agent")); // 无 '::' 退化为整串，不匹配目录即跳过
        assert_eq!(agent_id_prefix(""), None);
        assert_eq!(agent_id_prefix("::foo"), None);
    }

    /// 事件自带 task_id 解析：一致性闸的判据来源。
    #[test]
    fn test_event_task_id() {
        assert_eq!(event_task_id("web_agent::task-001"), Some("task-001"));
        assert_eq!(event_task_id("claude-code::0728-01"), Some("0728-01"));
        // 旧式无 '::' 纯 id 不带 task_id → 无从比对归属
        assert_eq!(event_task_id("web_agent"), None);
        // task_id 段为空视作缺失
        assert_eq!(event_task_id("web_agent::"), None);
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
        ensure_handoff_at(&root, "web_agent", "task-001", "任务", None).unwrap();
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
        ensure_handoff_at(&root, "web_agent", "task-001", "任务", None).unwrap();
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
