---
title: 内置工具参数手册
id: tools-internal
type: skill
tags: [工具, PDF, 图像, 视频, 音频, 语音克隆, 文档, invoke]
---

# 内置工具参数手册

> Nuphus 工具页 24 个内部机制命令。仅注册 invoke_handler，**不注册为 agent 工具**（LLM 不可经 execute_tool 调用）；用户经工具页手动使用。
> 本手册供 Agent 感知能力与引导用户，非用户文档。

## 能力总览（24）

| 分类 | 命令 | 输入 → 输出 |
|------|------|------------|
| PDF | `pdf_merge` | N 个 PDF → 1 个 |
| PDF | `pdf_compress` | PDF → PDF（prune 未引用对象） |
| PDF | `pdf_page_count` | PDF → 页数 |
| PDF | `pdf_extract_text` | PDF → 文本（逐页，默认上限 200 页） |
| PDF | `pdf_images_to_pdf` | N 图 → PDF（每图一页） |
| PDF | `pdf_extract_pages` | PDF+页码 → 新 PDF |
| PDF | `pdf_rotate` | PDF → PDF（90/180/270） |
| 图像 | `image_compress` | 图→图（限宽高+质量） |
| 图像 | `image_convert` | 图→图（按扩展名推断格式） |
| 图像 | `image_resize` | 图→图（contain 保比例，不放大） |
| 图像 | `image_info` | 图→宽高/大小/格式 |
| 图像 | `image_stitch` | N 图→长图（horizontal/vertical） |
| 图像 | `image_compress_batch` | N 图→目录（原名-out.扩展名） |
| 图像 | `image_convert_batch` | N 图→目录（目标格式） |
| 图像 | `image_resize_batch` | N 图→目录（目标宽高 contain） |
| 视频 | `video_compress` | 视频→视频（libx264 重编码） |
| 视频 | `video_extract_frames` | 视频→目录（frame_%04d.jpg） |
| 视频 | `video_info` | 视频→时长/编码/分辨率（ffprobe） |
| 视频 | `video_to_gif` | 视频→GIF（palettegen+paletteuse） |
| 视频 | `video_cut` | 视频→片段（-ss 快速 seek + -c copy） |
| 音频 | `video_extract_audio` | 视频/音频→音频（wav/mp3） |
| 音频 | `audio_convert` | 音频/视频→音频（mp3/wav/m4a/flac） |
| 音频 | `voice_clone` | 参考音频+文本→克隆语音（走云端） |
| 文档 | `doc_extract_text` | docx/pptx/xls/ods/odt/odp/pdf→文本 |

## 关键参数

### PDF（`src-tauri/src/commands/tools/pdf.rs`，lopdf）
- `pdf_merge(input_paths: Vec<String>, output_path)` — 多文件合并；输出扩展名必须 .pdf；单文件 ≤500MB
- `pdf_compress(input_path, output_path)` — 清理未引用对象后重写
- `pdf_page_count(path)` — 返回 pages
- `pdf_extract_text(path, max_pages: Option<u32>)` — 逐页 `--- Page N ---` 分隔
- `pdf_images_to_pdf(input_paths, output_path)` — 图片白名单 png/jpg/jpeg/bmp/gif/webp；单图像素 ≤5000 万
- `pdf_extract_pages(input_path, pages: Vec<u32>, output_path)` — 1-based，越界报错
- `pdf_rotate(input_path, output_path, degrees: u32, pages: Option<Vec<u32>>)` — degrees 仅 90/180/270

### 图像（`image.rs`，image crate）
- `image_compress(input, output, max_width: Option<u32>, max_height: Option<u32>, quality: Option<u8>)` — JPEG quality 默认 82；PNG Best+Adaptive
- `image_convert(input, output)` — 输出格式随扩展名（png/jpg/jpeg/bmp/gif/webp）
- `image_resize(input, output, width: u32, height: u32)` — contain，不放大
- `image_info(path)` — 宽高/大小/格式
- `image_stitch(input_paths, output, direction: Option<String>)` — horizontal 默认/vertical
- `image_compress_batch(input_paths, output_dir, max_width?, max_height?, quality?)` — 输出 `原名-out.扩展名`
- `image_convert_batch(input_paths, output_dir, format: String)` — 目标 png/jpg/jpeg/bmp/gif/webp；jpeg→jpg
- `image_resize_batch(input_paths, output_dir, width: u32, height: u32)` — contain 保比例，不放大；输出 `原名-out.扩展名`

### 视频（`video.rs`，ffmpeg）
- `video_compress(input, output, quality: Option<String>)` — low(1M/64k)/medium(2M/96k 默认)/high(4M/128k)
- `video_extract_frames(input, output_dir, interval: Option<f64>)` — 每 interval 秒一帧（0.001~3600）
- `video_info(path)` — ffprobe：duration_seconds(字符串)/codec/分辨率
- `video_to_gif(input, output, fps: Option<f64>, scale: Option<u32>)` — 输出必须 .gif；fps 默认 10(1-30)；scale 默认 480(16-2000)
- `video_cut(input, output, start_sec: Option<f64>, end_sec: Option<f64>)` — start 0~86400，end>start；流复制不重编码

### 音频（`video.rs` 音频部分 + `voice.rs`）
- `video_extract_audio(input, output, format: Option<String>)` — wav(pcm_s16le 44100 stereo)/默认 mp3(192k)
- `audio_convert(input, output, bitrate: Option<String>)` — 扩展名决定编码：mp3=libmp3lame/wav=pcm_s16le/m4a=aac/flac=flac；bitrate 如 "192k"（mp3/m4a 生效）
- `voice_clone(reference_path, text, output_path)` — **走云端**：读 capabilities.voice（providers.toml [capabilities] voice=模型ID）→ OpenAI 兼容 `POST {base_url}/audio/speech`；未配置返回「模型界面 → 图像音频配置 → 语音克隆 配置」引导；参考音频格式 mp3/wav/m4a/flac/ogg/aac；克隆音色由所选模型/云端默认 voice 处理（参考音频暂作触发凭证）

