//! side_chat — 临时对话（Side Chat）Tauri 命令
//!
//! 复用 ExecAgent（[`nuphus::runtime::SubTaskRunner`]）的一次性临时执行体：
//! 主会话左侧滑出的面板里处理临时问题，跟随主会话生命周期、用完即弃。
//!
//! ## 边界（硬约束，逐条对应任务要求）
//!
//! - **不参与 task_dispatch**：本模块不碰 Leader / Runtime / 派发链路，只持有
//!   自己的 `SubTaskRunner`；
//! - **不写会话库**：会话只活在进程内存（[`slots`]），close / reset 即丢弃，
//!   没有任何 SQLite / session json 落盘；
//! - **不写 memory**：runner 走 `set_suppress_memory_write(true)`，
//!   `learn_from_success` 整段跳过；
//! - **不进 Leader 上下文**：`set_suppress_lifecycle_events` +
//!   `set_suppress_error_events` + `set_suppress_memory_write`，轮次级事件
//!   （ExecutionStarted / Completed 等）不进主通道；执行过程事件
//!   （ToolCallStart / ToolCallEnd / LlmTextDelta / ToolOutputLine）经
//!   [`SideChatEmitter`] 发独立频道 `'side-chat-event'`，主会话的
//!   `'nuphus-event'` 通道零占用；
//! - **不污染执行态**：工具集自带独立 `SharedSignals`（`ToolRegistry::exec*` 默认
//!   新建），`run_free` 入口置的 `Running` 只落在隔离信号上——前端 HUD、终止按钮、
//!   会话锁、任务面板对此无感知；执行中追加指令同样只写这份隔离队列
//!   （[`side_chat_append`]），绝不进主会话的 `state.signals.append_queue`；
//! - **工具面按风险分级**（大王 2026-11-03 定：不影响并行就可给，伙伴是同伴不是
//!   敌人）：`system_automation` 恒为 false，工具集按权限**物理裁剪**
//!   （[`nuphus::ToolRegistry::exec_with_permissions`]），在此之上：
//!   ① shell 只读子集（[`SIDE_CHAT_RESTORED_TOOLS`］，含 system_sleep 纯等待与
//!   system_info）经 [`nuphus::ToolRegistry::restore_tools`] 精确补回，schema 与
//!   执行入口**同步**恢复（执行期权限在 `build_runner` 里配套放行）；
//!   ② 桌面观察子集（截图 / AI 看图）经 registry 白名单放开，schema / has_tool /
//!   执行分发三处同源收口，跨进程 AutomationLock 兜底互斥；
//!   ③ `process_kill`（破坏性）、桌面控制类（点击 / 输入 / 拖拽）与 browser_*
//!   既不进 schema 也无执行入口，不是「看得到但被拒」。
//! - **跨会话两条通道**：本模块的回复默认只进侧边面板（独立频道
//!   `'side-chat-event'`）。通往主会话的路有两条：① **自动交付**——门禁为
//!   **本轮由 Leader 派发（[`SendOrigin::LeaderDispatch`]）或主会话正在派发
//!   执行任务**（`agent::parallel` 的 Exec 在途态，由 `RunGuard` 写入），
//!   轮次成功且回复非空的结论才经主会话共享提交入口
//!   `submit_user_message`（`source="sidechat"`）送入：Running 入队追加 /
//!   Finalizing 拒收后由后台轮询重投（≤60s），仍拒则落交付箱
//!   （[`nuphus::agent::parallel`]），由伙伴下一轮完成时补投。**门禁由后端
//!   按派发态自动判定，没有用户开关**——Leader 没派发时的闲聊结论推过去
//!   就是无关信息涌入，违背本面板的隔离职责；② **用户手动直接提交**——
//!   强制通道，任何轮次都能用，与派发态无关。
//! - **Leader 派发入口**（[`side_chat_dispatch`]）：Rust 侧直选已激活的伙伴壳
//!   注入 brief，不经前端；用户在用伙伴（壳在途）时回 `busy:true` 让 Leader
//!   降级，绝不与其争用同一执行体。
//!
//! ## 并发纪律
//!
//! `side_chat_send` 全程 **取 runner → 放锁 → `run_free` → 放回**，绝不在持有
//! [`slots`] 锁的情况下 await。Map 里常驻的是「壳」（[`Slot`]），执行中 runner
//! 被取出、壳留架——这样 close / reset 即便打在在途执行上也能命中（置中断标志），
//! 而迟到的放回凭 `Arc::ptr_eq` 认壳，对不上就丢弃，**已关闭的会话不会被复活**。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::agent::goal_types;
use nuphus::agent::references::ChatReference;
use nuphus::api::ApiClient;
use nuphus::llm::ClientFactory;
use nuphus::permissions::ToolPermissions;
use nuphus::runtime::SubTaskRunner;
use nuphus::session::Session;
use serde_json::{json, Value};
use tauri::{Emitter, Manager, State};

/// 同一临时会话已有一次发送在途时的拒绝文案（前端本就会禁用发送按钮，
/// 这里是后端侧的兜底，避免两次 send 悄悄各建一个 runner）。
const BUSY_MESSAGE: &str = "该临时会话正在执行上一条消息，请稍候";

/// Side Chat 在「系统与桌面自动化」总开关之外精确放开的只读工具子集。
///
/// 经 [`nuphus::ToolRegistry::restore_tools`] 补回——可补回范围由该方法内部
/// `match` 硬编码收死，`process_kill` / desktop_* / browser_* 无法经此获得。
/// 用途限于本面板的自验与只读勘察：编译与测试、`git log/show/diff/blame` 只读、
/// 拉取内部统计、等待。写操作与部署仍归主会话 Exec。
const SIDE_CHAT_RESTORED_TOOLS: &[&str] = &[
    "system_shell",
    "system_env_get",
    "process_list",
    "system_sleep",
    "system_info",
];

/// 侧边助手的系统提示。
///
/// 与 Leader / Exec 同源：身份 + 职责 + 能力边界 + 环境 + L0 骨架。
/// 不把「临时 / 一次性 / 用完即弃」写进身份——那是面板的生命周期，不是它的角色。
/// 权限只声明**当前关闭**项：开启项由工具清单本身表达，重复枚举只会变成过期硬编码。
/// `user_label` 走 core 单点解析（`prompt::resolve_user_label` / `address_rule`），
/// 与 Leader L0 同一规则：默认「用户」不注入，配置了称呼才在身份段落一行。
fn system_prompt(
    permissions: ToolPermissions,
    model: &str,
    provider: Option<&str>,
    user_label: &str,
) -> String {
    let mut off: Vec<&str> = Vec::new();
    if !permissions.file_access {
        off.push("文件读写");
    }
    if !permissions.web_search {
        off.push("联网检索");
    }
    if !permissions.system_automation {
        off.push("桌面控制（点击/输入/拖拽）与浏览器自动化、进程终止");
    }
    let mut ability = if off.is_empty() {
        "本会话未关闭任何能力。".to_string()
    } else {
        format!(
            "本会话关闭：{}。对应工具不在清单内，用现有能力完成或说明受限。",
            off.join(" / ")
        )
    };
    // 精确放开的只读子集必须与上面的「关闭」表述区分开：否则模型会以为
    // system_shell 不可用而放弃自验。工具名从 SIDE_CHAT_RESTORED_TOOLS 派生，
    // 不写死副本——白名单调整时文案自动跟随（同 62 行原则：重复枚举只会变成
    // 过期硬编码）。这段的主体是用途指引，那是工具清单本身表达不了的信息。
    if !permissions.system_automation {
        ability.push_str(&format!(
            "\n另：{} 已放开，用于编译与测试自验、git 只读勘察、拉取内部统计、等待；\
             桌面截图与 AI 看图已放开（只读观察，跨进程锁兜底互斥）；\
             进程终止、桌面控制（点击/输入/拖拽）与浏览器仍未开放，写操作与部署归主会话。",
            SIDE_CHAT_RESTORED_TOOLS.join(" / ")
        ));
    }
    // 构建与测试纪律：唯一事实源是知识库的 build-verification-policy.md（不是
    // git-pr-protocol 的 §2.4/§7.3——那份偏发版流程，且与本文档在零行为改动
    // 「跑不跑」上结论相反）。此处只做索引不复制条文：复制一份就会和正文漂移，
    // 文档更新后提示词变成过期的第二真相。
    ability.push_str(
        "\n跑构建 / 编译 / 测试前，先读知识库 `build-verification-policy`\
         （knowledge_search 搜「构建 验证」，Read 全文）。核心是 §0 三问门禁：\
         已改动且影响交付结论、有结论消费方、本机能做——任一为「否」即不跑。\
         零行为改动（只碰注释 / 提示词字符串 / 排版 / 文档）按 §1 **不跑**，\
         核对目标行即止。Nuphus 仓库一律 `--release` 增量；\
         测试目标别选错，选错会报 0 tests passed 假绿；\
          禁止 kill nuphus 自身进程、禁止另建 target 目录。\
          target 目录锁是单实例：本机已有其它 cargo 在跑时，新的 cargo 命令会被\
          工具层直接拒绝（「另一构建正在进行」）——那是防静默排队的保护，不是故障，\
          稍后重试或与用户确认错峰即可。",
    );

    let skills = nuphus::agent::prompt::skill_registry_section();

    let address_line = nuphus::agent::prompt::address_rule(user_label);

    let env = nuphus::agent::prompt::env_info_section(
        model,
        provider,
        None,
        false,
        None,
        nuphus::agent::prompt::EnvAudience::SubAgent,
    );

    format!(
        r#"# L0 Runtime Constitution

## Identity

你是 Nuphus 的协作助手：创意方案 · 探索调研 · 临时任务 · 核对审计。
与主会话并行协作；你的回复默认只留在本面板，经用户显式操作才回传——
「送入主对话」进主会话输入栏，「直接提交」直达主会话，两者都是全文。
{address_line}
人格：专业 理性 务实

## 职责
| 领域 | 范围 |
|---|---|
| 创意方案 | 方案脑暴与发散收敛、架构与交互构思、命名与文案打磨；把谈定的需求整理成可直接使用的专业提示词 / 指令（目标、约束、输入输出、边界一次写全） |
| 探索调研 | 读代码、查知识库与源码、联网核对、技术选型对比——先取证再作答，不凭印象 |
| 临时任务 | 一次性演算、格式转换、草稿撰写、快速验证思路——即问即答 |
| 核对审计 | 事实核查、数据校对、逻辑验算、引用与 `路径:行号` 核对——结论附来源，存疑处明说 |

超出以上范围的重活（多文件重构、需要完整验证闭环的交付）→ 不硬撑：给出切入点与关键路径，由用户显式回传主会话推进。

## 能力边界
- 工具清单即边界：本请求 `tools` 字段之外的工具一律不可用，不必推测原因，也不要尝试绕过。
- {ability}

## 结论交付
你的结论可能被送到主会话丞相面前（用户也可手动提交）。丞相不在对话现场：结论必须自包含（问题+判断+证据）、可回溯（文件:行号/命令输出）、不确定处显式标注。

## 输出契约
| # | 约束 |
|---|---|
| 1 | 结论先行，依据其次；不复述问题、不客套、不反问是否继续 |
| 2 | 多要点分条或表格，单点直接成段；不用装饰性标题分节 |
| 3 | 引用代码用 `路径:行号`，引用网页给可访问链接 |

## 工具用法
| 需要 | 用 |
|---|---|
| 查 Nuphus 知识 / 技能细节 | `skill_query` → `skill_read` |
| 查既往结论、历史决策 | `memory_search`（只读；本会话不写记忆） |
| 读代码 / 配置 / 知识库文件 | Read / Grep / Glob（文件读写关闭时跳过） |
| 核对时效性信息 | `web_search` → `web_extract`（联网关闭时跳过） |
| 用户消息里的 `[附件: 路径]` | 本地文件绝对路径，用 Read 打开 |
| 用户显式选定的 skill | 内容已随消息注入，直接按它执行 |

## 代码地图
问「这功能在哪 / 为什么这样」时先按此定位，再 Read 具体文件；表里没有的先 Glob 目录，别凭文件名猜内容。

| 领域 | 位置 |
|---|---|
| 界面 / 交互 / 页面 | frontend/src/main-window/ |
| 样式 | frontend/src/styles/ |
| 桌面端命令 / IPC | src-tauri/src/commands/ |
| Agent 运行时 / 提示词 / ReAct 循环 | src/agent/ · src/runtime/ |
| 工具定义与注册 | src/tools/ |
| 模型绑定 / provider / 用户偏好 | src/config/ |
| 工作流引擎 | src/workflow/ |
| 技能 / 知识 / 工作流资产 | plugin/ |

{skills}
## 运行机制
| 概念 | 一句话 |
|---|---|
| 执行链路 | Leader 决策并派发 → Exec 执行（无桌面 / 浏览器工具）→ 任务面板是台账只读投影，不代表真实执行结果 |
| 记忆 | conversation 对话 / task_trace 执行轨迹 / distill 会话提炼 / pattern 复用经验 |
| 角色工具面 | 每个角色拿到的工具集不同；本面板默认关闭系统与桌面自动化，仅放开只读 shell 子集（见「能力边界」段，不在此重复列举） |

## 工作目录
用户在伙伴面板顶栏可选择项目目录：该选择随**每条消息**下发，形态是消息开头的
`[工作目录: <绝对路径>]` 一行。
- 出现该行 → 本轮所有相对路径以它为基准，**覆盖**上方「环境」段里的「项目目录」
  （那是主会话的设置，不是你这一轮的基准）；
- 没有该行 → 按上方「项目目录」执行；
- 文件类工具请用该目录下拼出的**绝对路径**调用，不要依赖进程当前目录。

## 对话保存位置（反查用）
本面板的产出全部落在同一个目录，用户随时可以翻：
- 目录: {archive}
- `history.jsonl` — **每轮执行自动归档**：question / reply / 工具步骤 / 成败 / 模型，一行一条，只保留最近 {history_cap} 轮；
- `{{YYYYMMDD-HHmmss}}-{{key 前 8 位}}.md` — 用户点「保存」时导出的整段会话 Markdown。
需要回溯「之前聊过什么」「上次的结论是什么」时，先读这个目录里的文件再回答，
不要凭印象复述；文件读不到就明说没查到。

{env}
{l0}"#,
        ability = ability,
        skills = skills,
        env = env,
        // 反查目录取运行时真实值（NUPHUS_DATA_DIR 可覆盖），不写死路径
        archive = nuphus::utils::nuphus_data_dir()
            .join("side-chat")
            .display()
            .to_string(),
        // 归档上限引用常量本身：写死数字会随 HISTORY_CAP 调整而过期
        history_cap = HISTORY_CAP,
        l0 = nuphus::agent::prompt::l0_kernel(),
    )
}

