# Tauri 壳 · IPC 命令层（src-tauri/src/commands/）

> 定位：前端 `invoke()` 的唯一 Rust 侧落点——把 IPC 参数适配、`AppState` 桥接与业务委托做成的「薄壳集合」，本身几乎不承载业务逻辑。
> 规模：67 文件；`#[tauri::command]` 属性 268 个（153 async + 115 裸 command 但全部紧跟 `pub async fn`），`main.rs` `generate_handler!` 注册总数 308。

## 结构

按业务域横切，一个文件 = 一个前端功能区；`mod.rs`（61 行）用 `pub mod` + `pub use self::x::*;` 把全部命令平铺到 `commands::` 命名空间：

| 分组 | 文件 | 职责 |
|---|---|---|
| 核心入口 | `process.rs`（1795）+ `process/`（leader 270 / shelf 3598 / session 1032 / lifecycle 351 / mode 491 / background 63 / retry 232 / refine 417） | send_message_cmd 核心链路 + 门铃唤醒重放；Runtime 构建/复用 + 断点恢复；会话展示台（内存 LRU + SQLite 快照 + 退出持久化）；中断/暂停/追加/终止；模式切换；后台任务面板；断点续跑重试；上下文提炼 |
| 遮罩截图 | `capture/`（mod 35 / geometry 346 / state 405 / session 272 等，三层） | 平台分派、几何/选区计算、交互状态机（纯计算）、流程编排（抓屏排除→冻结帧→遮罩→恢复） |
| 其余子模块 | config / memory / security / workflow / tools / dict_ocr / knowledge / speech / video / plugin_apps / external / mobile 等 27 个 | 各自前端功能区的命令薄壳 |

## 功能作用

- **跨语言边界的安全与稳定性**：前端传进来的每个字符串都可能是脏的——路径穿越、伪协议 URL、超长/超限文件都有专门防护。
- **主线程不卡死**：非 async 命令会跑在 Tauri 的 Blocking 执行上下文（主线程消息循环）里，触库/触盘即可让窗口「无响应」；`command_conventions.rs` 用一个契约测试把这条纪律钉死（当前 `tauri-sync-ok:` 豁免 0 处）。
- **双端（桌面 + 手机）共用同一业务入口**：`submit_user_message`、`chat_history`、`set_mode_impl` 等都做成泛型 `R: tauri::Runtime` 核心 + thin wrapper，让 mobile_server 的 HTTP 端点复用完全相同的行为。
- **执行态单一真相源**：跨命令共享的 busy / execution_stage / automation_gate / signals 全部挂在 AppState 上，命令层只做投影与守卫，不另设状态源。

## 规则与坑

- **薄壳纪律**：一个命令函数通常只做三件事——解引用 State<'_, AppState>、输入校验/归一化、委托给 `nuphus::*` 实现。典型 `send_message_cmd` 函数体只有一次 `submit_user_message(...)` 调用（`process.rs:118-149`）。
- 子目录只允许两层（process / config / capture 三层 / tools）；平台门控只出现在两处：`commands/mod.rs:9-10`（capture 仅 Windows）与各文件内的 `#[cfg(windows)]` 块。
- 「约 177 个命令」是过时数字，以实测 268/308 为准。
