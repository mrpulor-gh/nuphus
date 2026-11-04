# LLM 客户端与可插拔传输层（src/llm/ src/transports/ src/api/）

> 定位：多协议 LLM 接入层。以 `Transport` trait 为唯一分叉点，把 ChatCompletions SSE / Anthropic Messages / OpenAI Responses 三种上游协议归一为统一的 `StreamEvent` 事件流，再由 `ClientFactory` 按 `providers.toml` 动态装配出 `Arc<dyn ApiClient>` 交给 Agent/Runtime 消费。上层只面向一套稳定抽象编程，不关心背后是哪家 provider、哪种协议、什么鉴权方式。
> 规模：21 文件（全部 .rs）。

## 结构

三层分工：

| 层 | 目录 | 职责 |
|---|---|---|
| 抽象契约 | `src/api/` | `ApiClient` trait、`ProviderKind` 枚举（15 变体）、MessageRequest/AssistantEvent 等跨协议报文类型 |
| 客户端装配 | `src/llm/` | `LlmClient`（把 Transport 包成 ApiClient，207 行）、`ClientFactory`（按注册表动态建 client，548 行） |
| 协议适配 | `src/transports/` | Transport trait + 三种协议实现 + SSE 解析 + 重试/超时/断流 salvage |

关键文件：`transports/chat_completions/transport.rs`（2897 行，**最重**：请求构造 + 流式读循环 + SSE 解析 + 重试 + salvage）、`transports/anthropic/parser.rs`（462，SSE 事件状态机）、`transports/responses/parser.rs`（674）+ `transport.rs`（685）、`mock_transport.rs`（114，测试桩）、`opencode_session.rs`（65，OpenCode Go 实例级稳定会话 ID）。

## 功能作用

- **统一事件流**：`StreamEvent → AssistantEvent` 转换（`transports/mod.rs:34-66`），上层只见一套事件。
- **三协议适配**：报文形状、鉴权头、SSE 事件名差异全部封在 transports/ 内。
- **动态装配**：ClientFactory 按 ModelRegistry 的 (provider, model) 成对绑定创建 client；加新 provider 只需新 Transport 实现 + 工厂构造器，**Agent 层零改动**。
- **韧性**：SSE 解析、重试、超时、断流 salvage（已收部分内容不丢）。

## 规则与坑

- LLM 交互统一走 `ApiClient` trait，旧 `LLM` trait 已废止（`src/llm/mod.rs:4`）。
- `api/` 只保留 ProviderKind 枚举与 ApiClient trait 定义，模型配置已移入 `config::ModelRegistry`（`src/api/mod.rs:5-6`）。
- 模块内部依赖无环：api/（契约）← llm/client ← llm/factory → transports/* ← transport_base。
