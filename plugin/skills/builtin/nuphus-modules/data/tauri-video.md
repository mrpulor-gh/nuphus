# Tauri 壳 · 视频字幕流水线（src-tauri/src/video/）

> 定位：**视频字幕三级降级提取流水线**——本地文件/在线 URL → 内嵌文本字幕流导出 → 平台字幕下载 → 本地 SenseVoice ASR，外加 yt-dlp/ffmpeg/ffprobe 外部工具的探测与自动下载，通过 Tauri 命令与 nuphus-lib 工具桥双通道对外提供服务。是 Agent 工具 `video_subtitle_extract` 的实际执行体。
> 规模：5 文件共 1796 行。

## 结构

| 文件 | 职责 |
|---|---|
| `mod.rs`（17） | 模块声明 + 三级降级架构总览注释 |
| `deps.rs`（348） | **外部工具供给**：yt-dlp/ffmpeg/ffprobe 三级探测链 + Windows 自动下载（镜像回退、min_size 反投毒、zip 释放 bin/*.exe）+ 进程 helpers |
| `pipeline.rs`（1164） | **流水线编排**：输入分流、ffprobe 探测、内嵌字幕导出、yt-dlp 信息/字幕/音频下载、cookie vault 注入与重试、ASR 尾部、进度事件、截断、分阶段耗时埋点、集成测试 |
| `subtitle.rs`（215） | **字幕解析归一化**：SRT/WebVTT 内容自动判别 → 统一 `Cue` 列表；无外部 crate 的块解析器 |
| `commands.rs`（52） | **对外命令面 + 工具桥注册**：Tauri command `video_extract_subtitles`、`init_bridge()` 注入 |

## 功能作用

三级降级（OCR 硬字幕明确不在范围）：

1. **embedded（内嵌）**：本地文件经 ffprobe 发现文本字幕流 → ffmpeg 导出 srt；
2. **platform（平台）**：在线 URL 经 yt-dlp 拿到官方/自动字幕 → 只下字幕（`--skip-download`）；
3. **asr（本地识别）**：前两级都失败 → 抽音轨/下载音频 → 16kHz WAV → 本地 SenseVoice 语音识别。

## 规则与坑

- **串行不流水**（产品决策）：下载必须完整结束后才进识别（"product decision: no download-while-transcribing"，`pipeline.rs:14`）。
- **诚实失败**：命令契约明确 Err 是可操作的错误信息——"never fabricated subtitles"（`commands.rs:4-6`）。
- 复用壳层驻留的 STT 引擎（RecognizerCache），并通过 fn 指针注入桥接进 Tauri-agnostic 的 nuphus lib（`src/video_bridge.rs` 是注入点）。
- AppState 持有 SpeechState（内含 speech.cache），本模块经它取识别器。
