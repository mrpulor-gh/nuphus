# Tauri 壳 · 应用入口、窗口与全局状态（src-tauri/src/ 根文件）

> 定位：**零业务逻辑的 Tauri 外壳**——负责进程入口（含 `task` CLI 旁路）、窗口/托盘/热键的桌面呈现、全局唯一 AppState 的装配与注入、事件双推发射器、三个自愈/诊断守卫，以及构建期把三平台原生依赖（sherpa/onnxruntime/模型）自动补齐。
> 规模：16 文件（含 conf.json / Info.plist / installer-hooks.nsh）。

## 结构

| 文件 | 职责 |
|---|---|
| `main.rs`（1458） | 进程入口：CLI 分支、panic hook 落盘、CDP 环境注入、插件装配、setup() 全量启动编排、托盘、退出清算（spawn_exit_flush_and_exit） |
| `state.rs`（552） | 全局唯一 AppState 及三个分组结构（RuntimeContext / SessionState / ExecutionState）、ExecutionStageHandle、跨 IPC DTO 集合 |
| `emitter.rs`（265） | EventEmitter 的 Tauri/WS 实现：TauriEventEmitter / MobileWsEmitter / CompoundEmitter / FramedEvent；窗口可见性策略；HUD 仲裁桥接 |
| `command_conventions.rs`（127） | **纯测试模块**：禁止同步 `#[tauri::command]` 回流的契约闸门（every_tauri_command_runs_off_the_main_thread） |
| `shortcut.rs`（97）/ `splash.rs`（63）/ `startup_guard.rs`（122）/ `stall_probe.rs`（58）/ `resource_gate.rs`（105）/ `macos_permissions.rs`（293） | 便携模式桌面快捷方式；splash 进度 + 活性时钟；前端初始化卡死看门狗（45s）；主线程停摆留证探针（15s tick / 3s 判定）；core 资源互斥门在 Tauri 层的分类收口；macOS TCC 权限探测/请求 |
| `preview_protocol.rs`（518） | `preview://` 自定义协议：沙箱化本地文件运行底座 + 标注 overlay 注入 |
| `commands/cdp_window.rs`（307） | 唤起/新开标签到 Agent 的 CDP 浏览器并置前 |
| `build.rs`（972） | 构建脚本：中继凭据注入、sherpa/onnxruntime/OCR/YOLO 模型自动下载与同步、被锁 DLL 诊断 |
| `tauri.conf.json`（119）/ `installer-hooks.nsh`（16） | 三窗口/CSP/bundle/updater 配置；NSIS 安装后广播 SHCNE_ASSOCCHANGED 强制重建图标缓存 |

## 功能作用

- **进程入口与形态分发**：同一个 exe 承担两种形态——`nuphus task <verb> ...` 外部 Agent 上报 CLI（只发一次 HTTP POST 即退出，不建窗口）与默认 GUI 形态（`main.rs:198-215`）。
- **桌面呈现**：三个 WebView 窗口（splash / main / hud）、托盘图标与菜单、五条全局热键、Win11 圆角与窗口阴影。
- **全局状态唯一装配点**：`AppState::default()` 是全进程唯一的「共享信号 + 资源门 + 工具注册表 + 工作流引擎」的出生地，**构造顺序本身即语义**（`state.rs:271-350`）。
- **横切基础设施**：事件双推（桌面 IPC + 手机 WS）、preview:// 协议、命令执行上下文纪律（契约测试）、启动/停摆/资源三个守卫、构建期依赖自愈。

## 规则与坑

- 壳层与 core 的边界：「壳持句柄、core 不设全局 static」——SharedSignals、AutomationGate、cancel_flag 全部由 AppState 创建后显式注入 ToolRegistry / WorkflowEngine（`state.rs:298-310`）。
- `task` CLI 分支必须最先判定（`main.rs:198-215`）。
- 退出持久化必须异步化（序列化快照可达数 MB，主线程同步跑会「点了没反应」，看门狗 8s 兜底，`main.rs:36-45`）。
- Explorer 图标缓存污染：cargo build 覆盖 exe 时 Explorer 读到半写 PE 把白纸图标按路径+时间戳缓存，需 SHChangeNotify 自愈（`main.rs:144-158`）。
- `tauri-plugin-single-instance` 必须是第一个注册的插件，否则第二实例先建窗口/抢热键/抢 18771 门铃端口（`src-tauri/Cargo.toml:113-116`）。
