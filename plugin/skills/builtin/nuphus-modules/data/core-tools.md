# 工具系统（src/tools/）

> 定位：Nuphus 全部 Agent 能力的唯一供给层。以 `ToolDef`（定义）与 `executor`（执行体）分离为核心模式，由 `ToolRegistry` 统一注册、分发、超时、取消与提示词渲染，覆盖文件/系统/记忆/搜索/生成/桌面/浏览器/进程/技能/知识/协作 11 个工具域。
> 规模：30 文件约 22394 行。

## 结构

四层：

| 层 | 代表文件 | 职责 |
|---|---|---|
| 注册表层 | `registry.rs`（1885） | ToolDef/ToolCtx/角色工具集/execute 分发/超时取消/提示词渲染——唯一调度中枢 |
| 通道层 | `browser_tools.rs`（689）/ `desktop_executors.rs`（856）/ `semantic_desktop.rs`（3202）/ `background_tasks.rs`（1444） | 按工具名前缀分流的特殊通道（不经 ToolDef.executor）；后台任务账本 + 取消事实编码 + 看门狗 |
| 定义层 | `definitions/`（file 1799 / system 1213 / memory 624 / process 284 / skill 79 / knowledge 87 / annotation 134 / schedule 39） | 每个文件就是一组 `register_*` 方法，ToolDef 与执行闭包同址定义 |
| 内置实现层 | `builtin/`（web 3607 五源聚合 + 四层降级 / http 708 含 SSRF 防护 / generation 1391 / planner 743 / experience 541 / ui_maps 662 / ext_agent 63 / tenet 217） | 重逻辑薄封装成工具定义 |

## 功能作用

- **模型能看到什么**：`get_schemas()` 产出 ToolDefinition 随 API 请求下发；`render_tools_for_prompt()` 另产一份「按组只有名字」的极简总览嵌进 system prompt。
- **唯一同步分发入口** `execute()`：按工具名分流到 browser/desktop/semantic 三个特殊通道或已注册 executor fn 指针；用 `spawn_blocking + catch_unwind + select!` 三段式把「panic 隔离、超时、取消」三件事一次做掉。
- **副作用收尾**：`background_tasks` 进程级后台任务账本（登记/杀进程树/保留项/看门狗）；`desktop_approval` 关键桌面操作一次性宿主审批；语义桌面（UIA）「观察→候选→执行→验证」闭环 + `desktop_verification` 只读后置条件求值。

## 规则与坑

- 本模块是**纯能力层**：不含 Agent 决策；`src-tauri/` 只经 ToolRegistry 构造函数与注入方法交互，IPC 命令桥不接触工具内部。
- 超时分级 + 取消结算有专门常量表（`registry.rs:22-44`）；提示词按组渲染并利用缓存。