### 文档（`doc.rs`）
- `doc_extract_text(path)` — 复用 office.rs read_office：docx/pptx/xls/ods/odt/odp/pdf

### 生成（`src/tools/builtin/generation.rs`，agent 工具，非 invoke_handler 命令）
> `image_generate` / `video_generate` 是 **agent 工具**（LLM 可经 execute_tool / 工作流节点调用），不在上面 24 个 invoke_handler 命令里；参数由 registry schema 驱动，工具页不手工列参数。

**绑定语义（本轮重构的核心，改自「工具自行发现 MiniMax provider」）**
- 模型与凭证**只读模型界面的能力绑定**：providers.toml `[capabilities]` 的 `image_generation` / `video_generation` + 对应 `_provider` 归属（与 vision/stt/tts/voice 同一套机制，见 `src/config/mod.rs` 的 `resolve_generation_binding`）。
- 绑定入口：模型界面 → 图像音频模型 → **图片生成** / **视频生成**。用户绑定一次，工具默认读绑定。
- 工具参数 `provider` / `model` 都是**当次覆盖**，不是必填：`required` 只有 `["prompt"]`，且不带 schema 级 `default`（客户端回填 default 会无声覆盖绑定）。
- **未绑定且未覆盖 → 明确报错**，文案指明「去模型界面绑定」并列出可绑候选（注册表中声明了 `supports_image_generation` 的 `provider/model`）。禁止任何静默发现 / 静默回落主模型 / 静默挑第一个 provider。
- 模型查找只走 `find_model_for_provider`（provider 已知，精确）与 `find_model_candidates`（重名消歧）；`find_model` 段序首触是 legacy，禁用于能力决策。
- `base_url` 回落**只对 MiniMax 段**生效（未填时用内置默认端点）；其他协议类型的段缺 `base_url` 一律报错。

**`image_generate(prompt, model?, provider?, params?)`**
- `prompt` 必填；`model` / `provider` 可选，仅当次覆盖绑定。
- `params` 可选 JSON 对象，原样透传合并进请求体（`aspect_ratio`/`width`/`height`/`n`/`response_format` 及一切新参数；与顶层参数同名时**顶层优先**）。
- `response_format` 缺省 `url`（可在 `params` 里显式覆盖为 `base64`）。
- 宽高官方约束 512–2048 且 8 的倍数。
- 端点：`POST {base_url}/image_generation`，同步返回 `data.image_urls[]`（或 `data.image_base64[]`）。

**`video_generate(prompt, model?, provider?, params?, poll_interval_secs?, timeout_secs?)`**
- `prompt` 必填；`model` / `provider` 可选，仅当次覆盖绑定。
- `poll_interval_secs` 默认 5（1–60）；`timeout_secs` 默认 300（30–600）。超时 / 失败错误含 `task_id` 便于排查。
- `params` 透传：`duration`/`resolution`/`ratio`/`first_frame_image` 及一切新参数；图像输入按 MiniMax 要求以 URL 或 data URI 形式写进 `params`。
- V1/V2 路由按模型名自动判定（含 `H3` 走 V2，其余如 Hailuo-2.3 / MiniMax-Hailuo-02 / S2V-01 走 V1）。
- 端点：`POST {root}/v2/video_generation`（V2）或 `POST {base_url}/video_generation`（V1）→ 轮询 `query/video_generation` → `GET {base_url}/files/retrieve?file_id=` 取下载地址。

**落盘与消费**
- 产出目录 `~/.nuphus/generated/`（自动创建），文件名带时间戳。
- 工具返回值含**绝对路径**行（`.../image_<ts>_<i>.<ext>` / `.../video_<ts>.mp4`）+ 字节数；对话窗口的 MarkdownContent 只对绝对路径 + 白名单扩展名（含 `mp4`/`png`/`jpg`…）成链，点击后经 ChatPanel `previewPath` 打开 PreviewOverlay（视频/音频走 `convertFileSrc(path,'preview')`，图片走 base64 内联）。
- api_key 只进 Authorization 头，永不写入输出 / 日志 / 工作流文件。

## 调用边界

- 命令全部 `Result<serde_json::Value, String>`，错误中文可读，不 panic
- 注册：`src-tauri/src/main.rs` invoke_handler（`commands::tools::*`）；模块 `commands/tools.rs` 声明 `pub mod pdf/image/video/doc/voice`
- 前端：`ToolsPage.tsx`（六分类：全部/图片/视频/文档/音频/PDF，20+3 信息卡=23 能力卡，映射 24 命令）+ `lib/api.ts` wrappers
- 语音克隆模型配置：模型界面 → 图像音频配置 → 语音克隆（capabilities.voice）
- **生成模型配置：模型界面 → 图像音频模型 → 图片生成 / 视频生成（capabilities.image_generation / video_generation）**；未绑定时 image_generate / video_generate 直接报错并列候选
- 生成工具是 agent 工具（`src/tools/builtin/generation.rs`，registry schema 驱动；画布 ToolPalette 标签「生成图片」/「生成视频」），不经 invoke_handler
- 程序化执行：Rust 命令函数（内部代码/测试）或引擎二进制 `nuphus-tools-rs/target/release/nuphus-{pdf,image,video}.exe`（仅 merge/info/extract-text/compress 等子集）
- 引导用户：涉及 PDF/图像/视频/音频/文档处理 → 让用户打开工具页操作
