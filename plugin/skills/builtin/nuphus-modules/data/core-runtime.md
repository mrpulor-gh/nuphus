# runtime 统一运行时（src/runtime/）

> 定位：nuphus crate 的执行中枢。以「单一 Runtime + 三种运行模式（Leader / Workflow / Custom）」取代旧双 Agent 架构，统一拥有 ReAct 执行主循环、子任务分发、防护断路器，以及取消/暂停/追加指令信号的全程贯穿。
> 规模：12 文件约 9164 行。

## 结构

| 文件 | 职责 |
|---|---|
| `loop.rs`（879） | Runtime 主体：构建、生命周期、模式切换、断点续跑、执行中提炼 |
| `react_loop.rs`（2008） | **ReAct 主循环**（Leader/Custom 顶层，循环体在 `:59-1909`） |
| `dispatch.rs`（511） | `task_dispatch` 工具的实际处理（子任务派发） |
| `sub_task.rs` / `sub_task_loop.rs` / `sub_task_shell.rs` | SubTaskRunner 子任务执行体（590/1138/413）：自由执行主循环 + shell 流式 |
| `protection.rs`（651） | ProtectionGuard：死循环/连续错误/重复读检测器 |
| `workflow_agent.rs`（2058） | Workflow 模式独立执行器 |
| `tool_lifecycle.rs`（93） | ToolEndGuard：工具调用生命周期 RAII 守卫 |
| `mode.rs`（83） | `Mode{Leader,Workflow,Custom}` 枚举 |
| `tests.rs`（712） | 19 个模块级集成测试（MockApiClient） |

四层架构：入口层（loop.rs）→ 调度层（mode 路由 + dispatch）→ 执行层（react_loop / sub_task_loop / workflow_agent 三个平级 ReAct 循环）→ 横切层（protection / tool_lifecycle / sub_task_shell）。

## 功能作用

- **三模式路由**：Leader（默认，LLM 自主决策）/ Workflow（工作流设计器，独立 WorkflowAgent）/ Custom（用户自定义 Agent，L2 全替换 + 工具白名单）。
- **ReAct 主循环**：LLM 流式 → 工具执行 → 事件推送的主闭环；轮中增量快照（崩溃可恢复）；上下文预算三级防线。
- **子任务分发**：`task_dispatch` 把聚焦子任务交给 SubTaskRunner 并行执行。
- **生命周期控制**：取消/暂停/追加指令信号全程贯穿；`resume()` 断点续跑。

## 规则与坑

- 执行入口收敛：ReAct 循环整体 inline 进 `Runtime::react_loop()`，`ReactAgent` 退化为**纯状态容器**（`src/runtime/mod.rs:3-8`）。
- 安全链统一收敛到 `agent::exec_tool::check_tool_security()`，三个执行体共用同一入口（`src/runtime/mod.rs:9`）。
- 失控防护（死循环检测 ProtectionGuard、安全熔断、上下文溢出三级防线）全部在本模块落地。
