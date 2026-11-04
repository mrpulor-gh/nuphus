# Tauri 壳 · desktop-api 底层桌面 API crate（src-tauri/crates/desktop-api/）

> 定位：**把「截图 / OCR / YOLO / 键鼠输入 / 剪贴板 / 窗口枚举」这些与操作系统直接对话的底层桌面能力，收敛成一个可被 nuphus core 以 path 依赖引入、也可独立编译成 HTTP 服务二进制的 Rust crate**——它是 Nuphus 桌面自动化的执行层（effector），core 的 `src/desktop/` 是策略层（policy）。
> 规模：28 文件约 6.3k 行；crate 版本 2.0.0。

## 结构

| 分组 | 文件 | 职责 |
|---|---|---|
| crate 根 | `lib.rs`（68）/ `main.rs`（37） | 模块装配 + glob re-export + `DesktopApi` 入口（feature 门控）；独立 HTTP 服务二进制入口（带 http-server 特性时 127.0.0.1:8888 起 axum 服务） |
| 核心类型 | `core/mod.rs`（397） | Session / Target / Frame / Scope / AppKind + 错误枚举 + SessionManager |
| 平台抽象 | `platform/mod.rs`（454）+ `windows.rs`（113） | WindowManager（LRU 缓存查找 / 枚举 / 移动 / 缩放 / 详情）；**整体 `#[cfg(windows)]`** |
| 视觉 | `vision/`（mod 236 / capture 421 / runtime 145 / models 163 / paddle_ocr 487 / yolo 312 / perceive 404 / ocr 49 / locate 374） | 截图实现（Scope 分派、后端分派、跨显示器区域拼接）；ONNX Runtime 懒初始化；模型目录解析；PP-OCRv4 全管线（det→rec→CTC）；YOLO 图标检测（预处理/推理/NMS）；OCR+YOLO IoU 合并；找图（金字塔 MAD）/ 找色（DeltaColor 偏色） |
| 输入 | `input/`（mod 302 + sendinput 387 + keyboard 246 + mouse 628） | 输入引擎 + `HeldInput` RAII + enigo 全局串行化；输入法理念的 Unicode 逐码点注入（Windows）；按键/组合键 + 键名白名单；鼠标移动/点击/滚动/拖拽（含 ease-in-out 轨迹与微抖）+ macOS CoreGraphics FFI |
| 剪贴板 | `clipboard.rs`（139） | Win32 原生剪贴板读写（含 CF_HDROP 文件列表）+ macOS/Linux arboard |
| HTTP API | `api/`（mod 146 + http 310） | 统一 API 层 see/find/do/say + axum 路由 6+1 端点（**feature 门控**，大量 TODO 占位） |

## 功能作用

对应 crate 描述四词「感知 · 查找 · 操作 · 沟通」：

1. **感知**：xcap 跨平台截图（全屏/窗口/客户区/区域/点邻域五种 Scope），Windows 上按窗口图形后端（GDI/DirectX/OpenGL/Vulkan）分派策略，处理 Retina 物理像素→逻辑桌面归一化。
2. **查找**：模板找图（金字塔粗扫 + 原图精扫 MAD 匹配）、偏色找色、PaddleOCR、YOLO UI 元素检测、OCR+YOLO IoU 合并去重。
3. **操作**：SendInput/keybd_event/mouse_event 原生输入、Unicode 逐码点注入、跨线程焦点 AttachThreadInput、macOS CoreGraphics / Linux enigo 兜底；Win32 窗口枚举/查找/移动/缩放/详情。
4. **沟通**：Win32 原生剪贴板（含 CF_HDROP）；可选 http-server 特性把上述能力包成 axum HTTP 服务（默认关闭）。

## 规则与坑

- 架构位置：**src/desktop/（策略：语义定位、capture 缓存、坐标空间约定、工具 JSON 编排）→ 调用 → desktop-api（执行：Win32/xcap/ort 直接调用）→ 操作系统**。src-tauri 壳不直接依赖本 crate，全部经 nuphus core 传递。
- 三条编译期边界：platform 模块整体 cfg(windows)；api 模块与 DesktopApi 入口整体 cfg(feature="http-server")（默认关闭，core 常规构建完全不编译）；ort 版本钉死 `=2.0.0-rc.12`。
- 测试全部 `#[cfg(test)]` 内嵌在各源文件中，另有 3 个真实零帧/端到端冒烟 example。
