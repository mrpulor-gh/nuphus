---
title: 内置工具（PDF/图像/视频/文档）内部说明
id: tools-internal
type: skill
tags: [工具, PDF, 图像, 视频, 文档, 内部机制, invoke]
---

# 内置工具内部说明

> Nuphus 内置工具页（ToolsPage）的 **22 个内部机制命令** 能力全集。
> 工具设计定位（大王定调）：工具是**内部机制命令**（仅 `invoke_handler` 注册），
> **绝不注册进 `get_tools` / `execute_tool` 的 agent 工具列表**——用户经工具页手动调用，
> Agent/Workflow 不直接持有这些工具的调用入口。
>
> 本 skill 的作用：让 Agent 与 Workflow **感知**这些能力存在，
> 在对话/方案中知道「用户可经工具页处理 PDF/图像/视频」，并可引导使用。
> 工具页 UI 在桌面端导航「工具」；实现位于 `src-tauri/src/commands/tools/*`。

---

## 〇、感知与使用边界

| 角色 | 感知方式 | 能否直接调用 |
|------|----------|-------------|
| 用户 | 桌面端工具页（19 张能力卡片，两级导航：分类 tab → 卡片 → 详情页） | ✅ 手动调用 |
| Agent（Leader/Exec） | 本 skill（`skill_read tools-internal` / `skill_query`） | ❌ 不在 agent 工具列表，不可经 `execute_tool` 调用 |
| Workflow | 本 skill（`skill_query tools-internal` 检索） | ❌ 不可经 do.tool 调用（非 agent 工具） |

**Agent/Workflow 的正确姿态**：
- 用户需求涉及 PDF/图像/视频/文档处理 → 引导用户打开工具页操作，或说明能力清单让用户选择
- 若需程序化执行（自动化/批处理），路径是 **Rust 命令函数**（`src-tauri/src/commands/tools/*` 的 `#[tauri::command]` 函数，可被 Rust 测试/内部代码直接调用）或 **引擎二进制**（`nuphus-tools-rs/target/release/nuphus-{pdf,image,video}.exe`，仅覆盖部分命令）
- 实现要点：命令全部 `Result<serde_json::Value, String>` 返回，错误信息中文可读；PDF 走 lopdf、图像走 image crate、视频走 ffmpeg（`ensure_ffmpeg_suite` 定位）

---

## 一、命令总览（22 个）

| 类别 | 命令 | 函数 | 输入 → 输出 |
|------|------|------|------------|
| PDF | 合并 | `pdf_merge` | N 个 PDF → 1 个 PDF |
| PDF | 压缩 | `pdf_compress` | PDF → PDF（prune 未引用对象） |
| PDF | 页数 | `pdf_page_count` | PDF → 页数 |
| PDF | 提取文本 | `pdf_extract_text` | PDF → 文本（逐页，上限 200 页） |
| PDF | 图片转 PDF | `pdf_images_to_pdf` | N 张图片 → 1 个 PDF（每图一页） |
| PDF | 抽取页面 | `pdf_extract_pages` | PDF + 页码 → 新 PDF |
| PDF | 旋转 | `pdf_rotate` | PDF → PDF（90/180/270） |
| 图像 | 压缩 | `image_compress` | 图 → 图（限宽高 + 质量） |
| 图像 | 格式转换 | `image_convert` | 图 → 图（按扩展名推断） |
| 图像 | 缩放 | `image_resize` | 图 → 图（contain 保比例） |
| 图像 | 信息 | `image_info` | 图 → 宽高/大小/格式 |
| 图像 | 拼接 | `image_stitch` | N 图 → 1 长图（横/纵） |
| 图像 | 批量压缩 | `image_compress_batch` | N 图 → 目录（原名-out.扩展名） |
| 图像 | 批量转换 | `image_convert_batch` | N 图 → 目录（目标格式） |
| 视频 | 压缩 | `video_compress` | 视频 → 视频（libx264 重编码） |
| 视频 | 提取音频 | `video_extract_audio` | 视频 → 音频（wav/mp3） |
| 视频 | 抽帧 | `video_extract_frames` | 视频 → 目录（frame_%04d.jpg） |
| 视频 | 信息 | `video_info` | 视频 → 时长/编码/分辨率（ffprobe） |
| 视频 | 转 GIF | `video_to_gif` | 视频 → GIF（palettegen+paletteuse） |
| 视频 | 截取片段 | `video_cut` | 视频 → 片段（-ss 快速 seek + -c copy） |
| 视频 | 音频转换 | `audio_convert` | 音频/视频 → 音频（mp3/wav/m4a/flac） |
| 文档 | 文档转文本 | `doc_extract_text` | docx/pptx/xls/ods/odt/odp/pdf → 文本 |