/// 一个临时会话的「壳」：执行体 + 生命周期标志。
///
/// `runner` 为 `None` 有两种含义——尚未构建，或已被 `side_chat_send` 取出
/// 正在执行；靠 `activated` 区分（见 `side_chat_send` 的取用逻辑）。
struct Slot {
    runner: Option<SubTaskRunner>,
    /// 壳是否已激活过（至少完整走过一轮 send）。区分「新建未用」与「执行中」，
    /// 并发 send 才会被拒绝而不是悄悄重建第二个执行体。
    activated: bool,
    /// 建 runner 时生效的权限（工具集按它物理裁剪）；变化即重建并携带会话。
    permissions: ToolPermissions,
    /// 建 runner 时前端请求的模型标识；变化即重建并携带会话。
    model: Option<String>,
    /// 建 runner 时的提供商；与 model 成对定位。变化同样要重建——
    /// 同名模型跨提供商时只换 provider 就会连到不同 base_url。
    provider: Option<String>,
    /// 建 runner 时生效的推理强度覆盖（None = 提供商默认）；变化即重建——
    /// effort 烧在客户端里，复用的执行体换不了档位。
    effort: Option<String>,
    /// 中断标志：close / reset / 用户点「停止」（[`side_chat_cancel`]）置位。
    /// `run_free` 循环与在途工具执行（registry 持有同一实例）同时观测，
    /// 故关闭能立刻生效。置位后**不自动复位**——send 在轮次起点清零：
    /// 复用路径继承同一实例，不清则下一轮首个迭代边界就被误判取消。
    cancel: Arc<AtomicBool>,
    /// 执行体自带的**隔离**信号状态（`ToolRegistry::exec*` 新建的那份）。
    /// 追加指令队列住在这里：绝不能借主会话的 `state.signals`，否则
    /// Side Chat 的追加会注入 Leader 的上下文（违背模块边界的隔离约定）。
    /// 复用路径沿用上一轮的句柄；重建路径在 runner 建好后回填。
    signals: Option<nuphus::state::SharedSignals>,
    /// 是否有在途轮次。**必须是显式字段**，不能靠 `runner.is_none()` 推断——
    /// 那个条件同时覆盖「正在执行（runner 被 take）」与「尚未构建」，
    /// 而追加指令只在真正执行中才有消费者，空闲态必须明确拒绝。
    running: bool,
    /// 在途轮次的来源（大王 2026-11-04 规则）：派发轮在途时 Leader 可追加
    /// 任务（唯一豁免），用户轮在途时派发一律 busy 拒绝。随轮次起点写入、
    /// ReturnGuard drop 清除。
    current_origin: Option<SendOrigin>,
    /// 本轮主指令原文（append 判重基准，`mobile_append::is_duplicate_of_last`
    /// 的整轮有效口径：执行期提交同一内容没有合法语义）。
    last_message: String,
    /// 最近一轮的完成结果。**刷新恢复的关键**：前端刷新会丢掉 `invoke` 的
    /// Promise，回复只经返回值传递就意味着「永远到不了界面」。这里留一份快照，
    /// 由 [`side_chat_state`] 取走；同一轮完成时还会 emit `side_chat_turn_finished`
    /// 事件，覆盖「刷新发生在执行中」的场景。
    last_turn: Option<TurnResult>,
}

/// 一轮完成的结果快照（供刷新后回填界面）。
#[derive(Clone)]
struct TurnResult {
    /// 前端生成的轮次 id：回填时据此判重，避免正常返回 + 事件 + state 三路重复
    turn_id: String,
    reply: String,
    steps: Vec<Value>,
    ctx_used: u64,
}

impl Slot {
    fn shell(
        permissions: ToolPermissions,
        model: Option<&str>,
        effort: Option<&str>,
        provider: Option<&str>,
    ) -> Self {
        Self {
            runner: None,
            activated: false,
            permissions,
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            effort: effort.map(str::to_string),
            cancel: Arc::new(AtomicBool::new(false)),
            signals: None,
            running: false,
            current_origin: None,
            last_message: String::new(),
            last_turn: None,
        }
    }
}

/// 临时会话表（进程内唯一）。key = 前端 `crypto.randomUUID()`，随面板开合生灭。
fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析 LLM 客户端：优先前端传来的当前模型，失败回退主模型绑定。
///
/// `effort` = 用户当轮选择的推理强度（None / 空 = 不覆盖，用配置原值）。
/// 覆盖只发生在**本次建的客户端**上（[`ClientFactory::create_client_with_effort`]），
/// 不写 config.toml——主会话的 effort 不受影响。「跟随主会话」模式同样支持
/// 覆盖：解析主模型绑定 id 后走同一条覆盖路径。
fn resolve_client(
    model: Option<&str>,
    provider: Option<&str>,
    effort: Option<&str>,
) -> Result<Arc<dyn ApiClient>, String> {
    let factory = ClientFactory::live();
    let effort = effort
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let provider = provider.map(str::trim).filter(|s| !s.is_empty());
    let client = match model {
        // 知道 provider 就必须按 (provider, model) 精确建：同名模型跨 provider 时
        // 按 id 查找取的是「段序首段」，会连到别的 base_url（本地代理 8317 事故）。
        Some(id) if provider.is_some() => {
            let provider = provider.unwrap_or_default();
            factory
                .create_client_for_with_effort(provider, id, effort.clone())
                .or_else(|err| {
                    tracing::warn!(
                        "side_chat: create_client_for_with_effort({provider}/{id}) 失败({err})，回退主模型"
                    );
                    factory.create_main_client()
                })
        }
        Some(id) => {
            // 没 provider：先看是否歧义，宁可报错也不静默连错提供商。
            let ambiguous = factory
                .registry()
                .ok()
                .map(|r| r.find_model_candidates(id).len() > 1)
                .unwrap_or(false);
            if ambiguous {
                Err(nuphus::NuphusError::llm(format!(
                    "模型 '{id}' 在多个提供商下都存在，无法确定连接目标；请重选该模型（会带上提供商）"
                )))
            } else {
                factory
                    .create_client_with_effort(id, effort.clone())
                    .or_else(|err| {
                        tracing::warn!(
                            "side_chat: create_client_with_effort({id}) 失败({err})，回退主模型"
                        );
                        factory.create_main_client()
                    })
            }
        }
        None => match effort {
            Some(effort) => {
                // 跟随主会话 + 自选 effort：取主模型 id 走同一覆盖路径；
                // 解析失败或建失败则回退主模型客户端（effort 落空）。
                let main_id = factory
                    .registry()
                    .ok()
                    .and_then(|r| r.resolve_main_binding().ok())
                    .map(|(_, model)| model);
                match main_id {
                    Some(id) => factory
                        .create_client_with_effort(&id, Some(effort))
                        .or_else(|err| {
                            tracing::warn!(
                                "side_chat: 主模型 effort 覆盖失败({err})，回退主模型客户端"
                            );
                            factory.create_main_client()
                        }),
                    None => factory.create_main_client(),
                }
            }
            None => factory.create_main_client(),
        },
    }
    .map_err(|e| format!("创建 LLM 客户端失败: {e}"))?;
    Ok(client)
}

/// Side Chat 独立事件发射器：只发 `'side-chat-event'` 一个频道。
///
/// 与主会话通道（`'nuphus-event'`）零交集：ToolCallStart / ToolCallEnd /
/// LlmTextDelta / ToolOutputLine 经这里直达前端侧边面板（前端按
/// session_key 过滤后渲染实时执行过程），主会话的 useEvents 监听的是另一个
/// 事件名，永不被污染。轮次级事件（ExecutionStarted / Completed）在发射点
/// 之前就被 `set_suppress_lifecycle_events` 挡下，不会漏进来。
///
/// 只做「app.emit(事件名, payload)」这一件事：不套 FramedEvent / seq /
/// 排序 / 缓冲——独立频道，前端按 call_id 对齐即可。
struct SideChatEmitter<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    session_key: String,
}

impl<R: tauri::Runtime> EventEmitter for SideChatEmitter<R> {
    fn emit(&self, event: NuphusEvent) {
        let _ = self.app.emit(
            "side-chat-event",
            json!({ "session_key": self.session_key, "event": event }),
        );
    }
}

/// 「Explore」哨兵：面板项目选择器的内置项——它**不属于任何项目路径**。
///
/// 与 `None`（跟随主会话）的差别：`None` 不注入头部，模型沿用环境段里的全局
/// `preferences.project_dir`；哨兵则显式声明「本轮无项目」，把环境段那一行
/// 覆盖掉，模型不再假设代码库位置。
///
/// 前后端各存一份同名常量，改一处必须同步改另一处（前端
/// `frontend/src/main-window/chat/SideChatPanel.tsx` 的 `EXPLORE_PROJECT`）。
const EXPLORE_PROJECT: &str = "__explore__";

/// 面板选定的工作目录 → 消息头部标记段。空 / 纯空白 → None（不注入）。
///
/// 只做格式化，**不做路径存在性校验**：目录可能在网络盘 / 已卸载，硬校验会让
/// 「先问一句有没有这个项目」都发不出去。可达性由真正发起工具调用的那一步报错。
///
/// 措辞必须显式覆盖：system_prompt 的环境段里带着主会话的项目目录（全局
/// `preferences.project_dir`），不写清优先级时模型会二选一随机猜。
fn project_dir_header(raw: Option<&str>) -> Option<String> {
    let dir = raw?.trim();
    if dir.is_empty() {
        return None;
    }
    if dir == EXPLORE_PROJECT {
        return Some(
            "[工作目录: 无 · Explore]\n\
             用户在本面板显式选择了 Explore：本轮不绑定任何项目目录，\
             忽略环境段里主会话的「项目目录」，不要假设代码库位置；\
             需要读写文件时先向用户确认路径。"
                .to_string(),
        );
    }
    Some(format!(
        "[工作目录: {dir}]\n\
         用户在本面板显式选择了该项目：本轮所有相对路径以它为基准，\
         覆盖环境段里主会话的「项目目录」；文件类工具请拼该目录下的绝对路径。"
    ))
}

/// 一轮消息的来源：用户面板操作，或 Leader 后端派发（[`side_chat_dispatch`]）。
///
/// 只影响两件事：① 消息前缀是否叠派发性质声明（[`compose_message`]）；
/// ② 完成时结论是否走自动交付门禁（[`auto_deliver_gate`] 的
/// `leader_dispatched`）——派发轮次的结论必须有确定回路，否则派发等于把
/// 任务丢进黑洞。取壳 / 执行 / 交付闭环对两者完全同构。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendOrigin {
    User,
    LeaderDispatch,
}

/// Leader 派发轮次的任务性质声明（注入点 D）。
///
/// 伙伴没有跨轮上下文，brief 之外的信息无从推断；结论经门禁自动回丞相，
/// 这段把「自包含 + 可回溯」的交付要求写在指令最前面。与注入点 B 并存时
/// B 在前（B 是并行避让的环境事实，本段是任务性质——先知环境再知任务）。
const LEADER_DISPATCH_NOTE: &str = "[Leader 派发]\n本任务由主会话丞相派发：结论将自动回传丞相，必须自包含（任务+判断+证据）、可回溯（文件:行号/命令输出）。";

/// 消息前缀拼装（纯函数，可单测）：注入点 B（主会话 Exec 在途 → 并行避让）
/// 在前，注入点 D（Leader 派发 → 任务性质）次之，用户 / 派发指令原文垫底。
///
/// 用户轮次且 Exec 空闲时零前缀——输出与拆分前逐字节一致（无行为改动）。
fn compose_message(message: String, exec_title: Option<&str>, origin: SendOrigin) -> String {
    let mut prefixes: Vec<String> = Vec::new();
    if let Some(title) = exec_title {
        prefixes.push(format!(
            "[并行状态: 主会话正在执行任务「{}」]\n它可能正在改文件：你写前先读现状、不做无差异覆盖、以磁盘内容为准。",
            title
        ));
    }
    if origin == SendOrigin::LeaderDispatch {
        prefixes.push(LEADER_DISPATCH_NOTE.to_string());
    }
    if prefixes.is_empty() {
        message
    } else {
        format!("{}\n\n{}", prefixes.join("\n\n"), message)
    }
}

/// 身份称呼解析：读 AppState 的 `relation_cache`（`RelationConfig`）。
///
/// 为什么读缓存而不是 relation.json：缓存由启动时读盘播种（`state.rs`）、
/// 主会话每条消息与 SoulPage 保存时更新，是进程内唯一活值；直接读盘会漏掉
/// 「本次运行刚改过称呼」的那一次。锁中毒时降级为 None（回落默认称呼），
/// 与 parallel 模块同一约定——身份缺失不该升级成进程故障。
fn resolve_user_label(state: &AppState) -> String {
    let relation = state
        .relation_cache
        .read()
        .ok()
        .and_then(|cache| cache.clone());
    nuphus::agent::prompt::resolve_user_label(relation.as_ref())
}

/// 构建一个临时执行体。`session` 非空 = 携带既有会话（权限/模型/effort 变更时的重建）。
fn build_runner(
    permissions: ToolPermissions,
    model: Option<&str>,
    provider: Option<&str>,
    goal: &str,
    session: Option<Session>,
    cancel: &Arc<AtomicBool>,
    emitter: Arc<dyn EventEmitter>,
    effort: Option<&str>,
    user_label: &str,
) -> Result<(SubTaskRunner, nuphus::state::SharedSignals), String> {
    let llm = resolve_client(model, provider, effort)?;
    // 工具集按权限物理裁剪 + 中断标志与 run_free 共用同一实例
    let mut tools = nuphus::ToolRegistry::exec_with_permissions(permissions);
    // 精确放开只读 shell 子集：schema 层补回（白名单在 restore_tools 内硬编码收死）。
    tools.restore_tools(SIDE_CHAT_RESTORED_TOOLS);
    // 桌面观察子集（大王 2026-11-03 批：截图 + AI 看图）：registry 白名单放开，
    // schema 与执行入口同步收口；控制类与浏览器不可达。跨进程 AutomationLock
    // 在 desktop 执行入口兜底互斥（与主会话 / Exec 同路径）。
    tools.set_desktop_client(nuphus::desktop::DesktopClient::new());
    tools.enable_automation_observe_only();
    tools.set_cancel_flag(cancel.clone());
    // 追加指令队列的宿主：registry 自带（`exec*` 新建）的隔离信号状态。
    // 必须在 tools 被 move 进 runner 之前取出句柄——之后只剩 registry 内部持有。
    let signals = tools.signals().clone();

    // 空 provider 视为未知（与 SubTaskRunner::resolve_supports_vision 同一口径）
    let provider_name = llm.provider_name();
    let provider = if provider_name.is_empty() {
        None
    } else {
        Some(provider_name)
    };
    let prompt = system_prompt(permissions, llm.model_name(), provider, user_label);

    let mut runner = SubTaskRunner::new_free(llm.clone(), tools, prompt, goal.to_string());
    runner.set_tool_permissions(ToolPermissions {
        // 与 schema 层配套：restore_tools 只放回了白名单内的只读工具，可见面已被
        // 收窄，故此处放行 SystemAutomation 不会让未列名工具变得可调——未补回的
        // 工具不在 registry.tools 里，has_tool 守卫直接判「未知工具」。
        // Slot.permissions 仍保持前端语义（system_automation=false），两者刻意不同：
        // 前者是「用户没开系统自动化」的产品语义，后者是白名单机制下的执行期放行。
        system_automation: true,
        ..permissions
    });
    // Side Chat 独立事件频道：ToolCallStart/End/LlmTextDelta 直达前端侧边面板
    // （与下面的 suppress 三连无关——那三个只挡轮次级事件）
    runner.set_event_emitter(emitter);
    // 临时执行体三连：生命周期事件 / 错误事件 / 记忆写入全部抑制
    runner.set_suppress_lifecycle_events(true);
    runner.set_suppress_error_events(true);
    runner.set_suppress_memory_write(true);
    runner.set_context_window(goal_types::try_get_context_window_for(
        llm.model_name(),
        Some(llm.provider_name()),
    ));
    if let Some(session) = session {
        runner.set_session(session);
    }
    Ok((runner, signals))
}

