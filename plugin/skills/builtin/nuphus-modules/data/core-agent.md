# agent ReactAgent 引擎（src/agent/）

> 定位：Agent 基座层——**刻意不做执行循环**（循环在 runtime/），只提供循环所需的状态容器、纯函数与协议定义：ReactAgent 纯状态容器 + L0/L1/L2 三层提示词工厂 + 统一工具执行安全链 + 类型化事件协议 + 会话经验提炼与提醒队列。
> 规模：13 文件（12 rs + goal_warmups.toml）。

## 结构

| 文件 | 职责 |
|---|---|
| `mod.rs`（1303） | ReactAgent 状态容器 + AgentConfig + 提示词四级缓存 + 模型切换账本 |
| `common.rs`（903） | 流归一：AssistantEvent → ContentBlock、错误分类与重试判定、跨迭代文本边界 |
| `prompt.rs`（1521） | L0 宪法 / L1 运行时 / L2 元认知三层提示词，Leader + Exec + WorkAgent 三套 |
| `exec_tool.rs`（646） | 统一工具执行 + 五层安全链 + 分级熔断（`check_tool_security`） |
| `events.rs`（645） | `NuphusEvent` 类型化事件枚举（40+ 变体）+ EventEmitter |
| `distill.rs`（477） | 会话提炼引擎：分档阈值、归档、in-loop 硬止损 |
| `reminders.rs`（344） | 跨轮持久提醒队列（替换旧的单轮 pending_reminder） |
| `task_run.rs`（517） | ExecAgent 生命周期唯一真相（进程级单例 + 落盘 + RAII 守卫） |
| `turn_meta.rs` / `pause.rs` / `goal_types.rs` / `diagnostics.rs` | 轮次元数据单一数据源 / 暂停决策注册表 / GoalType 六分类 / reasoning 一致性诊断 |

## 功能作用

- **ReactAgent**：直接持有 LLM / Session / Tools / Policy，不引入 Agent trait 层级（`mod.rs:156-158`：Runtime owns the execution loop; ReactAgent holds session, tools, policy, and LLM connection）。
- **三层提示词体系**：L0 宪法（不可变铁律）/ L1 运行时（工具面、环境、技能索引）/ L2 元认知（人格与策略），三套 Agent 各一份。
- **五层安全工具执行链**：权限 → 安全 → 分类开关 → SecurityGuard → 分级熔断 → 纯执行。
- **经验提炼**：RefineTier 分档（85% 硬止损等），提炼结果归档为记忆条目。
- **事件协议**：NuphusEvent 40+ 变体是前后端事件契约的 Rust 侧真相源。

## 规则与坑

- 提炼轮禁工具必须**代码级**保证：`tools_suppressed` 开关让 build_request 不带 tools + react_loop 剔除模型产出的 tool_use 块，双保险（仅靠提示词模型不遵守，`mod.rs:541-544`）。
- 模型切换提示只进当次请求体（transient），**绝不入 session.messages**，否则每轮重发（`mod.rs:525-528`）。
- HUD 上屏判定必须收敛到 `hud_arbiter::is_hud_tool` 单一入口，本地副本漂移会导致「上了屏没人发终态帧」HUD 粘住（`mod.rs:769-771`）。
- 跨项目记忆注入按项目隔离，且检索键必须用 session_id 而非 entry id（`refine-{session_id}-{ts}` 不同值，曾造成检索断线，`mod.rs:607-611`）。
- `execute_tool_with_permission` 215 行单函数含六块关注点，文档建议拆分（`mod.rs:706-921`）。
