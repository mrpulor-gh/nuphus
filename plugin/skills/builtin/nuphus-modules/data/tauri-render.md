# Tauri 壳 · PDF 渲染桥（src-tauri/src/render/）

> 定位：**把主 WebView 里的 pdf.js 变成 Rust 可调用的「PDF → PNG / 文本」服务**——Tauri v2 的 `window.eval` 没有返回值，于是用「eval 触发前端全局函数 + Tauri 命令回传 + mpsc channel 结算」拼出一次前后端往返；唯一消费者是扫描件 PDF 的 OCR 兜底链路。
> 规模：2 文件共 172 行。

## 结构

| 文件 | 职责 |
|---|---|
| `render/mod.rs`（10） | 模块声明 + 架构总览文档注释（定位、首个消费者、注入模式） |
| `render/commands.rs`（162） | **命令面 + 桥接实现**：两个 Tauri 回传命令（pdf_render_done / pdf_render_error）、`init_bridge` 注册入口、前后端往返核心 `run_frontend_job`、两个 bridge 入口（bridge_render_pdf / bridge_extract_pdf_text）、挂起请求表 PENDING、RENDER_TIMEOUT |

契约对端与上下游：`src/render_bridge.rs`（84 行，nuphus crate 侧注入点：两个 fn 指针类型别名 + OnceLock 静态槽 + is_available() 探测）；`frontend/src/core/pdf-render.ts`（158 行，前端 pdf.js 服务：离屏 canvas 逐页渲染 PNG base64 / getTextContent 逐页取文本，invoke 回传）；`frontend/src/main.tsx:8-9`（副作用 import 保证全局函数在任何 eval 之前已注册）；`src-tauri/src/main.rs`（invoke_handler 注册两个命令 + setup() 调 init_bridge）；`src/utils/office.rs`（read_pdf / read_pdf_via_bridge / read_pdf_scanned_ocr，**唯一业务消费者**）。

## 功能作用

- 把 nuphus crate（Tauri 无关的核心引擎）需要的「渲染 PDF / 提取 PDF 文本」能力，以 **fn 指针注入** 方式注册进 `nuphus::render_bridge`（单进程直调，无 IPC）。
- 被调用时：读文件 → base64 → `window.eval` 触发前端全局函数 `window.__nuphusRenderPdf` / `window.__nuphusRenderPdfText`；前端渲染/提取完成后 invoke 回传，Rust 侧 settle 一个 mpsc channel，把结果（PNG 字节 / 逐页文本）交还调用方。

## 规则与坑

- 架构决策：**不引入原生 PDF 渲染库、不新开窗口、不落临时文件**，直接复用主 WebView 的 pdf.js——因为 lopdf 抽不出 CID-keyed 字体的文本，纯扫描件（PDF 里只有图片）根本没有文本层。
- 前端全局函数必须在启动时以副作用 import 方式注册，保证任何 eval 之前已就绪。