/// 执行体归宿守卫：`side_chat_send` 的任何退出路径（正常返回 / 构建失败 /
/// `run_free` panic）都由它收尾，杜绝「壳已激活但没有执行体」的卡死态——
/// 那种状态下后续同一 key 的发送会被永久误判为「执行中」。
///
/// Drop 语义：
/// - 持有 runner 且壳还在（cancel 实例对得上）→ 放回，供下一轮复用；
/// - 持有 runner 但壳已不在（close / reset / 权限变更顶替）→ 丢弃，
///   已关闭的会话绝不被迟到的放回复活；
/// - 未持有 runner（构建失败即返回）→ 复位壳的激活标志，后续发送可重试。
struct ReturnGuard {
    key: String,
    cancel: Arc<AtomicBool>,
    runner: Option<SubTaskRunner>,
}

impl ReturnGuard {
    /// 跑一轮临时问答：先清掉上轮取消/失败残留的孤儿 user 消息，再 push
    /// 本轮消息 → `run_free`。
    async fn run_turn(
        &mut self,
        message: &str,
        cancel: &AtomicBool,
    ) -> Result<(bool, String, Vec<nuphus::ExecutionStep>), String> {
        let runner = self
            .runner
            .as_mut()
            .ok_or_else(|| "内部错误：临时执行体未就绪".to_string())?;
        // 顺序不能反：pop 必须在 push 之前。取消早退 / LLM 错误上抛都会把上轮
        // 的 user 消息留在 session 尾部，下轮开跑前先清掉（同主循环
        // `react_loop` 入口手法）；若放到 run_free 开头，pop 掉的会是刚 push
        // 的本轮新消息，旧孤儿反而永远留下。
        runner.session_mut().pop_last_user_if_orphan();
        runner.session_mut().push_user(message.to_string());
        runner
            .run_free(cancel)
            .await
            .map_err(|e| format!("临时对话执行失败: {e}"))
    }
}

impl Drop for ReturnGuard {
    fn drop(&mut self) {
        // ── W2（并行协作契约）：伙伴在途收尾 ──
        // 放在 match 之前：放回 / 丢弃 / 构建失败三条退出路径都算本轮结束。
        // 按 key 匹配清——迟到的 drop（壳被重建顶替过）动不了新轮次。
        nuphus::agent::parallel::clear_side(&self.key);
        match self.runner.take() {
            Some(runner) => {
                let returned = slots().lock().ok().and_then(|mut map| {
                    map.get_mut(&self.key)
                        .filter(|slot| Arc::ptr_eq(&slot.cancel, &self.cancel))
                        .map(|slot| {
                            slot.runner = Some(runner);
                            // 轮次结束：追加指令不再有消费者。残留在队列里的
                            // （在最后一个迭代边界之后入队的）必须在放回时清掉，
                            // 否则会被下一轮的开头注入——用户已经看不到它被
                            // 受理，却会突然改变下一轮的行为。
                            slot.running = false;
                            slot.current_origin = None;
                            if let Some(signals) = slot.signals.as_ref() {
                                let mut state = nuphus::state::SignalState::write(signals);
                                if !state.append_queue.is_empty() {
                                    tracing::info!(
                                        "side_chat: 轮次结束丢弃未消费追加指令 {} 条（key={}）",
                                        state.append_queue.len(),
                                        self.key
                                    );
                                    state.append_queue.clear();
                                }
                            }
                        })
                });
                if returned.is_none() {
                    // 壳已不在：runner 随本函数结束丢弃，旧会话不复活
                    tracing::debug!("side_chat: 会话壳已不在，执行体丢弃（key={}）", self.key);
                }
            }
            None => {
                // 构建失败：复位激活标志与在途标志，避免后续发送被误判为「执行中」
                if let Ok(mut map) = slots().lock() {
                    if let Some(slot) = map.get_mut(&self.key) {
                        if Arc::ptr_eq(&slot.cancel, &self.cancel) {
                            slot.activated = false;
                            slot.running = false;
                            slot.current_origin = None;
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// 结论自动交付（并行协作契约）
// ============================================================================

/// Finalizing 拒收后的重投上限（每次间隔 2s，合计约 60s——覆盖记忆落盘 /
/// 自动提炼 / 镜像回填的常规收尾时长）。
const DELIVERY_RETRY_LIMIT: u32 = 30;

/// 自动交付门禁（纯函数，可单测）：轮次成功 + 回复非空 + **本轮是 Leader 派发轮**。
///
/// 只认「当前轮是不是派发来的」，没有别的判断（大王 2026-11-04 定）：
/// 派发轮 brief 已在起点经 `side_chat_dispatch_question` 显式落为用户消息、
/// 结论直推 Leader；伙伴执行中不接受派发（返回 busy），同一时刻只有一个伙伴轮，
/// 「当前轮是派发还是用户」清清楚楚，不需要再看主会话忙不忙。
///
/// 非派发轮（用户自己发的）结论不自动推——那是用户指令，要推请手动「直接提交」
/// （另一条始终可用的通道，与门禁无关）。丞相不在对话现场，派发轮次的结论必须
/// 有确定回路，否则派发等于把任务丢进黑洞。
fn auto_deliver_gate(turn_ok: bool, reply: &str, leader_dispatched: bool) -> bool {
    turn_ok && !reply.trim().is_empty() && leader_dispatched
}

/// 交付给主会话的结论前缀（纯函数：文案只此一处，单测覆盖）。
///
/// 丞相不在对话现场，前缀必须把「未经用户复核」说在最前，避免并行结论被
/// 当成用户已确认的指令。turn_id 取前 8 位——按**字符**切而非字节（前端
/// id 虽为 ASCII，按字节切在未来引入多字节 id 时会切出乱码）。
fn delivery_header(turn_id: &str) -> String {
    let short: String = turn_id.chars().take(8).collect();
    format!(
        "[来自临时伙伴的结论 · 轮次 {short}]\n丞相：以下是临时伙伴并行得出的结论（用户可能不在电脑旁，未经其复核）。请按证据采信或忽略：结论须可回溯（文件:行号/命令输出），不可回溯的标注「未验证」再决定是否采用。"
    )
}

/// 投递一条伙伴结论到主会话（复用共享提交入口，零新机制）。
///
/// `Ok(true)` = 主会话已受理（Idle 直接跑 / Running 入队追加）；
/// `Ok(false)` = Finalizing 窗口拒收（收尾期没有消费方，`submit_user_message`
/// 显式拒收）；`Err` = 提交入口本身失败。send_id 带轮次 id，与手动提交共用
/// 同一去重防线而不互相干扰。
async fn deliver<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    item: &nuphus::agent::parallel::DeliveryItem,
) -> Result<bool, String> {
    let resp = crate::commands::process::submit_user_message(
        app.clone(),
        state,
        format!("{}{}", delivery_header(&item.turn_id), item.reply),
        None,
        None,
        None,
        None,
        None,
        Some(format!("sidechat-deliver-{}", item.turn_id)),
        Some("sidechat".to_string()),
        false,
    )
    .await?;
    Ok(resp.rejected.is_none())
}

/// 伙伴结论自动交付的总入口：先投本条，再按受理结果决定补投 / 重试 / 落箱。
///
/// 只由 `side_chat_send_impl` 的「门禁通过」路径调用（见 [`auto_deliver_gate`]）。
async fn deliver_side_conclusion<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    item: nuphus::agent::parallel::DeliveryItem,
) {
    match deliver(app, state, &item).await {
        // 受理 → 顺手补投交付箱积压（此前被拒落下的）
        Ok(true) => drain_delivery_box(app, state).await,
        // 收尾期拒收 → 后台轮询重投，不占住伙伴这一轮的 IPC
        Ok(false) => spawn_delivery_retry(app.clone(), item),
        // 提交入口本身失败 → 落箱，等伙伴下一轮完成时补投
        Err(e) => {
            tracing::warn!(
                "side_chat: 结论交付提交失败，落交付箱等下一轮补投（turn={}）: {e}",
                item.turn_id
            );
            nuphus::agent::parallel::enqueue_delivery(item);
        }
    }
}

/// 补投交付箱积压（FIFO）。遇拒即停：本条与剩余全部按原顺序回箱，
/// 等下一个受理窗口（伙伴下一轮完成）再来。
async fn drain_delivery_box<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &AppState) {
    let mut pending = nuphus::agent::parallel::take_deliveries();
    while !pending.is_empty() {
        let item = pending.remove(0);
        match deliver(app, state, &item).await {
            Ok(true) => {}
            _ => {
                let mut back = Vec::with_capacity(pending.len() + 1);
                back.push(item);
                back.append(&mut pending);
                for it in back {
                    nuphus::agent::parallel::enqueue_delivery(it);
                }
                return;
            }
        }
    }
}

/// Finalizing 拒收后的后台重投：每 2s 探一次执行阶段，离开收尾期即重投，
/// 上限 [`DELIVERY_RETRY_LIMIT`] 次；仍拒则落箱，交给伙伴下一轮完成时补投。
///
/// 为什么不在 send_impl 里同步等：收尾窗口可能长达数十秒，让伙伴这一轮
/// invoke 挂住等它没有意义——用户还在面板前等着看回复。
fn spawn_delivery_retry<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    item: nuphus::agent::parallel::DeliveryItem,
) {
    tauri::async_runtime::spawn(async move {
        for _ in 0..DELIVERY_RETRY_LIMIT {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            // 块内重新取 state：spawn 出的任务不依赖 invoke 的生命周期，
            // AppState 由 Tauri 管理（Runtime P0 保护模式，同 process.rs）。
            let st = app.state::<AppState>();
            if st.busy.stage() == nuphus::state::ExecutionStage::Finalizing {
                continue;
            }
            match deliver(&app, st.inner(), &item).await {
                Ok(true) => {
                    drain_delivery_box(&app, st.inner()).await;
                    return;
                }
                // 竞态：刚离开收尾期又进去了，继续等下一个窗口
                Ok(false) => continue,
                Err(e) => {
                    tracing::warn!(
                        "side_chat: 结论重投提交失败，落交付箱（turn={}）: {e}",
                        item.turn_id
                    );
                    nuphus::agent::parallel::enqueue_delivery(item);
                    return;
                }
            }
        }
        tracing::info!(
            "side_chat: 结论重投 {DELIVERY_RETRY_LIMIT} 次仍未受理，落交付箱（turn={}）",
            item.turn_id
        );
        nuphus::agent::parallel::enqueue_delivery(item);
    });
}

/// 临时对话发送：一次问答 = 一个 `run_free` 轮次，多轮上下文由 runner 自身
/// session 累积（`run_free` 每轮把 assistant 回复写回 session）。
///
/// 桌面 IPC 入口（thin wrapper）：委托泛型核心 [`side_chat_send_impl`]。
/// `app` 由 Tauri 注入，只用于挂 [`SideChatEmitter`]（独立事件频道）；
/// `state` 只服务于结论自动交付（门禁见 [`auto_deliver_gate`]）。
/// origin 恒为 [`SendOrigin::User`]——派发轮次走 [`side_chat_dispatch`]，
/// 前端没有任何路径能自称派发（api.ts 不发这个字段）。
#[tauri::command]
pub async fn side_chat_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_key: String,
    message: String,
    file_access: bool,
    web_search: bool,
    model: Option<String>,
    references: Option<Vec<ChatReference>>,
    effort: Option<String>,
    provider: Option<String>,
    project_dir: Option<String>,
    turn_id: Option<String>,
) -> Result<Value, String> {
    side_chat_send_impl(
        app,
        state.inner(),
        session_key,
        message,
        file_access,
        web_search,
        model,
        references,
        effort,
        provider,
        project_dir,
        turn_id,
        SendOrigin::User,
    )
    .await
}

/// 泛型核心（Runtime 抽象）：测试用 MockRuntime 驱动，生产路径零改动。
///
/// `state` 只服务于**结论自动交付**（复用主会话共享提交入口
/// [`crate::commands::process::submit_user_message`]，零新机制）。
/// `origin` 区分用户轮次与 Leader 派发轮次：前者只可能被 Exec 在途态放行
/// 过门禁，后者自身即消费方（见 [`auto_deliver_gate`]）。
async fn side_chat_send_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: &AppState,
    session_key: String,
    message: String,
    file_access: bool,
    web_search: bool,
    model: Option<String>,
    references: Option<Vec<ChatReference>>,
    effort: Option<String>,
    provider: Option<String>,
    project_dir: Option<String>,
    turn_id: Option<String>,
    origin: SendOrigin,
) -> Result<Value, String> {
    let key = session_key.trim().to_string();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let message = message.trim();
    if message.is_empty() {
        return Err("消息内容不能为空".to_string());
    }
    // 系统操作恒关：硬件约束，不随前端开关变化
    let permissions = ToolPermissions {
        file_access,
        web_search,
        system_automation: false,
    };
    let model_ref = model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let effort_ref = effort.as_deref().map(str::trim).filter(|e| !e.is_empty());
    // 提供商：与 model 成对才有意义（单给 provider 无模型 → 视为未指定）
    let provider_ref = provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|_| model_ref.is_some());

    // ── references 解析（与主会话同一公共函数）：拼接到用户消息前 ──
    // skill / knowledge / workflow 读 plugin 资源，quote 回落 label，带
    // content 的引用（sidechat）直接注入 content；capture 不在通用分支
    // （Side Chat 无视觉工具，落未知类型提示）。
    // 判重基准必须取**解析前**的用户原文：references 注入段每轮不同，
    // 拿拼装后的文本比对会让「执行中重发同一句」逃过去重。
    let raw_message = message.to_string();
    let message = match references.as_deref() {
        Some(refs) if !refs.is_empty() => {
            let resolved = nuphus::agent::references::resolve_references(refs).await;
            if resolved.is_empty() {
                message.to_string()
            } else {
                format!("{resolved}\n\n---\n\n{message}")
            }
        }
        _ => message.to_string(),
    };
    // 工作目录段：与 references 同为「本轮前缀」，但语义更强——它改写的是
    // 路径基准，不是参考资料。放在最前，模型先读基准再读内容。
    let message = match project_dir_header(project_dir.as_deref()) {
        Some(header) => format!("{header}\n\n{message}"),
        None => message,
    };
    // ── 消息前缀拼装（注入点 B → 注入点 D → 指令原文）──
    // 注入点 B（并行避让）事实源是 agent::parallel 的进程级单一状态
    // （RunGuard 写入）；注入点 D（Leader 派发性质）只对派发轮次叠加。
    // 顺序即优先级：模型先知环境事实，再知任务性质，最后读指令。
    let exec_title = nuphus::agent::parallel::snapshot().exec.map(|e| e.title);
    let message = compose_message(message, exec_title.as_deref(), origin);
    let message = message.as_str();

