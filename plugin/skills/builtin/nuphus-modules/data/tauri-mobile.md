# Tauri 壳 · 移动端服务/中继/外部 Agent（mobile_server / relay_client / ext_agent / handoff_server）

> 定位：Tauri 壳侧的四个「对外通道」——手机第二屏 HTTP+WS 服务（mobile_server）、桌面出站中继客户端双回路（relay_client）、外部 Agent 派发编排与工具序列执行（ext_agent）、外部 Agent 门铃 HTTP 服务（handoff_server）；全部只做 IPC/IO 桥接，无业务逻辑，业务统一走 nuphus crate 与 commands 层共享入口。
> 规模：5 文件。

## 结构

| 文件 | 规模 | 职责 |
|---|---|---|
| `mobile_server.rs` | 4463 | 移动端局域网 HTTP+WS server（axum，绑 0.0.0.0，默认关闭）：~30 个 REST 端点 + `/ws` 事件推流 + SPA/插件静态伺服 + CORS 白名单 + 配对密码/token 鉴权（PairThrottle 防爆破） |
| `relay_client.rs` | 2036 | 桌面出站中继客户端：`/ws/device` 任务回路 + `/ws/tunnel` 隧道回路，指数退避重连状态机、代理感知拨号、TCP keepalive、配置持久化与凭据轮换 |
| `ext_agent/mod.rs` | 574 | `agent_dispatch` 工具编排：校验上板 → 在途闸 → 进程/窗口捕获（PID 父链回溯）→ SeqRunner 投递 → 失败收口；同步 fn 桥接 Leader tokio 上下文 |
| `ext_agent/seq_runner.rs` | 707 | agent_dispatch 内部工具序列轻量执行器：占位符替换 + 桌面工具白名单分发 + 分阶段校验（**预校验先于任何桌面副作用**） |
| `handoff_server.rs` | 499 | 外部 Agent 门铃 HTTP server（axum，仅 127.0.0.1）：`/handoff` 上报、`/handoff/dispatch` 派发前置、`/handoff/health`；薄适配层，状态全在 nuphus lib 的 HandoffStore |

数据流：

```
手机 PWA ──HTTP/WS──> mobile_server (0.0.0.0:18772) ──submit_user_message──> Agent 主循环
外部 Agent 进程 ──curl/CLI──> handoff_server (127.0.0.1:18771) ──push_event──> HandoffStore ──drain──> Leader 上下文
中继服务器 ──出站 WS──> relay_client /ws/device ──handle_task──> submit_user_message (source="relay")
中继服务器 ──出站 WS──> relay_client /ws/tunnel ──TCP-over-WS──> 127.0.0.1:18772 (mobile_server)
Leader ──agent_dispatch──> ext_agent bridge ──DesktopClient──> 外部 Agent 窗口（敲键投递）
```

## 功能作用

- **手机 = 同一会话的第二块屏**：局域网 HTTP+WS 把桌面 Agent 的事件/会话/控制面完整投射到手机 PWA。
- **外网通路**：手机不在同一 WiFi 时，桌面**出站**连接中继服务器（`/ws/device` 收外部任务、`/ws/tunnel` 把公网隧道字节流转发到本机 mobile_server）。
- **外部 Agent 派发**：Leader 把任务派发给外部 Agent（终端 TUI）——上板 brief、捕获进程窗口、确定性敲键投递、失败收口。
- **门铃回呼**：外部 Agent 完工/受阻时回「门铃」，事件入 HandoffStore，轮次边界被动注入 Leader。

## 规则与坑

- 四个文件共享同一套模式（`TcpListener::bind + axum::serve + token 鉴权 + tokio 生命周期 + 优雅降级`），差异只在绑定地址（0.0.0.0 vs 127.0.0.1）与协议（HTTP/WS vs 纯 WS vs TCP-over-WS）。
- 启动注册全部在 `src-tauri/src/main.rs` setup 阶段：handoff_server（:1119）、relay_client（:1170-1171）、mobile_server 仅当持久化配置 enabled=true 时自动恢复（:1175-1187）、ext_agent init_bridge（:772）。
- 端口事实：门铃 18771、mobile_server 18772。