---

## 二、PDF 命令（`src-tauri/src/commands/tools/pdf.rs`）

依赖：lopdf 0.34（与 nuphus core office.rs 共用）。单文件上限 500MB；extract_text 上限 200 页；图片转 PDF 单图像素上限 5000 万。

### pdf_merge
```rust
pdf_merge(input_paths: Vec<String>, output_path: String) -> Result<Value, String>
```
- 合并多个 PDF（保持各文档页面与书签大纲），输出 `pages` / `sources`
- 校验：每个输入必须存在且 ≤500MB；输出扩展名必须 `.pdf`

### pdf_compress
```rust
pdf_compress(input_path: String, output_path: String) -> Result<Value, String>
```
- prune_objects 清理未引用对象后重写，返回 `size_before` / `size_after` / `saved_bytes`

### pdf_page_count
```rust
pdf_page_count(path: String) -> Result<Value, String>
```
- 返回 `pages`

### pdf_extract_text
```rust
pdf_extract_text(path: String, max_pages: Option<u32>) -> Result<Value, String>
```
- lopdf 官方 extract_text（覆盖 TJ 数组/十六进制串/引号），逐页 `--- Page N ---` 分隔
- 返回 `text` / `pages` / `extracted_pages` / `truncated`

### pdf_images_to_pdf
```rust
pdf_images_to_pdf(input_paths: Vec<String>, output_path: String) -> Result<Value, String>
```
- 每张图一页，页面尺寸 = 图片像素（1px:1pt），JPEG 走 DCTDecode 其余 FlateDecode
- 图片扩展名白名单：png/jpg/jpeg/bmp/gif/webp

### pdf_extract_pages
```rust
pdf_extract_pages(input_path: String, pages: Vec<u32>, output_path: String) -> Result<Value, String>
```
- 1-based 页码抽取到新 PDF；去重保序；页码越界报错

### pdf_rotate
```rust
pdf_rotate(input_path: String, output_path: String, degrees: u32, pages: Option<Vec<u32>>) -> Result<Value, String>
```
- degrees 仅 90/180/270；pages 缺省 = 全部页，累积到现有 /Rotate

---

## 三、图像命令（`src-tauri/src/commands/tools/image.rs`）

依赖：image crate（png/jpg/jpeg/bmp/gif/webp）。支持扩展名由 `output_format` 白名单校验。

### image_compress
```rust
image_compress(input_path, output_path, max_width: Option<u32>, max_height: Option<u32>, quality: Option<u8>) -> Result<Value, String>
```
- 保纵横比（contain）；JPEG quality 生效（默认 82），PNG 走 Best+Adaptive 真压缩
- 返回 `width` / `height` / `size_before` / `size_after`

### image_convert
```rust
image_convert(input_path, output_path) -> Result<Value, String>
```
- 格式随输出扩展名推断（png/jpg/jpeg/bmp/gif/webp）

### image_resize
```rust
image_resize(input_path, output_path, width: u32, height: u32) -> Result<Value, String>
```
- contain 保比例，不放大

### image_info
```rust
image_info(path) -> Result<Value, String>
```
- 返回 `width` / `height` / `size_bytes` / `format`

### image_stitch
```rust
image_stitch(input_paths: Vec<String>, output_path: String, direction: Option<String>) -> Result<Value, String>
```
- horizontal（统一高度，宽求和）/ vertical（统一宽度，高求和）；默认 horizontal

### image_compress_batch
```rust
image_compress_batch(input_paths, output_dir, max_width: Option<u32>, max_height: Option<u32>, quality: Option<u8>) -> Result<Value, String>
```
- 输出目录每张 `原名-out.扩展名`；返回 `count`

### image_convert_batch
```rust
image_convert_batch(input_paths, output_dir, format: String) -> Result<Value, String>
```
- 目标格式 png/jpg/jpeg/bmp/gif/webp；输出目录 `原名.扩展名`（jpeg→jpg）；返回 `count`

---

## 四、视频命令（`src-tauri/src/commands/tools/video.rs`）