    // Side Chat 独立事件频道：按 session_key 绑定（rebuild 路径同样挂载，
    // 复用路径的执行体在构建时已带同一 key 的发射器）
    // 另留一份 app 句柄：轮次完成时要用它补发 `side_chat_turn_finished`
    // （刷新恢复路径依赖这条事件，不能只靠 invoke 返回值）。
    let app_for_turn = app.clone();
    let turn_id = turn_id.unwrap_or_default();
    let emitter: Arc<dyn EventEmitter> = Arc::new(SideChatEmitter {
        app,
        session_key: key.clone(),
    });

    // ── 取用：持锁只做 Map / 壳操作，guard 在本块结束即放（绝不持锁跨 await）──
    // reuse = 复用既有执行体；carried = 需新建时携带的既有会话（权限/模型/effort 变更）。
    let (reuse, carried, cancel, slot_signals) = {
        let mut map = slots()
            .lock()
            .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
        let slot = if origin == SendOrigin::LeaderDispatch {
            // 派发轮必须复用用户面板的壳：壳不存在 = 面板已关闭 / 会话已销毁。
            // 用户有权随时关闭伙伴，关闭后**不得静默重建**一个用户看不到的
            // 执行体（否则派发在面板外幽灵运行、结果无处呈现）。用户 send
            // 路径保持 or_insert（prepare 已建壳，此处兜底旧前端 / 重建竞态）。
            map.get_mut(&key)
                .ok_or_else(|| "临时伙伴会话已关闭（用户关闭了伙伴面板），派发终止".to_string())?
        } else {
            map.entry(key.clone())
                .or_insert_with(|| Slot::shell(permissions, model_ref, effort_ref, provider_ref))
        };
        let rebuild = !slot.activated
            || slot.permissions != permissions
            || slot.model.as_deref() != model_ref
            || slot.provider.as_deref() != provider_ref
            || slot.effort.as_deref() != effort_ref;
        let (reuse, carried) = if rebuild {
            // 携带既有会话重建：schema 必须与当前开关一致（否则模型会看到
            // 已关闭类别的工具并调用，走进没有 UI 应答的审批轮询）。
            // 若上一次执行还在途（壳被顶替），先置中断把它停掉。
            slot.cancel.store(true, Ordering::SeqCst);
            let carried = slot.runner.take().map(|r| r.session().clone());
            *slot = Slot::shell(permissions, model_ref, effort_ref, provider_ref);
            (None, carried)
        } else {
            match slot.runner.take() {
                Some(runner) => (Some(runner), None),
                None => return Err(BUSY_MESSAGE.to_string()),
            }
        };
        // 壳进入执行态：后续同 key 的并发 send 走 BUSY 分支
        slot.activated = true;
        // 在途态 + 判重基准在此刻落定：append 命令要靠 running 区分
        // 「执行中可入队」与「空闲请直接发送」，靠 last_message 做整轮去重。
        slot.running = true;
        // ── W1（并行协作契约）：伙伴在途写进程级事实源 ──
        // Leader 的派发 brief 与伙伴自己的消息前缀都从这里取「并行深度」；
        // 收尾在 ReturnGuard::drop（按 key 匹配清，见 W2）。
        nuphus::agent::parallel::set_side_start(&key);
        // 在途轮次来源落定：派发轮在途时 Leader 可追加（唯一豁免），用户轮
        // 在途时派发 busy 拒绝——见 side_chat_dispatch_impl 的在途分支。
        slot.current_origin = Some(origin);
        slot.last_message = raw_message.clone();
        // 必须在可能的重建**之后**取：放回时凭这个实例认壳，重建后旧实例已失效
        let cancel = slot.cancel.clone();
        // 重新建路径的 signals 此刻还是 None（runner 尚未构建），外面回填
        let slot_signals = slot.signals.clone();
        (reuse, carried, cancel, slot_signals)
    };

    // 守卫先建：构建失败即 return 时也能由 Drop 复位壳
    let mut guard = ReturnGuard {
        key: key.clone(),
        cancel: cancel.clone(),
        runner: None,
    };
    // 身份称呼：Side Chat 前端不传 relation（主会话才带），取 AppState 的
    // relation_cache —— 它由启动时 relation.json 播种、主会话消息 / SoulPage
    // 保存时更新，是不新开 IPC 通道的唯一权威来源。回落规则在 core 单点。
    let user_label = resolve_user_label(state);
    match reuse {
        Some(runner) => guard.runner = Some(runner),
        None => match build_runner(
            permissions,
            model_ref,
            provider_ref,
            message,
            carried,
            &cancel,
            emitter,
            effort_ref,
            &user_label,
        ) {
            Ok((runner, signals)) => {
                guard.runner = Some(runner);
                // 回填隔离信号句柄：append 命令只认壳上的这份，不碰主会话的
                // state.signals。壳若已被 close / 下一次重建顶替（ptr 不等），
                // 句柄随本次执行一起丢弃，绝不写到别人的壳上。
                if let Ok(mut map) = slots().lock() {
                    if let Some(slot) = map.get_mut(&key) {
                        if Arc::ptr_eq(&slot.cancel, &cancel) {
                            slot.signals = Some(signals);
                        } else {
                            tracing::debug!(
                                "side_chat: 构建期间壳已被顶替，信号句柄丢弃（key={}）",
                                key
                            );
                        }
                    }
                }
            }
            Err(e) => return Err(e),
        },
    }

    // 复用路径的队列可能在上一轮结束的瞬间被塞进残留项（Drop 已清，
    // 这里再兜一次）：本轮开跑前的队列必须是空的，否则追加会被算到
    // 「上一轮已结束、用户以为没生效」的语义错位里。
    if let Some(signals) = slot_signals.as_ref() {
        let mut state = nuphus::state::SignalState::write(signals);
        state.append_queue.clear();
    }

    // 轮次起点复位中断标志：上一轮「停止」/ 重建击杀置的 true 必须清零，
    // 否则复用路径继承同一 cancel 实例，下一轮首个迭代边界就被判取消。
    // 重建路径刚换了新实例（Slot::shell），此处为 no-op，不会擦掉对旧在途
    // 执行的击杀信号（那发生在换实例之前）。
    cancel.store(false, Ordering::SeqCst);
    let outcome = guard.run_turn(message, cancel.as_ref()).await;
    // ctx 读数必须在 `drop(guard)` **之前**取：归还后 runner 已放回壳里，
    // 这条路径上就无法再拿到本轮结束时的会话占用了。
    // 口径与主会话输入框完全一致（session::context_occupancy = 最近一次 API
    // 的 input+output，官方读数；0 = 尚无读数 → 前端显示 `--`）。
    let ctx_used = guard
        .runner
        .as_ref()
        .map(|r| r.session().context_occupancy())
        .unwrap_or(0);
    // 快照同样在 drop 之前落：`ReturnGuard::drop` 会把 running 置回 false，
    // 若之后才写 last_turn，就存在「running=false 且 last_turn=None」的瞬间——
    // 刷新恰好落在这一瞬，两条恢复路径都会落空，回复彻底丢失。
    if let Ok((_ok, reply, steps)) = &outcome {
        if let Ok(mut map) = slots().lock() {
            if let Some(slot) = map.get_mut(&key) {
                if Arc::ptr_eq(&slot.cancel, &cancel) {
                    slot.last_turn = Some(TurnResult {
                        turn_id: turn_id.clone(),
                        reply: normalize_reply(reply),
                        steps: steps_payload(steps),
                        ctx_used,
                    });
                }
            }
        }
    }
    // 显式归宿（紧随执行结束）：后续发送立刻能复用，不必等本函数返回
    drop(guard);

    // 执行归档（对话历史）：成功 / 失败 / 中断都记一条，失败不阻断返回。
    archive_after_turn(message, model_ref, &outcome).await;

    match outcome {
        Ok((success, reply, steps)) => {
            // ── 结论自动交付（并行协作契约）──
            // 门禁：本轮是 Leader 派发轮 → 结论直推 Leader；非派发轮（用户指令）
            // 不自动推（要推手动「直接提交」）。没有跨轮判断。
            if auto_deliver_gate(success, &reply, origin == SendOrigin::LeaderDispatch) {
                let item = nuphus::agent::parallel::DeliveryItem {
                    turn_id: turn_id.clone(),
                    reply: normalize_reply(&reply),
                    ts: nuphus::tools::background_tasks::now_ms().to_string(),
                };
                deliver_side_conclusion(&app_for_turn, state, item).await;
            }
            let payload = json!({
                "reply": normalize_reply(&reply),
                "steps": steps_payload(&steps),
                // 伙伴的 ctx 消耗（token，估算口径同主会话）
                "ctx_used": ctx_used,
                "turn_id": turn_id.clone(),
                // 用户中途停止（面板「停止」/关闭面板/重置）：轮次以
                // 「任务已被用户中断」收尾（run_free 取消分支，success=false）。
                // Leader 据此结构化区分「正常完成」与「被用户打断」，决定
                // 重派或改走 Exec——不能只靠 reply 文案判断。success=true 时
                // cancel 迟到置位不算中断。
                "interrupted": !success && cancel.load(Ordering::SeqCst),
            });
            // 补发完成事件：覆盖「刷新发生在执行中」——新页面只能靠它知道本轮收尾。
            // 前端按 turn_id 判重，正常路径（invoke 已落气泡）会跳过。
            // turn_finished 不带 question：派发轮 brief 已在起点经
            // side_chat_dispatch_question 显式落为用户消息（不存在闭合后补）；
            // 普通轮用户消息前端已落。此处再带 question 只会让前端补出第二条。
            let _ = app_for_turn.emit(
                "side-chat-event",
                json!({
                    "session_key": key,
                    "event": {
                        "type": "side_chat_turn_finished",
                        "turn_id": turn_id,
                        "reply": payload["reply"],
                        "steps": payload["steps"],
                        "ctx_used": ctx_used,
                    }
                }),
            );
            Ok(payload)
        }
        Err(e) => Err(e),
    }
}

// ============================================================================
// Leader 派发（后端通道：Rust 直选壳，不经前端）
// ============================================================================

/// AppHandle for the dispatch bridge path（工具 executor 无 Tauri State 访问，
/// 模式对齐 ext_agent / render / video 的桥注册）。
static DISPATCH_APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// main.rs setup 调用：存 AppHandle + 注册 side_chat_dispatch 桥实现。
///
/// 与 ext_agent 的 `init_bridge` 同款：nuphus lib 不依赖 Tauri，桌面壳在启动时
/// 把 fn 指针注入 [`nuphus::side_chat_bridge`]；未注册（headless 构建）时工具
/// 经桥诚实报错，绝不伪造派发结果。
pub fn init_dispatch_bridge(app: &tauri::AppHandle) {
    let _ = DISPATCH_APP.set(app.clone());
    nuphus::side_chat_bridge::register_side_chat_dispatch_impl(bridge_dispatch);
    tracing::info!("[side_chat] side_chat_dispatch bridge registered");
}

/// 桥入口（同步 fn）：在 Leader 的 tokio 上下文内驱动派发闭环。
/// 模式对齐 ext_agent 的 `bridge_dispatch`（run_blocking + Handle::block_on）。
fn bridge_dispatch(params: &serde_json::Value) -> Result<String, String> {
    let app = DISPATCH_APP
        .get()
        .ok_or_else(|| {
            "side_chat_dispatch 桥接未初始化（桌面壳未注册 side_chat bridge）".to_string()
        })?
        .clone();
    let brief = params
        .get("brief")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "brief 必填（字符串）".to_string())?
        .to_string();
    nuphus::tools::builtin::run_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let state = app.state::<AppState>();
        // 桥契约是「工具参数 JSON → 工具结果 JSON 字符串」：Value 序列化后返回
        rt.block_on(side_chat_dispatch_impl(&app, state.inner(), brief))
            .map(|payload| payload.to_string())
    })
}

/// Leader 派发临时伙伴（桌面壳薄入口）：选壳逻辑在 impl，参数只有 brief——
/// 无 session_key（Rust 侧选壳）、无前端开关（权限/模型取壳自带值）。
#[tauri::command]
pub async fn side_chat_dispatch(app: tauri::AppHandle, brief: String) -> Result<Value, String> {
    let state = app.state::<AppState>();
    side_chat_dispatch_impl(&app, state.inner(), brief).await
}

/// 派发闭环（泛型核心）：选壳 → 在途分流 → 空闲走 [`side_chat_send_impl`]
/// 的取壳 / 执行 / 交付全链路（origin = [`SendOrigin::LeaderDispatch`]）。
///
/// 选壳：取 [`slots()`] 里第一个壳。无 → Err（用户尚未打开过伙伴面板）。
/// 多个 → 任取第一个（现实单面板，并发多面板是未落地形态）。壳在
/// `activated == false`（打开面板即建、还没跑过轮）时同样可派发——首次
/// send/dispatch 走既有 rebuild 路径建 runner。
///
/// 在途分流（大王 2026-11-04 规则，同一时刻只有一个伙伴轮，来源在轮次起点
/// 落定于 `slot.current_origin`）：
/// - **用户轮在途**（用户在面板用伙伴）→ `busy: true`（reason=user_active），
///   正常回执非 Err，Leader 据此降级（改走 Exec 或等待）；
/// - **派发轮在途** → 唯一豁免：新 brief 作为追加任务入壳的隔离队列（判重 /
///   入队口径与 `side_chat_append` 完全一致），经迭代边界 drain 后进伙伴
///   上下文。**非必要不追加**——这是全系统唯一「伙伴执行中仍接收新任务」
///   的场景，用户轮没有这个口子；
/// - **空闲** → 完整派发（`dispatch_fresh`）。
/// 派发入池计划：选壳结果的四态分流（规则见 `side_chat_dispatch_impl` 文档）。
enum DispatchPlan {
    /// 伙伴空闲 → 走 `side_chat_send_impl` 完整派发链路。
    Fresh {
        key: String,
        permissions: ToolPermissions,
        model: Option<String>,
        effort: Option<String>,
        provider: Option<String>,
    },
    /// 派发轮在途 → 追加豁免：brief 已入壳的隔离追加队列，待迭代边界消费。
    Append {
        key: String,
        queued: bool,
        pending: usize,
    },
    /// 追加内容与本轮派发主指令重复 → 忽略（与 `side_chat_append` 同口径）。
    Duplicate,
    /// 用户轮在途（用户正在面板用伙伴）→ busy 拒绝，回执带原因。
    Busy,
}

