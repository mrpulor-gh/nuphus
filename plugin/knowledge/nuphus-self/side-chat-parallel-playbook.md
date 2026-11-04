# 临时伙伴并行协作速查

> agent 操作纪律，不是用户文档。回答「伙伴和 Exec 怎么分工 / 什么时候会并行 / 结论怎么回流」时查本文；实现细节以代码为准（文末索引）。

## 分工契约

| | 临时伙伴（SideChat） | ExecAgent |
|---|---|---|
| 触发 | 用户直连（左侧面板）· Leader 后端派发（`side_chat_dispatch`） | Leader `task_dispatch` 派发 |
| 派发条件 | 内部长任务可拆解 + 并行确有效率 + 无用户在用伙伴，三条件缺一不可；busy=true = 用户在用，改走 Exec 或等待 | 按 goal_type 随时可派 |
| 体量 | 单轮 / 少轮，轻量，可丢弃 | 多步，要落盘成果 |
| 典型用途 | 顺手看别的项目、快速验证思路、核对审计 | 完整任务闭环 |
| 上下文 | 只活进程内存，不写会话库 | brief 自足 + 参考上下文注入 |
| 记忆 | 不写（`set_suppress_memory_write`） | 正常沉淀 |
| 台账 | 无（不进 task 面板） | `agent::task_run` 全量快照 |
| 工具 | 系统操作恒关；只读 shell 子集精确放开 | 按 goal_type 权限 |

**派发路径只有两条**：用户面板直连，Leader 经 `side_chat_dispatch` 后端直派（Rust 选壳、不经前端）。此外没有任何路径把任务派给伙伴，也不许新建这种路径。

## 三条通道

| 通道 | 方向 | 性质 |
|---|---|---|
| **Leader→伙伴派发** | Leader → 伙伴 | `side_chat_dispatch`（Leader 独占工具）：三条件缺一不可——内部长任务可拆解、并行确有效率、无用户在用伙伴。brief 自包含（伙伴无跨轮上下文）；busy=true = 用户在用伙伴，Leader 据此降级 |
| **伙伴→Leader 派发回传** | 伙伴 → Leader | 自动：**门禁 = 本轮由 Leader 派发 ‖ 主会话 Exec 在途**。轮次成功且回复非空才经 `submit_user_message`（`source="sidechat"`）入链。Running 期入队为追加指令（迭代边界 drain）；Finalizing 拒收后后台每 2s 轮询重投（≤60s）；仍拒则落交付箱，伙伴下一轮完成时 drain 补投（遇拒即停、剩余保序回箱） |
| **用户手动提交** | 伙伴 → 主会话 | 强制通道：任何轮次都能用，与派发态无关（「送入主对话」进输入栏 /「直接提交」直达主会话，两者都是全文） |

**没有用户开关**（大王 2026-11-03 定）：Leader 没派发时的闲聊结论推过去就是无关信息涌入，违背本面板的隔离职责。

## 三种并行组合

| 组合 | 关系 | 注意 |
|---|---|---|
| 伙伴 × Leader 主循环 | 互不阻塞（伙伴走独立 runner 与事件频道） | 结论 Running 期走追加指令入链，不新起轮次 |
| 伙伴 × Exec | 两个独立 runner **真并行**，最需避让 | 双向注入避让段：Exec brief（注入点 A）+ 伙伴消息前缀（注入点 B） |
| Exec × Exec | 主循环内 `task_dispatch` 同步阻塞 = 串行 | 一个结清才开下一个，无需避让 |

## 冲突裁遗嘱

双方都要写同一文件时，**以用户注意力所在为优先**：侧边是用户当下正在看的面板，Exec 避让。
避让纪律（注入点 A/B 同款文案）：写前先读现状；不做无差异覆盖；发现内容与预期不符时以当前磁盘内容为准；不假设对方已验证。

## 实现索引

| 关注点 | 位置 |
|---|---|
| 进程级单一事实源（ParallelState + DeliveryBox） | `src/agent/parallel.rs` |
| 门禁 / 交付 / 注入点 B·D / W1·W2 写入 | `src-tauri/src/commands/side_chat.rs` |
| Leader 派发入口（选壳 + busy 回执 + 桥注册） | `src-tauri/src/commands/side_chat.rs` `side_chat_dispatch` / `init_dispatch_bridge` |
| 派发工具定义（Leader 独占）与执行桥注入点 | `src/tools/builtin/side_chat_dispatch.rs`、`src/side_chat_bridge.rs` |
| Exec brief 避让注入（注入点 A）+ RunGuard 在途态 | `src/runtime/dispatch.rs`、`src/agent/task_run.rs` |
| Leader 每轮并行注记（注入点 C） | `src-tauri/src/commands/process.rs` |
| 前端双可见（11px 提示条 + pending 计数）+ 派发轮次 question 补落 | `frontend/src/main-window/chat/SideChatPanel.tsx`、`App.tsx` |
| L2 派发策略与并行纪律 | `src/agent/prompt.rs` `build_l2_leader()` |
