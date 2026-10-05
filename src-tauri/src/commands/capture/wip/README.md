# capture/wip — 未完成的平台原生遮罩实现（**未接入编译**）

本目录存放从 `shot` 项目移植过来、但**从未编译通过**的 macOS / Linux 原生遮罩实现。
2026-10-05 v0.2.26 发布时，两者在 CI 上全部编译失败（Release 工作流 macos-14 与
ubuntu-24.04 两个 job 红、windows-latest 绿），已整体移出编译树——
`cargo` 不会编译未被 `mod` 声明的文件，放这里是「留着源码、不进构建」，
非 Windows 平台截图继续走既有的 WebView overlay 兜底链路（0.2.25 及以前的行为）。

## 移出时记录的真实错误（CI 日志，2026-10-05）

`macos.rs`（8 处 error）：
- `E0432`：`use crate::commands::capture::paint` 路径错，应为 `capture::surface::paint`
- `E0063`：`MacState` 初始化缺 `scale` 字段
- `E0277` ×5：`Retained<AnyObject>` 不满足 `Encode` / `MessageReceiver` /
  `ConvertReturn<MethodFamily<2|3>>`——rustc 提示 **依赖图里同时存在两个 objc2 版本**：
  objc2 0.5.2（`enigo 0.2.1 → icrate 0.1.2 → block2 0.4.0`）与 objc2 0.6.4
  （`objc2-foundation 0.3.2` 一线）。代码按 0.6 API 写（`Retained` / `msg_send`），
  与树里另一版本的类型/ trait 实现对不上。

`linux.rs`（4 处 error）：
- `E0432`：同一个 `paint` 导入路径错
- `E0599` ×2：`x11rb::cookie::Cookie` 没有 `check` 方法（x11rb 0.13 API 与代码假设不服）
- `E0308`：某方法参数不匹配

## 恢复编译需要做的事（按顺序）

1. **macOS**：先定 objc2 版本口径——要么全部统一到 0.6.4（`Cargo.toml` 的
   objc2 / objc2-foundation / objc2-core-graphics / block2 恢复声明，代码按 0.6 API 写），
   要么接受 enigo 一线带来的 0.5.2 并把实现改回 0.5 API；二者混用必然继续红。
   修 `MacState.scale` 缺字段、`paint` 导入路径、`CGColorSpaceCreateDeviceRGB` 等
   已废弃函数（CI 里有 deprecated 警告）。
2. **Linux**：修 `paint` 导入路径，把 cookie 等待/校验改到 x11rb 0.13 的真实 API
   （`Cookie::check` 不存在，改用 `reply()` / `check()` 的等价物，以 0.13 文档为准）。
3. 恢复 `surface/mod.rs` 的 `pub mod` 与 `pub use … run_overlay`、
   `capture/mod.rs` 架构注释里的对应行、`Cargo.toml` 对应 target 依赖段
   （段落里留了注释标记，取消注释即可）。
4. **去掉 `commands/mod.rs` 里 `pub mod capture;` 上的 `#[cfg(windows)]` 门控**——
   v0.2.26 起因：模块不门控时，非 Windows 平台上消费者（windows.rs）整体消失，
   geometry/state/session/paint 全部变成死代码，`clippy --all-targets -D warnings`
   报 81 处 dead_code 直接红 CI。门控是当时的止血，接回平台时必须一并拆除。
5. **真机验证通过前不得切主链路**：`session::run_screenshot` 的
   `#[cfg(not(windows))]` 分支继续返回 `native-capture-unsupported`，
   非 Windows 一律回落 WebView overlay。

## 当前生效的范围

- Windows：原生遮罩主链路（分层窗口 + 冻结帧 + WDA_EXCLUDEFROMCAPTURE），已真机验证。
  `commands/mod.rs` 用 `#[cfg(windows)]` 门控整个 capture 模块。
- macOS / Linux / 其它：WebView overlay 兜底（`surface/fallback.rs` 之外的既有链路，
  由 `commands/toolbar.rs` 的旧分支处理）；capture 模块在这些平台不参与编译。