async fn side_chat_dispatch_impl<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    brief: String,
) -> Result<Value, String> {
    let brief = brief.trim();
    if brief.is_empty() {
        return Err("派发 brief 不能为空".to_string());
    }
    // 选壳 + 在途分流入池：持锁只做 Map / 壳操作（enqueue 是内存队列写，与
    // side_chat_append 同款在锁内完成），emit 移出锁外，绝不持锁跨 await。
    let plan = {
        let map = slots()
            .lock()
            .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
        // 选壳判据 = 壳存在（打开面板即建壳，见 side_chat_prepare），不要求
        // activated——那只是「走过一轮 send」的并发防护标志，不是派发前置。
        // 无壳 → Err：从未打开面板，或打开后又关闭（dispose_slot 移出 Map）。
        // 两种状态行为一致（拒绝派发，绝不在用户看不到的地方执行），文案合并。
        let (key, slot) = map
            .iter()
            .next()
            .ok_or_else(|| "无临时伙伴会话（伙伴面板未打开或已关闭），派发终止".to_string())?;
        if slot.running {
            // 在途：按轮次来源分流。派发轮在途 = 追加豁免（同一时刻只有一个
            // 伙伴轮，来源在轮次起点已落定于 slot.current_origin）。
            if slot.current_origin != Some(SendOrigin::LeaderDispatch) {
                DispatchPlan::Busy
            } else {
                let signals = slot
                    .signals
                    .clone()
                    .ok_or_else(|| "临时伙伴执行体尚未就绪，请稍候".to_string())?;
                // 判重口径与 side_chat_append 一致：与本轮主指令重复没有合法语义
                if nuphus::mobile_append::is_duplicate_of_last(&slot.last_message, brief) {
                    DispatchPlan::Duplicate
                } else {
                    let queued = nuphus::mobile_append::enqueue(&signals, brief.to_string());
                    let pending = nuphus::state::SignalState::read(&signals)
                        .append_queue
                        .len();
                    DispatchPlan::Append {
                        key: key.clone(),
                        queued,
                        pending,
                    }
                }
            }
        } else {
            DispatchPlan::Fresh {
                key: key.clone(),
                permissions: slot.permissions,
                model: slot.model.clone(),
                effort: slot.effort.clone(),
                provider: slot.provider.clone(),
            }
        }
    };
    match plan {
        // 用户正在面板执行 → busy（正常回执非错误，Leader 据此降级 Exec/等待）
        DispatchPlan::Busy => Ok(json!({
            "busy": true,
            "reason": "user_active",
            "detail": "用户正在临时伙伴面板执行任务，派发拒绝；可改走 Exec 或等待"
        })),
        // 追加内容与主指令重复 → 忽略（非错误）
        DispatchPlan::Duplicate => Ok(json!({
            "busy": false,
            "appended": false,
            "duplicate": true,
            "detail": "追加内容与本轮派发主指令重复，已忽略"
        })),
        // 追加豁免：brief 已入队，伙伴窗口显「已入队」气泡（Leader 派发来源），
        // 迭代边界 drain 后进伙伴上下文并转正（append_queue_updated）。
        DispatchPlan::Append {
            key,
            queued,
            pending,
        } => {
            let _ = app.emit(
                "side-chat-event",
                json!({
                    "session_key": key,
                    "event": {
                        "type": "side_chat_dispatch_question",
                        "question": brief,
                        "queued": true,
                    }
                }),
            );
            Ok(json!({
                "busy": false,
                "appended": queued,
                "duplicate": !queued,
                "pending": pending,
                "detail": if queued {
                    "已追加进在途派发轮（唯一豁免：非必要不追加）"
                } else {
                    "同一内容已在队列中，忽略"
                }
            }))
        }
        DispatchPlan::Fresh {
            key,
            permissions,
            model,
            effort,
            provider,
        } => dispatch_fresh(app, state, brief, key, permissions, model, effort, provider).await,
    }
}

/// 空闲派发路径（原 `side_chat_dispatch_impl` 尾部）：壳参数原样回传 +
/// brief 起点显式 + send_impl 全链路。
async fn dispatch_fresh<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    brief: &str,
    key: String,
    permissions: ToolPermissions,
    model: Option<String>,
    effort: Option<String>,
    provider: Option<String>,
) -> Result<Value, String> {
    // 壳自带参数原样回传：send_impl 取壳逻辑的 rebuild 判据（权限 / 模型 / 商 /
    // effort 四项）全部不命中——不会重建执行体，也不会误置中断标志。壳上的
    // model/effort/provider 本就是 model_ref/effort_ref/provider_ref 形态
    // （trim + 非空过滤后的值），回传必然相等。
    // 派发轮次的 turn_id 由后端生成（无前端参与）：交付判重与 send_id 派生都要它。
    let turn_id = uuid::Uuid::new_v4().to_string();
    // 派发 brief 显式化（大王 2026-11-04 决策：Leader 派发必须在伙伴执行
    // 窗口可见，避免黑盒与指令阻塞）：执行开始前就把 brief 作为用户气泡推到
    // 伙伴窗口（带「Leader 派发」来源）。user 消息只来自此处与用户手动发送，
    // turn_finished 不补 user（后端不带 question，前端无补落逻辑）。
    let _ = app.emit(
        "side-chat-event",
        json!({
            "session_key": key,
            "event": {
                "type": "side_chat_dispatch_question",
                "turn_id": turn_id,
                "question": brief,
            }
        }),
    );
    match side_chat_send_impl(
        app.clone(),
        state,
        key,
        brief.to_string(),
        permissions.file_access,
        permissions.web_search,
        model,
        None,
        effort,
        provider,
        None,
        Some(turn_id),
        SendOrigin::LeaderDispatch,
    )
    .await
    {
        Ok(mut payload) => {
            payload["busy"] = json!(false);
            Ok(payload)
        }
        // 竞态兜底：预判空闲后、send_impl 取壳前被用户 send 抢先取走执行体
        // → 与 Busy 计划同口径的回执（用户轮在途，附原因）
        Err(e) if e == BUSY_MESSAGE => Ok(json!({
            "busy": true,
            "reason": "user_active",
            "detail": "用户正在临时伙伴面板执行任务，派发拒绝；可改走 Exec 或等待"
        })),
        Err(e) => Err(e),
    }
}

/// 打开面板即建壳（大王 2026-11-04 定）：前端挂载时同步面板当前配置，
/// **不依赖用户先发一条消息**——派发选壳只要求壳存在，不是 activated。
///
/// 为什么不复用 `side_chat_state` 建壳：state 是**只读查询**（刷新恢复也靠它，
/// 不能被写入副作用污染）；配置同步是独立意图，单独一条命令语义清晰。
///
/// 规则：
/// - 无壳 → 按面板配置建壳（activated=false：首次 send/dispatch 走既有
///   rebuild 路径建 runner，与「没 prepare 过」完全同构）；
/// - 有壳且空闲 → 更新壳配置（面板改了模型/权限，壳跟随；下次 send 的
///   rebuild 判据因此对齐，派发轮也拿到最新配置）；
/// - 有壳且执行中 → 不动（在途轮不换配置，配置变更留给下一轮 rebuild）。
#[tauri::command]
pub async fn side_chat_prepare(
    session_key: String,
    file_access: bool,
    web_search: bool,
    model: Option<String>,
    effort: Option<String>,
    provider: Option<String>,
) -> Result<Value, String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    // 系统操作恒关：与 side_chat_send_impl 同一口径（硬件约束，不随前端变化）
    let permissions = ToolPermissions {
        file_access,
        web_search,
        system_automation: false,
    };
    let model_ref = model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let effort_ref = effort.as_deref().map(str::trim).filter(|e| !e.is_empty());
    let provider_ref = provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|_| model_ref.is_some());

    let mut map = slots()
        .lock()
        .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
    match map.get_mut(key) {
        Some(slot) => {
            if !slot.running {
                slot.permissions = permissions;
                slot.model = model_ref.map(str::to_string);
                slot.effort = effort_ref.map(str::to_string);
                slot.provider = provider_ref.map(str::to_string);
            }
            Ok(json!({ "created": false }))
        }
        None => {
            map.insert(
                key.to_string(),
                Slot::shell(permissions, model_ref, effort_ref, provider_ref),
            );
            Ok(json!({ "created": true }))
        }
    }
}

/// 会话状态查询：**刷新恢复的唯一入口**。
///
/// 背景缺陷：`side_chat_send` 的结果只经 invoke 返回值传递，而 webview 刷新会
/// 丢掉那个 Promise——执行体在 Rust 侧照常跑完，界面却永远收不到回复，表现为
/// 「刷新后正在执行的气泡消失，跑完了也不回来」。
///
/// 两个字段各覆盖一半场景：
/// - `running` —— 刷新时轮次仍在跑：前端据此恢复「执行中」态，继续消费
///   `side-chat-event` 的实时事件，并在完成事件到达时落气泡；
/// - `last_turn` —— 刷新时轮次已结束（或完成事件落在页面重载窗口里被丢弃）：
///   前端按 `turn_id` 判断本地是否已有这条回复，没有就补一条。
///
/// 另带 `pending_deliveries`：交付箱积压条数（Finalizing 拒收后落箱、尚未
/// 补投的结论）。它不属于单个会话壳，故壳不存在时也照常返回。
///
/// 只读：不改任何状态、不推进队列，重复调用安全。
#[tauri::command]
pub async fn side_chat_state(session_key: String) -> Result<Value, String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let map = slots()
        .lock()
        .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
    let Some(slot) = map.get(key) else {
        // 壳不存在 = 后端已无该会话（整机重启 / 已关闭）：前端按空态处理，
        // 不报错——刷新后的常见情形，报错只会变成一条无意义的 toast。
        return Ok(json!({
            "exists": false,
            "running": false,
            "last_turn": null,
            "pending_deliveries": nuphus::agent::parallel::delivery_pending(),
        }));
    };
    Ok(json!({
        "exists": true,
        "running": slot.running,
        "last_turn": slot.last_turn.as_ref().map(|t| json!({
            "turn_id": t.turn_id.clone(),
            "reply": t.reply.clone(),
            "steps": t.steps.clone(),
            "ctx_used": t.ctx_used,
        })),
        // 交付箱积压（Finalizing 拒收落箱、尚未补投的结论条数）——
        // 前端角标数据源，与 last_turn 同属「刷新后要知道的事」。
        "pending_deliveries": nuphus::agent::parallel::delivery_pending(),
    }))
}

/// 关闭 / 重置共用的销毁路径：移出 Map + 置中断。在途执行凭共享的 cancel
/// 实例收到中断，其迟到的放回因壳已不在而自动丢弃。
fn dispose_slot(session_key: &str) {
    let key = session_key.trim();
    if key.is_empty() {
        return;
    }
    let removed = slots().lock().ok().and_then(|mut map| map.remove(key));
    if let Some(slot) = removed {
        slot.cancel.store(true, Ordering::SeqCst);
        // slot（含 runner 与内存会话）随本块结束丢弃：不落盘、无残留
        tracing::debug!("side_chat: 会话已销毁（key={key}）");
    }
}

/// 停止当前临时会话的在途执行（「停止」按钮）：不销毁会话、不清消息。
///
/// 只置位该壳的 `cancel`——`run_free` 循环与在途工具（registry 持有同一
/// 实例）在迭代边界观测，封口未配对工具后返回「任务已被用户中断」，作为
/// 普通回复呈现（不是错误态）。会话与已封口的工具结果保留，下一轮 send
/// 复用执行体（send 在轮次起点复位标志，停止不会毒化后续轮次）。
/// 无对应会话（尚未发送过 / 已关闭）时为空操作。
#[tauri::command]
pub async fn side_chat_cancel(session_key: String) -> Result<(), String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let map = slots()
        .lock()
        .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
    if let Some(slot) = map.get(key) {
        slot.cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// 执行中追加指令：**不新建轮次**，把消息入队到该临时会话的隔离信号队列，
/// 由 `run_free` 的迭代边界 drain 注入（`sub_task_loop.rs` 的
/// `append_queue` 消费点），与主会话 `append_instruction` 同一语义。
///
/// 与 [`side_chat_send`] 的分工是硬边界，不是偏好：
/// - `running == true` → 只能走这里（send 会被 BUSY 拒绝）。入队即刻生效于
///   本轮的下一个迭代边界，不产生新的 assistant 回复；
/// - `running == false` → 只能走 send。此处明确报错而不是静默入队——
///   空闲态没有消费者，入队等于把消息丢进黑洞，用户却以为发出去了。
///
/// 队列宿主是执行体自带的隔离 `SharedSignals`（见 [`Slot::signals`]），
/// 绝不写主会话的 `state.signals`，否则追加会注入 Leader 的上下文。
#[tauri::command]
pub async fn side_chat_append(
    session_key: String,
    message: String,
    project_dir: Option<String>,
) -> Result<Value, String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let text = message.trim();
    if text.is_empty() {
        return Err("追加内容不能为空".to_string());
    }
    let mut map = slots()
        .lock()
        .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
    let slot = map
        .get_mut(key)
        .ok_or_else(|| "临时会话不存在，请重新发送".to_string())?;
    if !slot.running {
        return Err("当前没有在途执行，请直接发送".to_string());
    }
    let signals = slot
        .signals
        .clone()
        .ok_or_else(|| "临时会话执行体尚未就绪，请稍候".to_string())?;
    // 与主会话同一判据（整轮有效、无时间窗口）：执行期提交同一内容没有
    // 合法语义——要追加新内容，内容必然不同。
    if nuphus::mobile_append::is_duplicate_of_last(&slot.last_message, text) {
        tracing::info!("[side_chat] 丢弃重复追加指令（与本轮主指令相同）");
        return Ok(json!({ "queued": false, "duplicate": true, "pending": 0 }));
    }
    let queued = nuphus::mobile_append::enqueue(
        &signals,
        match project_dir_header(project_dir.as_deref()) {
            Some(header) => format!("{header}\n\n{text}"),
            None => text.to_string(),
        },
    );
    let pending = nuphus::state::SignalState::read(&signals)
        .append_queue
        .len();
    // enqueue 返回 false 的另一个原因：同一内容已在队列里等着（连续两次追加
    // 同一句）。对用户而言这与「与本轮指令重复」是同一件事——已忽略，不是失败。
    Ok(json!({ "queued": queued, "duplicate": !queued, "pending": pending }))
}

