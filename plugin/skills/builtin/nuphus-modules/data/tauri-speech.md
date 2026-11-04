# Tauri 壳 · STT 语音识别（src-tauri/src/speech/）

> 定位：**双引擎语音转文字子系统**——本地 sherpa-onnx（FunASR-Nano / sense-voice int8 离线识别 + silero VAD 端点检测 + cpal 麦克风采集 + 技术词典纠错）与云端 OpenAI 兼容 `/audio/transcriptions` 批量转写，共用一套 Tauri 命令/事件契约，外加按需模型下载管理。
> 规模：10 文件共 2823 行。

## 结构

| 文件 | 职责 |
|---|---|
| `commands.rs`（652） | **命令面与状态机**：stt_start/stop/cancel/status、session worker、事件 Sink、隐藏调试命令、视频字幕用结构化转写（Phase / SessionSlot / SpeechState / run_session） |
| `engine.rs`（437） | sherpa-onnx 安全封装：模型路径解析、识别器/VAD 句柄、懒加载共享缓存（RecognizerCache）、版本查询 |
| `pipeline.rs`（195） | VAD 切分 + 预滚回看 + 实时 partial 解码管道（mic 会话与文件回归共用） |
| `ffi.rs`（391） | 手写 sherpa-onnx C API 绑定（31 个 `#[repr(C)]` 结构 + 19 个 extern 函数，1:1 转录自 v1.13.4 c-api.h） |
| `mic.rs`（224）/ `wav.rs`（170） | cpal 麦克风采集（F32/I16/U16 → mono f32）+ 线性插值重采样；极简 RIFF/WAVE 编解码 |
| `cloud.rs`（129） | 云端 STT：capabilities.stt 解析为 CloudSttConfig + OpenAI 兼容 multipart 上传转写 |
| `dict.rs`（264） | 技术词封闭词表模糊后纠错（编辑距离 + 同首字母守卫，CJK 安全，74 词默认表） |
| `download.rs`（331） | 模型按需下载：镜像回退、指数退避、min_size 反投毒、断点跳过、全局单例锁 |

## 功能作用

- **本地路径（默认）**：cpal 采集（Windows 上即 WASAPI）→ 单声道下混 → 线性重采样到 16kHz → silero VAD 端点切分（带 300ms 预滚回看）→ sherpa-onnx 离线识别（sense-voice，language=zh，greedy_search）→ 封闭技术词典模糊纠错 → 逐段 `stt:final` + 进行中 `stt:partial`。
- **云端路径（cloud-first，2026-07-27 产品决策）**：当 `capabilities.stt` 在模型注册表中能解析出 provider+model 时，录音进入同一内存缓冲，停止时上传一个 wav，文本作为单条 `stt:final` 返回——无 VAD 分段、无 partial。
- **模型分发**：`stt_download_model` 后台线程把 model.int8.onnx / tokens.txt / silero_vad.onnx 下载进 `%APPDATA%\Nuphus\models\stt`，事件驱动进度。

## 规则与坑

- **音频全程驻留内存，不落临时 wav 文件**（mpsc channel → history Vec / Vec<f32>）。
- 所有原生调用集中在 ffi.rs 一层手写绑定；worker 线程 `catch_unwind` 包裹，任何失败只发 `stt:error` 而绝不 panic 外溢（`commands.rs:326-341`）。
- STT 模型是唯一保持用户手动触发的本地模型（models/bootstrap.rs 注释明示：除 STT 外所有本地模型都必须自动获取）。
