# 跨领域模块与库入口（core-crosscut）

> 定位：一批「不属于任何单一业务域、却被全仓反复依赖」的基础设施——要么把桌面壳的能力反向注入 Tauri-agnostic 的 lib（render/video/ext_agent bridge、hud_arbiter），要么给 lib 提供进程内唯一的共享状态与横切工具（state、utils、cache、cookies、annotation、custom_agents），要么打通跨进程/跨语言的边界（handoff 门铃 + CLI、cookies CDP 回退、automation_lock 跨进程锁），外加编译期内嵌资产与两个 opt-in 真实环境冒烟 example。
> 规模：34 文件。

## 结构

| 主题 | 文件 | 职责 |
|---|---|---|
| 注入桥四兄弟 | `render_bridge.rs`（84）/ `video_bridge.rs`（38）/ `ext_agent_bridge.rs`（35）/ `hud_arbiter.rs`（612） | 壳注册普通 fn、core 同进程直调，无 IPC 无消息循环；hud_arbiter 是「壳调 core 的仲裁器」（HUD 并行工具计时，纯派生状态机） |
| 共享状态 | `state.rs`（482）/ `mobile_append.rs`（154） | SharedSignals / SignalState / ExecutionStage 三态（执行态唯一真相源）/ SubmitDisposition / SecurityState；追加指令入队/去重/格式化 |
| 横切工具库 | `utils/mod.rs`（2743）/ `office.rs`（1237）/ `xlsx.rs` / `xlsx_write.rs` / `proxy.rs` / `net_diag.rs`（665）/ `automation_lock.rs`（592） | 运行时可写路径权威解析（plugin_root / nuphus_data_dir / resolve_user_path）；文本净化与截断；办公文档三格式解析 + PDF 三层降级；系统代理探测；网络诊断；跨进程桌面/浏览器自动化互斥锁 |
| 数据与凭据 | `cookies/vault.rs`（1030）/ `cdp_source.rs`（209）/ `cache/tool_cache.rs`（663） | Chrome Cookies DB 读取 + DPAPI 解密 + 内存缓存 + 敏感配置加密；DB 不可用时的 CDP live 回退；持久化只读工具结果缓存（LRU + mtime/TTL 校验） |
| 提示词与 Agent | `annotation/`（types 194 + store 317 + presets 41）/ `custom_agents.rs`（256）/ `api/mod.rs`（156） | 关键词→关系映射的提示词注入；用户自定义 Agent 卡片 CRUD + 会话归属锚点；ProviderKind 权威枚举 + ApiClient trait |
| 跨进程边界 | `handoff.rs`（563）/ `handoff/cli.rs`（248） | 外部 Agent 门铃事件队列（lib 侧）；桌面壳内置的 `task` 子命令 CLI |
| 入口与资产 | `main.rs`（97）/ `build.rs`（143）/ `examples/`（2 个 opt-in 冒烟）/ `test_helpers/` | CLI 单次执行入口；编译期内嵌随包只读 plugin 资产；真实桌面/工作流冒烟 |

## 功能作用

- **bridge 模式**：lib 里的工具执行器是裸 fn 指针、无法捕获状态，而 PDF 渲染/视频字幕/外部 Agent 编排/HUD 推送天然长在桌面壳里——壳在启动时注册一个普通 fn，core 直接同进程调用。四者模式完全同构，**任何一个的注册遗漏都表现为「诚实的不可用错误」而非伪造结果**。
- **ExecutionStage 三态**把曾经分散在前端 isProcessing、后端 busy、guard_switch.can_switch、rail OR 派生等 5 处的执行态收敛为唯一真相源。
- **utils 四类关键职责**：① 运行时可写路径权威解析（直接决定发布版能否启动）；② 文本净化与截断（think 标签剥离、工具输出限长，直接影响模型看到什么）；③ 办公文档三格式解析 + PDF 三层降级；④ 网络诊断与跨进程自动化锁。
- **handoff 门铃 + CLI**：外部 Agent 经 HTTP 上报事件入队，轮次边界被动注入 Leader；`task` CLI 让外部进程一条命令完成上报。

## 规则与坑

- `src/`（lib crate nuphus）刻意保持 **Tauri-agnostic**：不依赖 tauri、不依赖 IPC，只提供纯 Rust 能力。
- `pub static APP_STATE` 全局单例已改造成 `Arc<RwLock<SignalState>>` 显式句柄（PR-2），由 src-tauri 的 AppState.signals 持有并逐层注入。