/// 取消一条**尚未消费**的追加指令：按内容匹配从该会话的隔离队列移除。
///
/// 为什么按内容而不是下标（主会话 `remove_append_queue_item` 用下标）：侧边
/// 面板的 UI 队列与后端队列在「刷新后又追加」场景会漂移——刷新续命只恢复
/// localStorage 里的消息（queued 标记被清掉），后端队列里刷新前那条仍在，
/// 此后新追加的下标整体后移，按下标取消会删错人。按内容匹配则永远命中用户
/// 眼前点的那条。
///
/// 匹配口径与 [`side_chat_append`] 的入队格式严格对应：队列项是
/// `format!("{header}\n\n{text}")`（带了项目目录头）或原文；`mobile_append::
/// enqueue` 的队列内判重已保证同一文本不会排队两次，首个匹配即唯一目标。
///
/// `removed: false` 不是错误：条目已被迭代边界 drain（消费者抢先一步），
/// 调用方应按「已生效」收尾，而不是当失败重试。
#[tauri::command]
pub async fn side_chat_remove_append_item(
    session_key: String,
    message: String,
) -> Result<Value, String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let text = message.trim();
    if text.is_empty() {
        return Err("追加内容不能为空".to_string());
    }
    let mut map = slots()
        .lock()
        .map_err(|_| "side_chat 状态锁已中毒".to_string())?;
    let slot = map
        .get_mut(key)
        .ok_or_else(|| "临时会话不存在，请重新发送".to_string())?;
    let signals = slot
        .signals
        .clone()
        .ok_or_else(|| "临时会话执行体尚未就绪，请稍候".to_string())?;
    let mut state = nuphus::state::SignalState::write(&signals);
    let suffix = format!("\n\n{text}");
    let removed = match state
        .append_queue
        .iter()
        .position(|queued| queued == text || queued.ends_with(&suffix))
    {
        Some(i) => {
            state.append_queue.remove(i);
            true
        }
        None => false,
    };
    Ok(json!({ "removed": removed, "pending": state.append_queue.clone() }))
}

/// 面板关闭 / 主会话切换：销毁该临时会话（含在途执行）。
#[tauri::command]
pub async fn side_chat_close(session_key: String) -> Result<(), String> {
    dispose_slot(&session_key);
    Ok(())
}

/// 清空对话：丢弃执行体与会话，下次 send 重建（面板保持打开）。
#[tauri::command]
pub async fn side_chat_reset(session_key: String) -> Result<(), String> {
    dispose_slot(&session_key);
    Ok(())
}

