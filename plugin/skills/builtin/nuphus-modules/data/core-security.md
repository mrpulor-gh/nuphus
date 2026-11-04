# 安全体系（src/permissions.rs src/security/ src/filter/ src/hooks/ src/automation_gate.rs）

> 定位：Agent 每一次工具调用的「四道闸门 + 一条后处理链」——工具分类授权（permissions）→ 危险操作检测与用户审批（security）→ 外部内容注入防护（security/injection）→ 资源级互斥锁（automation_gate），辅以输出降 token 过滤（filter）与 Shell 生命周期钩子（hooks），共同把「模型能做什么」约束在可授权、可确认、可熔断、可审计的边界内。
> 规模：10 文件。

## 结构

四层：

| 层 | 文件 | 职责 |
|---|---|---|
| 授权层 | `permissions.rs`（295） | ToolCategory / ToolPermissions / PermissionOutcome / PermissionPolicy：粗粒度开关，决定「这类工具现在能不能用」，与设置页三个开关一一对应 |
| 检测与审批层 | `security/mod.rs`（533）/ `injection.rs`（488）/ `user_input.rs`（195）/ `approval.rs`（138） | SecurityGuard 危险检测三态判定 + 异步审批闭环；InjectionDetector 只扫工具输出；PendingInput / PendingApproval |
| 互斥层 | `automation_gate.rs`（625） | AutomationGate：可实例化的单槽互斥门（RAII 租约），ResourceClass / HoldKind |
| 后处理层 | `filter/`（rule 63 + filter 331）/ `hooks/mod.rs`（243） | ToolOutputFilter 规则执行 + 5 条内置规则；Shell 生命周期钩子运行时 |

## 功能作用

- **前置授权**：「文件读写 / 网络搜索 / 系统自动化」三组独立开关 + 工具→类别映射表（`permissions.rs:101-194`）。
- **危险检测与审批**：SecurityGuard 在参数解析后、真正执行前做危险模式检测，产出 `Allow / RequireConfirmation / Deny` 三态；需要确认的走「后端发事件 → 前端弹窗 → 轮询结果」异步闭环，支持 once / session 两种授权粒度。
- **注入防护**：只扫描**工具输出**（网页、外部文件等），经零宽字符/HTML 注释清洗 + 中英双语正则扫描 + 不可信边界标记（UNTRUSTED_BOUNDARY）后才进入 LLM 上下文。
- **资源互斥**：把桌面自动化、浏览器 CDP 单例、主执行体三类的并发访问收敛为「同 owner 重入、异 owner 立即拒绝」——根因是进程级单例与物理输入设备不可并发。
- **后处理**：filter 在工具结果进 Session 前做轻量规则化裁剪（截断/去重/去 ANSI）从源头省 token；hooks 允许外部 Shell 脚本挂在 `pre_tool_call`（可否决）等四个生命周期事件上，无需写 Rust 插件。

## 规则与坑

- 本模块是**纯横切层**：不拥有 Agent 决策，被 agent/（exec_tool 安全链、断路器）、tools/registry.rs（执行前二次校验）、runtime/（三个循环的输出后处理）调用；壳层通过 IPC 命令（approve_security 等）把用户决策写回。
- automation_gate 与「安不安全」正交——它是「现在允不允许碰这台机器」的另一维度，单槽语义，不排队不降级。