依赖：ffmpeg（`crate::video::deps::ensure_ffmpeg_suite` 定位，探测链 + PATH）。所有命令 async + spawn_blocking，不在 UI 线程执行。

### video_compress
```rust
async fn video_compress(input_path, output_path, quality: Option<String>) -> Result<Value, String>
```
- quality: low(1M/64k) / medium(2M/96k 默认) / high(4M/128k)；libx264 + aac 重编码
- 返回 `size_before` / `size_after`

### video_extract_audio
```rust
async fn video_extract_audio(input_path, output_path, format: Option<String>) -> Result<Value, String>
```
- wav（pcm_s16le 44100 stereo）/ 默认 mp3（libmp3lame 192k）

### video_extract_frames
```rust
async fn video_extract_frames(input_path, output_dir, interval: Option<f64>) -> Result<Value, String>
```
- 每 interval 秒抽一帧（fps=1/interval，interval 0.001~3600 秒），输出 `frame_%04d.jpg`（q:v 2）
- 返回 `frames`（文件数）

### video_info
```rust
async fn video_info(path) -> Result<Value, String>
```
- ffprobe JSON 解析：`duration_secs` / 视频编码 / 分辨率 / 音频编码

### video_to_gif
```rust
async fn video_to_gif(input_path, output_path, fps: Option<f64>, scale: Option<u32>) -> Result<Value, String>
```
- 输出必须 .gif；fps 默认 10（1-30）；scale 输出宽度默认 480（16-2000），高度等比
- palettegen + paletteuse 两段式（色彩保留）；临时 palette.png 完成后删除

### video_cut
```rust
async fn video_cut(input_path, output_path, start_sec: Option<f64>, end_sec: Option<f64>) -> Result<Value, String>
```
- -ss 置于 -i 前（快速 seek）+ -c copy（流复制不重编码）；start 0~86400，end > start

### audio_convert
```rust
async fn audio_convert(input_path, output_path, bitrate: Option<String>) -> Result<Value, String>
```
- 输出扩展名决定编码：mp3=libmp3lame / wav=pcm_s16le / m4a=aac / flac=flac
- bitrate 可选（如 "192k"，mp3/m4a 生效）；输入可为音频或视频（提取音轨）

---

## 五、文档命令（`src-tauri/src/commands/tools/doc.rs`）

### doc_extract_text
```rust
doc_extract_text(path: String) -> Result<Value, String>
```
- 复用 nuphus core `utils::office::read_office`：支持 docx / pptx / xls / ods / odt / odp / pdf
- 返回 `chars` / `text`；不支持格式报错「不支持的文档格式」

---

## 六、注册与调用链

- 注册：`src-tauri/src/main.rs` `invoke_handler`（L335-357 段，22 个命令全部 `commands::tools::*`）
- 模块：`src-tauri/src/commands/tools.rs`（声明 `pub mod pdf/image/video/doc`，并承载 agent 工具 `get_tools`/`execute_tool`——两者职责隔离，内部机制命令绝不进 agent 工具列表）
- 前端：`frontend/src/main-window/tools/ToolsPage.tsx`（ABILITIES 配置表驱动）+ `lib/api.ts` wrappers
- 预览：`frontend/src/main-window/chat/PreviewOverlay.tsx`（`FilePreviewContent` 无壳内嵌预览）
- 引擎二进制（覆盖子集，可命令行调用）：`nuphus-tools-rs/target/release/nuphus-{pdf,image,video}.exe`
  - `nuphus-pdf.exe merge/info/extract-text/compress`
  - `nuphus-image.exe compress/convert/resize/info`
  - `nuphus-video.exe compress/extract-audio/info/extract-frames`

---

## 七、经验与陷阱

- **工具不进 agent 工具列表是设计铁律**：任何「把内置工具注册进 get_tools/execute_tool」的改动都是错误方向
- 视频命令依赖 ffmpeg：`ensure_ffmpeg_suite` 探测链失败时返回中文错误提示；运行测试需 ffmpeg 在 PATH
- 路径处理统一 `to_string_lossy`（含中文/空格路径不 panic）；错误信息 `Result<Value, String>` 中文可读
- PDF 大文件/图片高像素有资源上限（防 OOM），超限提示改用系统程序
- 工具页 UI 迭代历史：三级分类 → 应用内全屏 → 左右分栏（左上传+预览 / 右设置+状态+保存）