/// 保存临时对话为 md 存档：`nuphus_data_dir()/side-chat/{YYYYMMDD-HHmmss}-{key 前 8 位}.md`。
///
/// `content` 是前端组装好的 md 全文；目录不存在则创建。返回写入路径字符串。
/// 落点复用 `nuphus::utils::nuphus_data_dir()`（运行时写入的权威根，
/// `NUPHUS_DATA_DIR` 可覆盖），不自己拼 `%APPDATA%`。
#[tauri::command]
pub async fn side_chat_save(session_key: String, content: String) -> Result<String, String> {
    let key = session_key.trim();
    if key.is_empty() {
        return Err("session_key 不能为空".to_string());
    }
    let dir = nuphus::utils::nuphus_data_dir().join("side-chat");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("创建 side-chat 存档目录失败: {e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let key_prefix: String = key.chars().take(8).collect();
    let path = dir.join(format!("{stamp}-{key_prefix}.md"));
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("写入临时对话存档失败: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// 空回复兜底：前端整段渲染，空串会渲染出一个空气泡。
fn normalize_reply(reply: &str) -> String {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        "（临时助手没有返回内容）".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 步骤摘要里优先取哪些字段：先看「目标」，再看名字。
/// 参数结构由模型决定，这里只做「有则显示」，不做强解析。
const SUMMARY_KEYS: [&str; 6] = ["path", "file_path", "query", "url", "pattern", "name"];

/// 一步的一行摘要：取参数字符串里最像「目标」的那个，截到 80 字符。
fn step_summary(params: &Value) -> String {
    let text = match params {
        Value::String(s) => s.clone(),
        Value::Object(map) => SUMMARY_KEYS
            .iter()
            .find_map(|k| map.get(*k).and_then(|v| v.as_str().map(str::to_string)))
            .or_else(|| map.values().find_map(|v| v.as_str().map(str::to_string)))
            .unwrap_or_default(),
        _ => String::new(),
    };
    text.chars().take(80).collect()
}

/// 执行步骤 → 前端紧凑结构：工具名 + 成败 + 一行摘要。
///
/// 只暴露这三项：完整参数与工具输出可能很长，侧边面板放不下、也不该放。
/// 面板侧据此渲染「已用 N 个工具」折叠条，展开才看明细。
fn steps_payload(steps: &[nuphus::ExecutionStep]) -> Vec<Value> {
    steps
        .iter()
        .map(|s| {
            json!({
                "tool": s.tool,
                "ok": s.result.as_ref().map(|r| r.success).unwrap_or(false),
                "summary": step_summary(&s.params),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(tag: &str) -> String {
        format!("side-chat-test-{tag}-{}", std::process::id())
    }

    /// 权限构造：system_automation 恒 false，且工具集按它物理裁剪——
    /// system_shell / process_kill / system_info / system_env_get /
    /// system_sleep 既不进 schema 也无执行入口。
    ///
    /// 注意本测试测的是 [`nuphus::ToolRegistry::exec_with_permissions`] 本身，
    /// **不含** Side Chat 的精确放开（那由 `side_chat_restored_subset_is_read_only`
    /// 覆盖）：放开发生在 build_runner 的 restore_tools 一步，不改这个函数的语义。
    #[test]
    fn permissions_force_system_automation_false() {
        let permissions = ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: false,
        };
        assert!(!permissions.system_automation);

        let registry = nuphus::ToolRegistry::exec_with_permissions(permissions);
        let names: Vec<String> = registry
            .get_schemas()
            .into_iter()
            .map(|s| s.function.name)
            .collect();
        assert!(
            !names.iter().any(|n| n == "system_shell"),
            "system_shell 必须被物理剔除: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "process_kill"),
            "process_kill 必须被物理剔除: {names:?}"
        );
        // system_ 前缀三工具的声明类别也必须是 SystemAutomation：曾以 Core
        // 声明躲过裁剪，执行期才被前缀判成 SystemAutomation，空转 300s 审批
        for tool in ["system_info", "system_env_get", "system_sleep"] {
            assert!(
                !names.iter().any(|n| n == tool),
                "{tool} 必须被物理剔除: {names:?}"
            );
        }
        // 文件 / 联网开关为真时对应类别可用
        assert!(names.iter().any(|n| n == "Read"), "文件工具应在: {names:?}");
        assert!(
            names.iter().any(|n| n == "web_search"),
            "联网工具应在: {names:?}"
        );
    }

    /// 精确放开子集的封闭性：schema 含放开的 shell 子集，但不含进程终止 /
    /// 桌面控制类 / 浏览器——「放开的只有白名单」必须被钉住，否则
    /// restore_tools 的白名单一旦被绕开，Side Chat 就拿到了完整系统权限。
    #[test]
    fn side_chat_restored_subset_matches_whitelist() {
        let permissions = ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: false,
        };
        let mut registry = nuphus::ToolRegistry::exec_with_permissions(permissions);
        registry.restore_tools(SIDE_CHAT_RESTORED_TOOLS);
        let names: Vec<String> = registry
            .get_schemas()
            .into_iter()
            .map(|s| s.function.name)
            .collect();

        for want in SIDE_CHAT_RESTORED_TOOLS {
            assert!(names.iter().any(|n| n == want), "{want} 应放开: {names:?}");
            assert!(registry.has_tool(want), "{want} 应有执行入口: {names:?}");
        }
        for forbidden in [
            "process_kill",
            "desktop_click",
            "desktop_input",
            "desktop_mouse",
            "desktop_mouse_drag",
            "desktop_semantic_execute",
            "browser_navigate",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "{forbidden} 不应放开: {names:?}"
            );
        }
    }

    /// 两个开关都关时，FileAccess / WebSearch 类别整体不出现在工具集里。
    #[test]
    fn exec_with_permissions_drops_disabled_categories() {
        let registry = nuphus::ToolRegistry::exec_with_permissions(ToolPermissions {
            file_access: false,
            web_search: false,
            system_automation: false,
        });
        let names: Vec<String> = registry
            .get_schemas()
            .into_iter()
            .map(|s| s.function.name)
            .collect();
        for forbidden in [
            "Read",
            "Write",
            "Edit",
            "web_search",
            "web_extract",
            "http_request",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "关闭类别的工具 {forbidden} 不应出现: {names:?}"
            );
        }
        // Core 类（只读信息 / 记忆检索）不受开关影响
        assert!(
            names.iter().any(|n| n == "knowledge_search"),
            "Core 工具应保留: {names:?}"
        );
    }

    /// close 后 Map 为空（壳与执行体一并销毁）。
    #[tokio::test]
    async fn close_removes_slot_from_map() {
        let key = test_key("close");
        let slot = Slot::shell(
            ToolPermissions {
                file_access: true,
                web_search: true,
                system_automation: false,
            },
            Some("test-model"),
            None,
            None,
        );
        let cancel = slot.cancel.clone();
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(key.clone(), slot);
        }
        side_chat_close(key.clone()).await.expect("close 应成功");
        let empty = {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            !map.contains_key(&key)
        };
        assert!(empty, "close 后 Map 不应再持有该会话");
        assert!(
            cancel.load(Ordering::SeqCst),
            "close 必须置中断标志（在途执行据此停止）"
        );
    }

    /// reset 与 close 同一条销毁路径：Map 清空 + 中断置位。
    #[tokio::test]
    async fn reset_discards_runner() {
        let key = test_key("reset");
        let slot = Slot::shell(
            ToolPermissions {
                file_access: false,
                web_search: false,
                system_automation: false,
            },
            None,
            None,
            None,
        );
        let cancel = slot.cancel.clone();
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(key.clone(), slot);
        }
        side_chat_reset(key.clone()).await.expect("reset 应成功");
        let empty = {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            !map.contains_key(&key)
        };
        assert!(empty, "reset 后 Map 不应再持有该会话（runner 已丢弃）");
        assert!(cancel.load(Ordering::SeqCst), "reset 必须置中断标志");
    }

    /// 追加指令：只在执行中受理，且只写本会话的**隔离**队列。
    ///
    /// 两条边界各钉一个断言：
    /// - 空闲态必须报错（静默入队 = 消息进黑洞，用户却以为发出去了）；
    /// - 入队落在 `Slot::signals` 指的那份信号状态上，主会话队列零污染。
    #[tokio::test]
    async fn append_requires_running_and_uses_isolated_queue() {
        let key = test_key("append");
        let signals = nuphus::state::new_shared_signals();
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(
                key.clone(),
                Slot::shell(
                    ToolPermissions {
                        file_access: false,
                        web_search: false,
                        system_automation: false,
                    },
                    None,
                    None,
                    None,
                ),
            );
        }
        let idle = side_chat_append(key.clone(), "再查一下".to_string(), None).await;
        assert!(idle.is_err(), "空闲态必须拒绝追加，而不是入队等下一轮");

        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get_mut(&key).expect("壳应在");
            slot.running = true;
            slot.last_message = "先看看目录".to_string();
            slot.signals = Some(signals.clone());
        }
        let ok = side_chat_append(key.clone(), "顺便看下配置".to_string(), None)
            .await
            .expect("执行中应受理");
        assert_eq!(ok["queued"], json!(true));
        assert_eq!(
            nuphus::state::SignalState::read(&signals).append_queue,
            vec!["顺便看下配置".to_string()],
            "追加必须落在该会话的隔离队列上"
        );

        // 与本轮主指令相同 → 判重丢弃（整轮有效，无时间窗口）
        let dup = side_chat_append(key.clone(), "先看看目录".to_string(), None)
            .await
            .expect("判重不是错误，是明确回执");
        assert_eq!(dup["duplicate"], json!(true));
        assert_eq!(
            nuphus::state::SignalState::read(&signals)
                .append_queue
                .len(),
            1,
            "重复指令不得入队"
        );

        side_chat_close(key).await.expect("清理临时会话");
    }

    /// 取消追加：按内容匹配移除；条目已被消费时 `removed:false`（不是错误）。
    ///
    /// 钉两条口径：
    /// - 匹配要穿透项目目录头（入队格式是 `format!("{header}\n\n{text}")`）；
    /// - 取不到的条目返回 removed:false 而不是 Err——调用方据此按「已生效」
    ///   收尾，不会把它当失败反复重试。
    #[tokio::test]
    async fn remove_append_item_matches_by_content_not_index() {
        let key = test_key("append-cancel");
        let signals = nuphus::state::new_shared_signals();
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let mut slot = Slot::shell(
                ToolPermissions {
                    file_access: false,
                    web_search: false,
                    system_automation: false,
                },
                None,
                None,
                None,
            );
            slot.running = true;
            slot.signals = Some(signals.clone());
            map.insert(key.clone(), slot);
        }
        // 带项目目录头入队（与 side_chat_append 的写入格式一致）
        nuphus::mobile_append::enqueue(&signals, "[工作目录: D:/repo]\n\n附带说明".to_string());

        // 按用户原文取消：必须命中带头的那条
        let res = side_chat_remove_append_item(key.clone(), "附带说明".to_string())
            .await
            .expect("取消应成功");
        assert_eq!(res["removed"], json!(true));
        assert!(
            nuphus::state::SignalState::read(&signals)
                .append_queue
                .is_empty(),
            "取消后隔离队列必须为空"
        );

        // 已被消费 / 本就不在：removed:false，不是 Err
        let gone = side_chat_remove_append_item(key.clone(), "附带说明".to_string())
            .await
            .expect("取不到不是错误");
        assert_eq!(gone["removed"], json!(false));

        side_chat_close(key).await.expect("清理临时会话");
    }

    /// 状态查询：壳不存在时返回 exists=false（不报错），存在时带回 running 与快照。
    #[tokio::test]
    async fn state_reports_running_and_last_turn() {
        let key = test_key("state");
        let missing = side_chat_state(key.clone()).await.expect("不存在也不报错");
        assert_eq!(missing["exists"], json!(false));
        assert_eq!(missing["running"], json!(false));

        let signals = nuphus::state::new_shared_signals();
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let mut slot = Slot::shell(
                ToolPermissions {
                    file_access: false,
                    web_search: false,
                    system_automation: false,
                },
                None,
                None,
                None,
            );
            slot.running = true;
            slot.signals = Some(signals);
            slot.last_turn = Some(TurnResult {
                turn_id: "turn-1".to_string(),
                reply: "刷新后补上的回复".to_string(),
                steps: vec![],
                ctx_used: 1024,
            });
            map.insert(key.clone(), slot);
        }
        let state = side_chat_state(key.clone()).await.expect("查询应成功");
        assert_eq!(state["exists"], json!(true));
        assert_eq!(state["running"], json!(true));
        assert_eq!(state["last_turn"]["turn_id"], json!("turn-1"));
        assert_eq!(state["last_turn"]["reply"], json!("刷新后补上的回复"));
        assert_eq!(state["last_turn"]["ctx_used"], json!(1024));
        // 交付箱积压计数随状态下发（本二进制无投递路径被触发，故为 0）
        assert_eq!(state["pending_deliveries"], json!(0));

        side_chat_close(key).await.expect("清理临时会话");
    }

    /// 工作目录段：空 / 空白不注入；路径两端空白被裁掉，且必须自带优先级声明。
    #[test]
    fn project_dir_header_formats_and_skips_blank() {
        assert!(project_dir_header(None).is_none());
        assert!(project_dir_header(Some("")).is_none());
        assert!(project_dir_header(Some("   ")).is_none());
        let header = project_dir_header(Some("  E:/work/A  ")).expect("非空应生成");
        assert!(
            header.starts_with("[工作目录: E:/work/A]"),
            "实际: {header}"
        );
        assert!(
            header.contains("覆盖"),
            "必须写明覆盖主会话项目目录：{header}"
        );
    }

    /// Explore 哨兵：生成「无项目」段——既不是空白（不注入），也不是具体路径。
    #[test]
    fn project_dir_header_explore_sentinel_declares_no_project() {
        let header = project_dir_header(Some(EXPLORE_PROJECT)).expect("哨兵应生成头部");
        assert!(header.starts_with("[工作目录: 无"), "实际: {header}");
        assert!(
            header.contains("忽略"),
            "必须写明忽略环境段里的主会话项目目录：{header}"
        );
        // 与「跟随主会话」的 None 语义区分开：哨兵不是空白串
        assert!(project_dir_header(None).is_none());
        assert!(project_dir_header(Some("")).is_none());
    }

    /// 空回复兜底：trim 后为空 → 固定文案，非空 → 原样（去首尾空白）。
    #[test]
    fn normalize_reply_handles_empty_and_trims() {
        assert_eq!(normalize_reply("  答案  "), "答案");
        assert_eq!(normalize_reply("\n\t"), "（临时助手没有返回内容）");
        assert_eq!(normalize_reply(""), "（临时助手没有返回内容）");
    }

    /// 系统提示必须是一个正常 agent 的骨架，而不是「临时外来者」的自我声明。
    #[test]
    fn system_prompt_is_a_normal_agent() {
        let all_off = ToolPermissions {
            file_access: false,
            web_search: false,
            system_automation: false,
        };
        let prompt = system_prompt(all_off, "test-model", None, "用户");

        // 身份：协作助手 + 职责表（四大定位逐项在表内）
        assert!(prompt.contains("Nuphus 的协作助手"));
        assert!(prompt.contains("创意方案"));
        assert!(prompt.contains("探索调研"));
        assert!(prompt.contains("临时任务"));
        assert!(prompt.contains("核对审计"));
        // 输出契约 / 工具路由 / 代码地图 / 运行机制
        assert!(prompt.contains("输出契约"));
        assert!(prompt.contains("工具用法"));
        assert!(prompt.contains("代码地图"));
        assert!(prompt.contains("运行机制"));
        // 不把面板生命周期写成角色身份
        assert!(!prompt.contains("用完即弃"));
        assert!(!prompt.contains("一次性执行体"));
        assert!(!prompt.contains("临时对话"));

        // 复用 L0 骨架：仲裁链 + 交付律
        assert!(prompt.contains("仲裁链"));
        assert!(prompt.contains("交付之后不得再调用任何工具"));

        // 环境信息由 env_info_section 提供
        assert!(prompt.contains("当前模型: test-model"));

        // 权限只声明关闭项；边界由工具清单承载
        assert!(prompt.contains(
            "本会话关闭：文件读写 / 联网检索 / 桌面控制（点击/输入/拖拽）与浏览器自动化、进程终止"
        ));
        assert!(prompt.contains("工具清单即边界"));
        // 放开的只读子集同样从常量派生而非写死：白名单调整时这条断言自动跟随，
        // 不会在测试里留一份过期副本（对应「禁止硬编码」）
        assert!(prompt.contains(&SIDE_CHAT_RESTORED_TOOLS.join(" / ")));
        // 关闭项不得把已放开的子集一并否掉——否则模型会以为 shell 不可用
        assert!(!prompt.contains("本会话关闭：文件读写 / 联网检索 / 系统与桌面自动化"));

        // 构建纪律必须指向唯一事实源（知识库的 build-verification-policy），
        // 而非在提示词里复制条文
        assert!(prompt.contains("build-verification-policy"));
        assert!(prompt.contains("--release"));
        assert!(prompt.contains("假绿"));
        // 不得把发版流程手册当成验证纪律的事实源（两者在零行为改动上结论相反）
        assert!(!prompt.contains("先 skill_read 读 `git-pr-protocol`"));

        // 定位：创意方案 · 探索调研 · 临时任务 · 核对审计；回传经用户显式操作
        assert!(prompt.contains("创意方案 · 探索调研 · 临时任务 · 核对审计"));
        assert!(prompt.contains("送入主对话"));
        assert!(prompt.contains("直接提交"));
        // 工具用法：附件是本地绝对路径 / 显式选定的 skill 已随消息注入
        assert!(prompt.contains("用户消息里的 `[附件: 路径]`"));
        assert!(prompt.contains("内容已随消息注入"));
        // 超出范围的重活：不硬撑，给切入点与关键路径，交用户显式回传主会话
        assert!(prompt.contains("不硬撑"));
        assert!(prompt.contains("给出切入点与关键路径"));
        assert!(prompt.contains("由用户显式回传主会话推进"));
        // 已拆除的限制与矛盾点（逐项对应大王指示，防回潮）：
        // 自相矛盾的「不追后续」/ 窄化审计的「专挑错与漏」/ 与全文回传冲突的
        // 300 字上限与「只贴关键片段」/ 与工具清单重复的禁令排比
        assert!(!prompt.contains("不落产物"));
        assert!(!prompt.contains("专挑错与漏"));
        assert!(!prompt.contains("只贴关键片段"));
        assert!(!prompt.contains("300 字"));
        assert!(!prompt.contains("你不能写文件"));

        let all_on = ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: true,
        };
        assert!(system_prompt(all_on, "test-model", None, "用户").contains("本会话未关闭任何能力"));
    }

    /// 身份称呼：默认「用户」不注入；配置了称呼才落一行，且与 Leader L0 同一规则
    /// （core 单点 `address_rule`，禁止在各提示词里各写一份判定）。
    #[test]
    fn system_prompt_injects_user_label_only_when_configured() {
        let perms = ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: true,
        };

        // 默认 / "user"（大小写不敏感）都不注入——把默认值写成事实会让模型
        // 以为用户显式要求被叫做「用户」
        for label in ["用户", "user", "USER", "User"] {
            let prompt = system_prompt(perms, "test-model", None, label);
            assert!(
                !prompt.contains("称呼："),
                "label={label} 不应注入称呼行：\n{prompt}"
            );
        }

        // 配置了称呼 → 身份段一行，且落在「人格」之前
        let prompt = system_prompt(perms, "test-model", None, "大王");
        assert!(prompt.contains("称呼：大王"));
        let addr_at = prompt.find("称呼：大王").expect("称呼行应在");
        let persona_at = prompt.find("人格：专业 理性 务实").expect("人格行应在");
        assert!(addr_at < persona_at, "称呼行必须位于人格行之前");

        // 空白串按未配置处理（trim 后为空 → 回落默认，不注入空行）
        let prompt = system_prompt(perms, "test-model", None, "   ");
        assert!(!prompt.contains("称呼："));
    }

    /// 停止命令：置位当前壳的 cancel；空 key 拒绝；不存在的 key 为空操作。
    /// 同时钉死「停止不毒化后续轮次」的前提——send 在轮次起点复位标志。
    #[tokio::test]
    async fn cancel_sets_flag_and_tolerates_missing_session() {
        let key = format!("cancel-test-{}", std::process::id());
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            map.insert(
                key.clone(),
                Slot::shell(
                    ToolPermissions {
                        file_access: true,
                        web_search: true,
                        system_automation: false,
                    },
                    None,
                    None,
                    None,
                ),
            );
        }
        side_chat_cancel(key.clone()).await.expect("cancel 应成功");
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                map.get(&key).expect("壳应在").cancel.load(Ordering::SeqCst),
                "cancel 必须置位中断标志"
            );
        }
        // 空 key 拒绝；不存在的会话为空操作（无在途执行可停）
        assert!(side_chat_cancel(String::new()).await.is_err());
        side_chat_cancel("no-such-side-chat-key".to_string())
            .await
            .expect("不存在的会话也应是 Ok");
        // 清理，避免污染同包其他用例
        side_chat_close(key.clone()).await.expect("close 应成功");
    }

    /// 参数校验：空 key / 空消息在动 LLM 之前就被拒。
    #[tokio::test]
    async fn send_rejects_empty_arguments() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let state = AppState::default();

        let err = side_chat_send_impl(
            handle.clone(),
            &state,
            "  ".to_string(),
            "hi".to_string(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            SendOrigin::User,
        )
        .await
        .expect_err("空 session_key 必须报错");
        assert!(err.contains("session_key"));

        let err = side_chat_send_impl(
            handle,
            &state,
            test_key("empty-msg"),
            "   ".to_string(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            SendOrigin::User,
        )
        .await
        .expect_err("空消息必须报错");
        assert!(err.contains("消息"));
    }

    /// 自动交付门禁（纯函数）：轮次成功 + 回复非空 + **本轮是 Leader 派发轮**。
    /// 只认当前轮是否派发来的，没有跨轮判断（exec_in_flight 已移除，大王
    /// 2026-11-04）：派发轮结论直推，非派发轮（用户指令）一律不自动推；
    /// 失败 / 空回复轮也不是结论。
    #[test]
    fn auto_deliver_gate_only_dispatch_pushes() {
        // 派发轮 + 成功 + 有内容 → 投
        assert!(auto_deliver_gate(true, "结论", true));
        // 非派发轮（用户自己发的）→ 一律不投，与内容无关
        assert!(!auto_deliver_gate(true, "结论", false));
        assert!(!auto_deliver_gate(true, "闲聊", false));
        // 派发轮但失败 / 中断（success=false）→ 事故报告不是结论
        assert!(!auto_deliver_gate(false, "执行失败: xxx", true));
        // 派发轮但回复空白 → 无可交付内容
        assert!(!auto_deliver_gate(true, "  \n ", true));
    }

    /// 派发在途分流（大王 2026-11-04 规则，`side_chat_dispatch_impl` 选壳后：
    /// 同一时刻只有一个伙伴轮，来源在轮次起点落定于 `current_origin`）：
    /// - 用户轮在途 → busy 拒绝（reason=user_active），不碰队列；
    /// - 派发轮在途 → 唯一豁免：brief 入隔离追加队列（appended=true）；
    /// - 追加内容与本轮主指令重复 → duplicate 忽略（与 side_chat_append 同口径）。
    #[tokio::test]
    async fn dispatch_in_flight_routes_by_origin() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let state = AppState::default();
        let perms = ToolPermissions {
            file_access: false,
            web_search: false,
            system_automation: false,
        };

        // ① 用户轮在途 → busy(user_active)
        let key_user = test_key("dispatch-busy");
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map
                .entry(key_user.clone())
                .or_insert_with(|| Slot::shell(perms, None, None, None));
            slot.activated = true;
            slot.running = true;
            slot.current_origin = Some(SendOrigin::User);
        }
        let r = side_chat_dispatch_impl(&handle, &state, "任务".to_string())
            .await
            .expect("busy 是正常回执，不是 Err");
        assert_eq!(r["busy"], serde_json::json!(true));
        assert_eq!(r["reason"], serde_json::json!("user_active"));
        // 立即清壳：选壳是「任取第一个 activated」（进程级 slots），两个激活壳
        // 共存时②会选中①的壳——先清再建，②才测到派发轮在途的追加分支。
        side_chat_close(key_user.clone())
            .await
            .expect("close 应成功");

        // ② 派发轮在途 → 追加入队（唯一豁免）
        let key_disp = test_key("dispatch-append");
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map
                .entry(key_disp.clone())
                .or_insert_with(|| Slot::shell(perms, None, None, None));
            slot.activated = true;
            slot.running = true;
            slot.current_origin = Some(SendOrigin::LeaderDispatch);
            slot.signals = Some(std::sync::Arc::new(std::sync::RwLock::new(
                nuphus::state::SignalState::default(),
            )));
            slot.last_message = "主任务".to_string();
        }
        let r = side_chat_dispatch_impl(&handle, &state, "追加任务".to_string())
            .await
            .expect("派发轮在途派发应走追加，不是 Err");
        assert_eq!(r["busy"], serde_json::json!(false));
        assert_eq!(r["appended"], serde_json::json!(true));
        // 队列实证：brief 确实进了壳的隔离队列（待迭代边界 drain）
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get(&key_disp).expect("壳应在");
            let pending =
                nuphus::state::SignalState::read(slot.signals.as_ref().expect("signals 应已建"))
                    .append_queue
                    .len();
            assert_eq!(pending, 1, "追加 brief 应入队");
        }

        // ③ 追加内容与本轮主指令重复 → duplicate 忽略（不入队）
        let r = side_chat_dispatch_impl(&handle, &state, "主任务".to_string())
            .await
            .expect("重复追加是忽略，不是 Err");
        assert_eq!(r["duplicate"], serde_json::json!(true));
        assert_eq!(r["appended"], serde_json::json!(false));

        // 清理，避免污染同包其他用例
        side_chat_close(key_disp.clone())
            .await
            .expect("close 应成功");
    }

    /// 打开面板即建壳（大王 2026-11-04）：壳存在 = 派发前置，不再要求
    /// 「先跟伙伴对话」（activated 只是走过一轮 send 的并发防护标志）。
    #[tokio::test]
    async fn prepare_creates_shell_for_dispatch() {
        let key = test_key("prepare-shell");
        // 无壳 → 建壳（created=true，未激活、未在途）
        let r = side_chat_prepare(key.clone(), true, true, Some("m1".to_string()), None, None)
            .await
            .expect("prepare 应成功");
        assert_eq!(r["created"], serde_json::json!(true));
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get(&key).expect("壳应已建");
            assert!(
                !slot.activated,
                "prepare 建壳不得置 activated——首次轮要走 rebuild 路径建 runner"
            );
            assert!(!slot.running);
            assert_eq!(slot.model.as_deref(), Some("m1"));
        }
        // 再 prepare（空调壳）→ 更新配置不重复建
        let r = side_chat_prepare(
            key.clone(),
            true,
            false,
            Some("m2".to_string()),
            Some("high".to_string()),
            None,
        )
        .await
        .expect("重复 prepare 应成功");
        assert_eq!(r["created"], serde_json::json!(false));
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get(&key).expect("壳应在");
            assert_eq!(slot.model.as_deref(), Some("m2"));
            assert_eq!(slot.effort.as_deref(), Some("high"));
            assert!(!slot.permissions.web_search, "空闲壳的配置应跟随面板");
        }
        // 派发选壳：壳存在即可（不再要求 activated）——选壳条件是
        // `slots().iter().next()`，prepare 建壳后必然通过；`dispatch_impl`
        // 之后的链路要走 resolve_client（未知模型会回退主模型 → 真发请求），
        // 测试不进入执行链路，避免真实 LLM 调用。选壳 + 在途分流的完整
        // 行为由 dispatch_in_flight_routes_by_origin（手建壳）覆盖。
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                map.get(&key).is_some(),
                "prepare 后壳必须存在（派发选壳的唯一前置）"
            );
        }
        // 在途壳不换配置（在途轮不被打扰）：配置变更留给下一轮 rebuild
        {
            let mut map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get_mut(&key).expect("壳应在");
            slot.running = true;
            slot.current_origin = Some(SendOrigin::User);
        }
        side_chat_prepare(key.clone(), false, false, None, None, None)
            .await
            .expect("在途 prepare 应成功（不动配置）");
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            let slot = map.get(&key).expect("壳应在");
            assert_eq!(
                slot.model.as_deref(),
                Some("m2"),
                "在途轮不得被 prepare 换配置"
            );
        }
        // 清理
        side_chat_close(key.clone()).await.expect("close 应成功");
    }

    /// 用户中途关闭伙伴（close / reset 销毁壳）后，派发不得静默重建执行体：
    /// `SendOrigin::LeaderDispatch` 必须复用已存在的壳，无壳直接 Err——在
    /// resolve_client / 建 runner 之前返回，测试不会触达真实 LLM。
    #[tokio::test]
    async fn dispatch_requires_existing_shell() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let state = AppState::default();
        let key = test_key("dispatch-no-shell");
        let err = side_chat_send_impl(
            handle,
            &state,
            key.clone(),
            "hi".to_string(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            SendOrigin::LeaderDispatch,
        )
        .await
        .expect_err("派发轮无壳必须 Err，不得静默重建");
        assert!(
            err.contains("已关闭") || err.contains("不存在"),
            "错误应说明会话已关闭，实际: {err}"
        );
        // 壳不得被重建（否则派发在用户看不到的面板外幽灵运行）
        {
            let map = slots().lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                map.get(&key).is_none(),
                "派发轮不得创建壳——面板已关闭就是关闭"
            );
        }
    }

    /// 消息前缀拼装（纯函数）：用户轮次零前缀（与拆分前逐字节一致）；
    /// 派发轮次叠注入点 D；两段并存时顺序必须是 B（并行避让）→ D（任务性质）
    /// → 指令原文——先知环境事实，再知任务性质，最后读指令。
    #[test]
    fn compose_message_orders_avoidance_before_dispatch_note() {
        // 用户轮次 + Exec 空闲 → 原样（零行为改动）
        assert_eq!(
            compose_message("hi".to_string(), None, SendOrigin::User),
            "hi"
        );
        // 用户轮次 + Exec 在途 → 仅 B 段，形态与拆分前一致
        let user_b = compose_message("hi".to_string(), Some("任务A"), SendOrigin::User);
        assert!(user_b.starts_with("[并行状态: 主会话正在执行任务「任务A」]"));
        assert!(user_b.ends_with("\n\nhi"));
        assert!(!user_b.contains("[Leader 派发]"));
        // 派发轮次 + Exec 空闲 → 仅 D 段，指令垫底
        let dispatch_only = compose_message("brief".to_string(), None, SendOrigin::LeaderDispatch);
        assert!(dispatch_only.starts_with("[Leader 派发]"));
        assert!(dispatch_only.ends_with("\n\nbrief"));
        // 派发轮次 + Exec 在途 → B 在前、D 居中、指令垫底
        let both = compose_message(
            "brief".to_string(),
            Some("任务A"),
            SendOrigin::LeaderDispatch,
        );
        let b_end = both.find("以磁盘内容为准。").expect("B 段应在");
        let d_at = both.find("[Leader 派发]").expect("D 段应在");
        let user_at = both.rfind("brief").expect("指令原文应在");
        assert!(
            b_end < d_at && d_at < user_at,
            "顺序必须 B → 派发段 → 指令原文，实际: {both}"
        );
        // 派发段的交付要求：自包含 + 可回溯（伙伴无跨轮上下文）
        assert!(both.contains("自包含"));
        assert!(both.contains("可回溯"));
    }

    /// 交付前缀纯函数：轮次号取前 8 位、未经复核的采信声明必须在最前，
    /// 可回溯要求与「未验证」标注指引齐全（丞相据此决定采信或忽略）。
    #[test]
    fn delivery_header_declares_unverified_parallel_conclusion() {
        let header = delivery_header("turn-abcdef123456");
        assert!(
            header.starts_with("[来自临时伙伴的结论 · 轮次 turn-abc]"),
            "实际: {header}"
        );
        assert!(header.contains("丞相："));
        assert!(header.contains("未经其复核"));
        assert!(header.contains("可回溯"));
        assert!(header.contains("未验证"));
        // 前缀与结论正文直接相接（无分隔符）：投递消息 = header + reply
        assert!(delivery_header("x").ends_with("再决定是否采用。"));
        // 多字节 id 按字符切，不 panic 不乱码
        let _ = delivery_header("轮次-abcdefghij");
    }

    /// side_chat_save：content 落盘到 NUPHUS_DATA_DIR 隔离的临时目录，
    /// side-chat 子目录自动创建、文件内容一致、文件名带时间戳与 key 前 8 位。
    #[tokio::test]
    async fn side_chat_save_writes_md_under_isolated_data_dir() {
        // env 是进程级共享：加锁串行化本测试的写与恢复，避免波及其他测试
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // side-chat 子目录刻意不预建：断言命令自己创建
        let base = std::env::temp_dir().join(format!("nuphus-side-chat-save-{nanos}"));
        let previous = std::env::var("NUPHUS_DATA_DIR").ok();
        std::env::set_var("NUPHUS_DATA_DIR", &base);

        let key = test_key("save");
        let content = "# 临时对话存档\n\n正文";
        let saved = side_chat_save(key.clone(), content.to_string())
            .await
            .expect("保存应成功");

        // 先恢复 env 再做断言（断言失败也不留下被改写的进程环境）
        match previous {
            Some(v) => std::env::set_var("NUPHUS_DATA_DIR", v),
            None => std::env::remove_var("NUPHUS_DATA_DIR"),
        }

        let path = std::path::PathBuf::from(&saved);
        assert_eq!(
            path.parent().expect("存档路径有父目录"),
            base.join("side-chat"),
            "必须落在 nuphus_data_dir()/side-chat 下: {saved}"
        );
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let key_prefix: String = key.chars().take(8).collect();
        assert!(
            name.contains(&key_prefix),
            "文件名须带 session_key 前 8 位: {name}"
        );
        assert!(name.ends_with(".md"), "存档是 md 文件: {name}");
        // 时间戳前缀 {YYYYMMDD-HHmmss}：8 位日期 + '-' + 6 位时间 + '-' + key 前 8 位
        let (date, rest) = name.split_at(8);
        assert!(
            date.chars().all(|c| c.is_ascii_digit()),
            "日期应为 8 位数字: {date}"
        );
        assert!(rest.starts_with('-'), "日期后接 '-': {name}");
        let (time, rest) = rest[1..].split_at(6);
        assert!(
            time.chars().all(|c| c.is_ascii_digit()),
            "时间应为 6 位数字: {time}"
        );
        assert!(rest.starts_with('-'), "时间戳后接 '-': {name}");
        assert!(
            rest[1..].starts_with(&key_prefix),
            "'-' 后接 session_key 前 8 位: {name}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("存档应可读"),
            content,
            "落盘内容必须与入参一致"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// references 解析复用主会话公共函数（同一份实现，禁止两处各写一份）：
    /// skill 读文件拼接 / quote 回落 label / content 优先于 label。
    #[tokio::test]
    async fn side_chat_references_resolve_via_shared_function() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("nuphus-side-chat-refs-{nanos}"));
        let entry = root
            .join("plugin")
            .join("skills")
            .join("demo")
            .join("SKILL.md");
        std::fs::create_dir_all(entry.parent().expect("有父目录")).expect("建临时 skill 目录");
        std::fs::write(&entry, "SKILL 全文").expect("写临时 SKILL.md");

        // skill：读 plugin/skills/{id}/SKILL.md，按通用格式拼接
        let skill = ChatReference {
            ref_type: "skill".into(),
            id: "demo".into(),
            label: "演示技能".into(),
            content: None,
        };
        let out = nuphus::agent::references::resolve_reference_text(&root, &skill)
            .await
            .expect("读到文件不该跳过");
        assert_eq!(out, "[skill] 演示技能:\nSKILL 全文");

        // quote 无 content → 回落 label，只发类型标记 + 原文（不套通用格式）
        let quote = ChatReference {
            ref_type: "quote".into(),
            id: "q1".into(),
            label: "选中原文".into(),
            content: None,
        };
        let out = nuphus::agent::references::resolve_reference_text(&root, &quote)
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[quote]\n选中原文");

        // content 优先于 label：sidechat 新类型无需特判，走通用格式
        let mut sidechat = quote.clone();
        sidechat.ref_type = "sidechat".into();
        sidechat.label = "来自临时对话".into();
        sidechat.content = Some("回复全文".into());
        let out = nuphus::agent::references::resolve_reference_text(&root, &sidechat)
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[sidechat] 来自临时对话:\n回复全文");

        // content 也优先于 quote 的 label（显示与注入分离）
        let mut quote_with_content = quote.clone();
        quote_with_content.content = Some("注入全文".into());
        let out = nuphus::agent::references::resolve_reference_text(&root, &quote_with_content)
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[quote]\n注入全文");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// ChatReference 反序列化兼容：缺 content 字段 → None（旧前端不改也能用）。
    #[test]
    fn chat_reference_without_content_deserializes_as_none() {
        let json = r#"{"type":"skill","id":"demo","label":"演示技能"}"#;
        let r: ChatReference = serde_json::from_str(json).expect("旧前端 payload 必须能反序列化");
        assert!(r.content.is_none());
        assert_eq!(r.ref_type, "skill");

        let json = r#"{"type":"sidechat","id":"s1","label":"来自临时对话","content":"全文"}"#;
        let r: ChatReference = serde_json::from_str(json).expect("新 payload 必须能反序列化");
        assert_eq!(r.content.as_deref(), Some("全文"));
    }
}

