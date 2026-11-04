# 配置管理（src/config/）

> 定位：Nuphus 的**模型注册表与凭据保险库**——以 `providers.toml` 为唯一权威配置源，把「有哪些 Provider 段 / 模型 / 能力绑定 / OAuth 令牌」解析成 `ModelRegistry`，并经 DPAPI 加密、文件锁与原子写保证凭据安全，供传输层（transports）、Agent、桌面 IPC 三方消费。
> 规模：31 文件（29 rs + 2 个 OIDC 测试夹具）。

## 结构

| 文件 | 职责 |
|---|---|
| `mod.rs`（742） | 模块出口、写锁、原子写、路径发现、能力策略解析（VisionStrategy / GenerationStrategy / SearchSourceBinding） |
| `model.rs`（2028） | **核心数据模型**：ModelRegistry / ProviderConfig / ModelEntry / ProviderOAuth / Capabilities / DecisionBackend |
| `provider.rs`（615） | `Provider` trait + 协议枚举 + 鉴权头归一 + 本地端点判定 + ProviderQuirks |
| `registry.rs`（287） | 内置 Provider 注册表（builtin）+ 模糊匹配（tier-2） |
| `credentials.rs`（57）/ `oauth.rs`（430）/ `oauth_flow.rs`（382）/ `oauth_accounts.rs`（151）/ `auth_state.rs`（204） | 令牌解析入口；段内 OAuth 磁盘读写 + 过期刷新；PKCE/回调校验/OIDC 验签纯逻辑；独立账号凭据文件；免解密登录态快照 |
| `preferences.rs`（536）/ `last_model.rs`（312） | 用户偏好持久化；`[last_model]` 影子表 + 三级归属解析 |
| `chatgpt/`（protocol 239 + store 599 + mod 90） | ChatGPT 订阅（SIWC）协议约束与账号生命周期 |
| `providers/`（15 个 rs） | anthropic / bytedance / custom / deepseek / google / kimi / local / minimax / openai / opencode_go / openrouter / qwen / stepfun / xai / zhipu 各自的模型表 / quirks / transport |

内部依赖方向（无环）：`api::ProviderKind`（权威枚举，config 只 re-export）← model.rs（数据）← provider.rs（trait）；credentials → oauth → oauth_flow ↘ oauth_accounts。

## 功能作用

- **配置发现与加载**：按优先级链找 `providers.toml`（桌面端经 `set_config_override` 锚定 AppData 路径），找不到回退环境变量（from_env）。
- **模型路由消歧三件套**：同一 model id 跨多个 provider 段（官方段 vs custom-xxx 中转实例）是常态——「成对绑定权威」（resolve_main_binding）、候选枚举（find_model_candidates）、能力段限定解析（resolve_capability），**禁止静默取段序首段**。
- **凭据安全**：api_key 与 OAuth 令牌落盘一律 `enc:v1:`（Windows DPAPI）密文，读取时三态透明解密；独立账号凭据存 JSON 文件，全程文件锁 + 原子替换 + 0600 权限。
- **OAuth 通用流程**：PKCE + 本地回调 + OIDC 验签 + scope 校验；ChatGPT 订阅（SIWC）协议。

## 规则与坑

- 两个并发安全基座：进程级 `PROVIDER_CONFIG_WRITE: Mutex<()>`（序列化所有 providers.toml 读改写事务）+ 文件级 `fs2` 排他锁。**持锁期间不得做网络 IO / await / 再拿另一把配置写锁**（`mod.rs:31-33`）。
- Tauri 壳本身不含任何配置业务逻辑——所有读写都收敛到本模块的公开函数。
