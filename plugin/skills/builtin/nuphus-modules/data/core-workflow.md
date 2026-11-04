# 工作流引擎（src/workflow/）

> 定位：以「单遍规划 → 确定性执行」为纲领的声明式工作流引擎——用编译期静态验证 + 13 种步骤语义 + 全局执行闸门 + Cron 调度 + ChatAgent 智能节点，把 LLM 规划出的 IR 变成可复现、可断点续连、可全程取证的桌面自动化执行体。
> 规模：33 文件约 16300 行（含测试与 JSON）。

## 结构

| 层 | 文件 | 职责 |
|---|---|---|
| 门面层 | `mod.rs`（989） | WorkflowEngine + 全局执行闸门 + 调度入口 + 双工件导出 |
| IR/契约层 | `types.rs`（621）/ `inputs.rs`（261）/ `references.rs`（340）/ `step_schema.json`（337） | 13 种 Action + Step + Condition + OnError + ScheduleConfig；外部输入契约；`{{var}}` 模板与 `var["k"]` 字段引用；给 AI 生成侧的唯一真相源 Schema |
| 编译层 | `compiler.rs`（1021） | 运行前静态验证（ID 唯一性、工具注册表/必填参数、regex 可编译、loop 上限、inputs 声明一致性、Call 目标存在性与循环调用链 DFS） |
| 执行层 | `executor/`（dispatch 384 + 各 step_* + lifecycle 132 + variables 516 + control_flow 53） | 按 Step 树递归执行（`#[async_recursion]`）；13 种步骤各一个执行器 |
| 横切层 | `scheduler.rs`（914）/ `store.rs`（1014）/ `chat_agent.rs`（421）/ `trace.rs`（532）/ `debug.rs`（302）/ `scoped_edit.rs`（696）/ `events.rs` / `hud_control.rs` | 持久化 Cron、工作流持久化、ChatAgent 配置、执行证据、编辑器调试、AI 局部编辑、事件总线、HUD 取消信号 |

## 功能作用

- **编译期拦截设计错误**：产出 `ValidationReport{errors 阻断 / warnings 提示 / diagnostics 机器可读定位}`，回传前端画布 ProblemsPanel。
- **确定性执行**：管道内不自动调用 LLM（只有 chat 步骤显式调用），失败直接传播、无自愈状态机；执行态全程可取消/暂停/断点续连/超时/重试（on_error 支持 abort/skip/retry/allow_codes）。
- **无人值守运行**：scheduler 持久化五段式 Cron（IANA 时区 + 固定间隔），并静态判定「会无限期停下来等人」的构造；wait 步骤人工确认暂停（HUD 轮询，30 分钟超时）。
- **观测与调试**：trace 把每次调用的输入/变量/输出/attempt 落盘为本地执行证据；debug 让画布用**同一套生产执行器**做冻结定义的 Node/Through 调试。
- **ChatAgent 智能节点**：ReAct 工具循环 + 工具白名单 + 四段式人设。

## 规则与坑

- 执行层只做类型校验，**不做隐式转换**（`types.rs:11`）。
- 纲领（`mod.rs:3-4`）：single-pass planning → deterministic execution；旧多轮对话/逐步调试/模拟/自愈状态机已移除，调试能力以 debug.rs 冻结快照形式重建。
- `scoped_edit` 仅提案式 AI 局部编辑：不执行、不落盘。