// ═══════════════════════════════════════════════════════════════════
// 执行归档（对话历史）
//
// 定位：side chat 的**可追溯记录**，每轮结束自动落一条，关面板 / 切主会话后
// 仍能回看。与「保存」的分工：
//   - 保存（side_chat_save）= 用户主动导出一份 md 全文，落在 side-chat/*.md；
//   - 归档（本段）        = 每轮自动追加一行 JSONL，落在 side-chat/history.jsonl。
//
// 隔离边界一字不改：归档**不进 memory、不进上下文、不参与派发**，只是把
// 已经发生的事实写到磁盘。写入失败一律静默降级（不阻断本轮回复）。
// ═══════════════════════════════════════════════════════════════════

/// 一条执行归档。字段只放「事后要能回答发生了什么」的最小集。
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SideChatHistoryEntry {
    pub id: String,
    /// 本地时间 `2026-11-02 12:34:56`
    pub ts: String,
    /// 使用的模型（空 = 跟随主会话）
    pub model: String,
    pub question: String,
    pub reply: String,
    /// 工具步骤（与 steps_payload 同形），没有工具时为空数组
    pub steps: Value,
    /// 本轮是否正常完成（中断 / 失败也会归档，用于事后分辨）
    pub ok: bool,
}

/// 归档文件上限：只保留最近这么多轮，避免无限增长。
const HISTORY_CAP: usize = 200;
/// 历史面板一次最多取多少条（需求：最多 10 条最新）。
const HISTORY_LIST_DEFAULT: usize = 10;

fn history_file() -> std::path::PathBuf {
    nuphus::utils::nuphus_data_dir()
        .join("side-chat")
        .join("history.jsonl")
}

/// 读全部归档；坏行跳过（手工编辑 / 半截写入不应该让整个历史不可用）。
fn read_history() -> Vec<SideChatHistoryEntry> {
    let Ok(raw) = std::fs::read_to_string(history_file()) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<SideChatHistoryEntry>(l).ok())
        .collect()
}

/// 追加一条归档（尽力而为：目录不存在则建；失败只是没有历史，不影响回复）。
async fn archive_turn(entry: SideChatHistoryEntry) {
    let path = history_file();
    if let Some(dir) = path.parent() {
        if tokio::fs::create_dir_all(dir).await.is_err() {
            return;
        }
    }
    // 读-截断-写：归档量级小（≤200 行），比追加后再裁剪更简单可靠
    let mut all = read_history();
    all.push(entry);
    if all.len() > HISTORY_CAP {
        let drop_n = all.len() - HISTORY_CAP;
        all.drain(0..drop_n);
    }
    let mut body = String::new();
    for e in &all {
        if let Ok(s) = serde_json::to_string(e) {
            body.push_str(&s);
            body.push('\n');
        }
    }
    let _ = tokio::fs::write(&path, body).await;
}

/// 最近 N 条执行归档（新的在前）。`limit` 缺省 10，上限 HISTORY_CAP。
#[tauri::command]
pub async fn side_chat_history_list(
    limit: Option<usize>,
) -> Result<Vec<SideChatHistoryEntry>, String> {
    let n = limit.unwrap_or(HISTORY_LIST_DEFAULT).clamp(1, HISTORY_CAP);
    let mut all = read_history();
    all.reverse();
    all.truncate(n);
    Ok(all)
}

/// 删除一条归档（按 id）。找不到即视为已删除（幂等）。
#[tauri::command]
pub async fn side_chat_history_delete(id: String) -> Result<(), String> {
    let target = id.trim();
    if target.is_empty() {
        return Err("id 不能为空".to_string());
    }
    let mut all = read_history();
    let before = all.len();
    all.retain(|e| e.id != target);
    if all.len() == before {
        return Ok(());
    }
    let path = history_file();
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let mut body = String::new();
    for e in &all {
        if let Ok(s) = serde_json::to_string(e) {
            body.push_str(&s);
            body.push('\n');
        }
    }
    tokio::fs::write(&path, body)
        .await
        .map_err(|e| format!("写入归档失败: {e}"))
}

/// 一轮结束后写归档：question / reply / steps / 是否成功 / 使用模型。
/// 失败（中断、LLM 错误）也归档，`ok=false` —— 事后才能分辨「它当时是挂了还是没跑」。
async fn archive_after_turn(
    question: &str,
    model: Option<&str>,
    outcome: &Result<(bool, String, Vec<nuphus::ExecutionStep>), String>,
) {
    let (ok, reply, steps): (bool, String, Value) = match outcome {
        Ok((success, reply, steps)) => (
            *success,
            normalize_reply(reply),
            json!(steps_payload(steps)),
        ),
        Err(e) => (false, format!("执行失败: {e}"), json!([])),
    };
    let entry = SideChatHistoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        model: model.unwrap_or("").to_string(),
        question: question.to_string(),
        reply,
        steps,
        ok,
    };
    archive_turn(entry).await;
}
