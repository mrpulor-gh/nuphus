# Nuphus 核心库（`nuphus` crate）

> **路径**: `src/` | **入口**: `lib.rs` | **Crate**: `nuphus`  
> **模块数**: 32 | **版本**: 0.1.0

Nuphus 是统一 Runtime + ReactAgent 架构的桌面 Agent 引擎，包含运行时、Agent、记忆、工具、安全、多 Provider 传输等完整能力。

## 模块总览（32 子模块）

### 核心引擎层

| 模块 | 文件 | 说明 |
|------|------|------|
| `runtime` | 11 文件 | **统一运行时** — 所有用户请求的统一入口。包含 ReAct 主循环（`react_loop.rs`）、三模式路由（`mode.rs`：Leader/Workflow/Custom）、子任务分发（`dispatch.rs`）、SubTaskRunner（`sub_task.rs`）、ProtectionGuard（`protection.rs`）、WorkflowAgent（`workflow_agent.rs`） |
| `agent` | 11 文件 | **Agent 引擎** — ReactAgent 纯状态容器（循环已移入 Runtime）。包含 L0/L1/L2 三层提示词构建（`prompt.rs`）、统一工具执行 + 五层安全链（`exec_tool.rs`）、类型化事件流（`events.rs`）、会话经验提炼（`distill.rs`）、持久提醒队列（`reminders.rs`） |

### LLM 层

| 模块 | 文件 | 说明 |
|------|------|------|
| `llm` | 3 文件 | **LLM 客户端工厂** — LlmClient 通用 LLM 客户端（通过 Transport 适配所有 Provider）、ClientFactory 动态创建 |
| `transports` | 8+ 文件 | **可插拔传输层** — Transport trait 抽象（7 个方法），ChatCompletionsTransport（OpenAI 兼容 SSE）、Anthropic 消息格式 parser、MockTransport 测试桩、StreamEvent 统一事件流 |

### 记忆存储层

| 模块 | 文件 | 说明 |
|------|------|------|
| `session` | 6 文件 | **会话管理** — Session / Message / ContentBlock 结构化消息，上下文窗口管理，消息变换 |
| `memory` | 3 文件 | **记忆体系** — MemoryEntry 统一结构，双通路检索（关键词 FTS5 + 语义 Embedding），TenetStore 用户教导不可变原则 |
| `store` | 4 文件 | **SQLite 持久化** — 连接池 + WAL 模式，FTS5 全文搜索 + BM25 排序，Schema 版本化迁移，Session 持久化 |
| `embed` | 1 文件 | **向量嵌入** — bge-small-zh 模型，512 维语义向量生成，用于记忆语义检索通路 |
| `segmenter` | 1 文件 | **中文分词** — jieba 分词，用于 FTS5 索引预处理和关键词检索 |

### 工具技能层

| 模块 | 文件 | 说明 |
|------|------|------|
| `tools` | 多文件 | **工具系统** — ToolRegistry 注册 + 调度 + 提示词缓存，覆盖文件/系统/桌面/浏览器/记忆/知识/标注/技能/工作流/规划/办公文档 11 个域 |
| `skill` | 多文件 | **检索式技能** — SkillRegistry 管理可插拔知识包，按需检索不占 prompt |
| `workflow` | 26 文件 | **工作流引擎** — WorkflowEngine 编排器 + Compiler 编译验证 + Executor 确定性执行（10 种步骤类型）+ SchedulerEngine Cron 定时 + ChatAgent 智能决策节点 |

### 自动化层

| 模块 | 文件 | 说明 |
|------|------|------|
| `desktop` | 13 文件 | **桌面控制** — Windows 桌面自动化（截图/鼠标/键盘/窗口/OCR/YOLO/Vision），Win32 API 封装 + ONNX 推理 |
| `browser` | 1 文件（mod.rs）+ 独立 crate | **浏览器自动化** — 实现位于独立 crate `crates/nuphus-browser`（Chrome DevTools Protocol 封装：导航/快照/点击/输入/截图/JS 执行/Cookie/批处理），src/browser 仅 re-export |

### 安全层

| 模块 | 文件 | 说明 |
|------|------|------|
| `permissions` | 1 文件 | **权限策略** — ToolCategory 分类（Core/FileAccess/WebSearch/SystemAutomation），PermissionPolicy 授权 |
| `security` | 4 文件 | **安全体系** — SecurityGuard 危险操作检测、InjectionDetector 外部注入防护、用户审批流程、断路器熔断 |
| `filter` | 3 文件 | **输出过滤** — 工具返回值过滤，减少 token 消耗 |
| `hooks` | 多文件 | **Shell Hooks** — 工具执行前后的自定义脚本钩子 |

### 跨领域层

| 模块 | 文件 | 说明 |
|------|------|------|
| `mcp` | 4 文件 | **MCP 集成** — Model Context Protocol 客户端（client/config/dual），连接外部工具服务 |
| `handoff` | 1 文件 | **外部 Agent 门铃** — HTTP 端点接收外部 Agent 完工上报 |
| `workflow` | 26 文件 | （已列于工具技能层） |
| `cache` | 2 文件 | **工具缓存** — 文件读取/Web 搜索结果缓存 |
| `config` | 18 文件 | **配置管理** — 多 Provider 模型配置注册表 |
| `cookies` | 3 文件 | **Cookie 管理** — Chrome Cookie 导入（DPAPI 解密）+ Cookie Vault |
| `state` | 1 文件 | **全局状态** — AppState 统一状态管理（替代原有多个 static Mutex） |
| `utils` | 6 文件 | **工具集** — 办公文档读取（`office.rs`，7 格式 + PDF 三层降级）、Excel 写出（`xlsx_write.rs`）、xlsx 读取、代理配置 |
| `custom_agents` | 1 文件 | **自定义 Agent** — 用户自定义 Agent 配置管理 |
| `mobile_append` | 1 文件 | **移动端追加** — 手机端追加指令处理 |
| `render_bridge` | 1 文件 | **渲染桥接** — PDF/文档渲染服务注入桥 |
| `video_bridge` | 1 文件 | **视频桥接** — 视频字幕流水线注入桥 |
| `annotation` | 多文件 | **标注管理** — 关系标注 CRUD |
| `api` | 多文件 | **Provider 抽象** — ApiClient trait + 统一类型 |

---

## 架构关系

```
runtime (统一入口)
  └─ react_loop() ── ReAct 主循环
        ├─ llm → transports ── Provider 适配
        ├─ agent::exec_tool ── 五层安全链
        │     ├─ permissions ── 权限检查
        │     ├─ security ── 规则审查
        │     └─ tools ── 工具执行
        ├─ session ── 上下文管理
        ├─ memory → store ── 双通路检索
        │     ├─ segmenter ── 分词
        │     └─ embed ── 语义向量
        ├─ skill ── 按需知识检索
        ├─ workflow ── 自动化编排
        ├─ browser ── 浏览器控制
        ├─ desktop ── 桌面操控
        ├─ mcp ── 外部工具集成
        ├─ handoff ── 外部 Agent 门铃
        └─ protection ── 断路器保护
```