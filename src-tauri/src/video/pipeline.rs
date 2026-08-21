//! Orchestration: input → probe → three-tier degradation → normalized cues.
//!
//! ```text
//! input (local path | URL)
//!   local: ffprobe → text subtitle stream? ──yes──→ ffmpeg export srt (embedded)
//!                     │no
//!                     └→ audio track? ──yes──→ ffmpeg 16k wav → ASR (asr)
//!                          │no → explicit error (no subtitles AND no audio)
//!   URL:   yt-dlp -J（info.json 落盘复用）→ subtitles/automatic_captions? ──yes──→ yt-dlp subs (platform)
//!                     │no
//!                     └→ yt-dlp bestaudio -x → 16k wav（失败回退独立 ffmpeg convert）→ ASR (asr)
//! ```
//!
//! Sequential by design (product decision: no download-while-transcribing).
//! Progress is broadcast via `video:progress` events; temp files live in a
//! per-task dir under the system temp and are cleaned on every exit path.

use super::deps::{self, BinTool};
use super::subtitle::{self, Cue};
use crate::speech::engine::RecognizerCache;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

/// Hard cap on rendered subtitle text returned to the LLM (~150k chars).
pub const MAX_OUTPUT_CHARS: usize = 150_000;

/// Extraction result (tool bridge serializes this to JSON).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct VideoSubtitleResult {
    /// "embedded" | "platform" | "asr"
    pub source: String,
    pub title: Option<String>,
    pub duration_ms: Option<i64>,
    pub cues: Vec<Cue>,
    /// True when cues were dropped to stay under MAX_OUTPUT_CHARS.
    #[serde(default)]
    pub truncated: bool,
}

// ── Progress events ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct ProgressPayload {
    /// "probe" | "download_deps" | "fetch_subs" | "download_audio" |
    /// "convert" | "asr" | "done" | "error"
    stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    percent: Option<f32>,
    message: String,
}

pub fn emit_progress(
    app: &AppHandle,
    stage: &str,
    percent: Option<f32>,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "video:progress",
        ProgressPayload {
            stage: stage.to_string(),
            percent,
            message: message.into(),
        },
    );
}

// ── Temp task dir ───────────────────────────────────────────────────────

struct TaskDir(PathBuf);

impl TaskDir {
    fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir()
            .join("nuphus_video")
            .join(format!("{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {e}"))?;
        Ok(Self(dir))
    }
}

impl Drop for TaskDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ── Entry ───────────────────────────────────────────────────────────────

/// Progress sink: (stage, percent, message) — decouples the pipeline from
/// Tauri so it stays testable; the app layer maps this to `video:progress`.
pub type ProgressSink<'a> = &'a dyn Fn(&str, Option<f32>, String);

/// Full extraction, emitting `video:progress` Tauri events.
/// `cache` is the shared STT recognizer cache (resident engine reuse);
/// called on a blocking thread from both the Tauri command and the
/// nuphus-lib tool bridge.
pub fn run(
    app: &AppHandle,
    cache: &RecognizerCache,
    input: &str,
) -> Result<VideoSubtitleResult, String> {
    let sink = |stage: &str, percent: Option<f32>, message: String| {
        emit_progress(app, stage, percent, message)
    };
    run_core(cache, input, &sink)
}

/// UI-agnostic core: all progress goes through `sink`. Terminal stages
/// ("done" / "error") are emitted exactly once, here.
pub fn run_core(
    cache: &RecognizerCache,
    input: &str,
    sink: ProgressSink,
) -> Result<VideoSubtitleResult, String> {
    let result = run_inner(cache, input, sink);
    match &result {
        Ok(r) => sink(
            "done",
            Some(100.0),
            format!(
                "字幕获取完成（{}，{} 段）",
                source_label(&r.source),
                r.cues.len()
            ),
        ),
        Err(e) => sink("error", None, format!("字幕获取失败：{e}")),
    }
    result
}

fn source_label(source: &str) -> &'static str {
    match source {
        "embedded" => "内嵌字幕",
        "platform" => "平台字幕",
        "asr" => "本地语音识别",
        _ => "未知来源",
    }
}

fn run_inner(
    cache: &RecognizerCache,
    input: &str,
    sink: ProgressSink,
) -> Result<VideoSubtitleResult, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("输入为空".to_string());
    }
    let task_dir = TaskDir::new()?;
    let deps_progress = |label: &str, downloaded: u64, total: u64| {
        let msg = if total > 0 {
            format!(
                "正在下载依赖 {}（{:.0}%）",
                label,
                (downloaded as f64 / total as f64) * 100.0
            )
        } else {
            format!(
                "正在下载依赖 {}（已下载 {} MB）",
                label,
                downloaded / 1024 / 1024
            )
        };
        let percent = if total > 0 {
            Some(((downloaded as f64 / total as f64) * 100.0) as f32)
        } else {
            None
        };
        sink("download_deps", percent, msg);
    };

    if input.starts_with("http://") || input.starts_with("https://") {
        from_url(cache, input, &task_dir.0, sink, &deps_progress)
    } else if Path::new(input).is_file() {
        from_local(cache, input, &task_dir.0, sink, &deps_progress)
    } else {
        Err(format!(
            "无法识别的输入：「{}」既不是存在的本地文件，也不是 http(s) 视频链接",
            input
        ))
    }
}

// ── Local file path ─────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    tags: Option<ProbeTags>,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeTags {
    title: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

/// Text-based subtitle codecs ffmpeg can export to srt. Image-based ones
/// (dvd_subtitle / hdmv_pgs_subtitle / xsub) need OCR — out of scope, and
/// they make `-map 0:s:0` produce garbage, so we skip straight to ASR.
fn is_text_subtitle(codec: &str) -> bool {
    matches!(
        codec,
        "subrip" | "ass" | "ssa" | "webvtt" | "mov_text" | "text" | "srt"
    )
}

fn probe_media(ffprobe: &Path, path: &str) -> Result<ProbeOutput, String> {
    let out = deps::run_capture(
        deps::quiet_command(ffprobe)
            .arg("-v")
            .arg("error")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(path),
        "ffprobe 探测媒体信息",
    )?;
    serde_json::from_slice(&out.stdout).map_err(|e| format!("解析 ffprobe 输出失败: {e}"))
}

fn from_local(
    cache: &RecognizerCache,
    path: &str,
    tmp: &Path,
    sink: ProgressSink,
    deps_progress: &dyn Fn(&str, u64, u64),
) -> Result<VideoSubtitleResult, String> {
    sink("probe", None, "探测本地视频文件…".to_string());
    let (ffmpeg, ffprobe) = deps::ensure_ffmpeg_suite(deps_progress)?;
    let probe = probe_media(&ffprobe, path)?;

    let duration_ms = probe
        .format
        .as_ref()
        .and_then(|f| f.duration.as_ref())
        .and_then(|d| d.parse::<f64>().ok())
        .map(|d| (d * 1000.0) as i64);
    let title = probe
        .format
        .as_ref()
        .and_then(|f| f.tags.as_ref())
        .and_then(|t| t.title.as_ref())
        .cloned()
        .or_else(|| {
            Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        });

    // ① Embedded text subtitle stream. ffmpeg `-map 0:s:m` indexes among
    // subtitle streams only, so compute the ordinal within subtitle streams.
    let mut sub_ordinal = 0usize;
    let mut first_text_sub: Option<usize> = None;
    for s in &probe.streams {
        if s.codec_type.as_deref() == Some("subtitle") {
            if first_text_sub.is_none()
                && s.codec_name
                    .as_deref()
                    .map(is_text_subtitle)
                    .unwrap_or(false)
            {
                first_text_sub = Some(sub_ordinal);
            }
            sub_ordinal += 1;
        }
    }

    if let Some(ordinal) = first_text_sub {
        sink("fetch_subs", None, "导出内嵌字幕流…".to_string());
        let srt_path = tmp.join("embedded.srt");
        let export = deps::run_capture(
            deps::quiet_command(&ffmpeg)
                .arg("-y")
                .arg("-v")
                .arg("error")
                .arg("-i")
                .arg(path)
                .arg("-map")
                .arg(format!("0:s:{}", ordinal))
                .arg(&srt_path),
            "ffmpeg 导出字幕流",
        );
        if export.is_ok() {
            if let Ok(content) = std::fs::read_to_string(&srt_path) {
                let cues = subtitle::parse(&content);
                if !cues.is_empty() {
                    return Ok(build_result("embedded", title, duration_ms, cues));
                }
            }
        }
        tracing::warn!("[video] embedded subtitle export yielded nothing, falling back to ASR");
    }

    // ③ ASR fallback — requires an audio track.
    let has_audio = probe
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("audio"));
    if !has_audio {
        return Err("该视频既没有可用的文本字幕流，也不含音轨，无法获取字幕".to_string());
    }
    sink("convert", None, "抽取音轨并转换为 16kHz WAV…".to_string());
    let wav = extract_wav(&ffmpeg, path, tmp)?;
    asr_result(cache, &wav, title, duration_ms, sink)
}

// ── URL path ────────────────────────────────────────────────────────────

/// 从统一 cookie vault 导出该 URL host 适用的 Netscape cookies.txt 到任务
/// 临时目录（`TaskDir` 退出时自动删除，明文不落持久盘）。
/// 无可用 cookie 时返回 `None` —— yt-dlp 按原匿名方式运行，行为不变。
/// 安全约束：日志只记录域与条数，cookie 值永不进日志。
fn cookies_file_for(url: &str, tmp: &Path, force_refresh: bool) -> Option<PathBuf> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_string();
    let vault = nuphus::cookies::vault();
    let cookies = if force_refresh {
        vault.refresh_host(&host)
    } else {
        vault.cookies_for_host(&host)
    };
    if cookies.is_empty() {
        return None;
    }
    let path = tmp.join("cookies.txt");
    std::fs::write(&path, nuphus::cookies::to_netscape(&cookies))
        .map_err(|e| {
            tracing::warn!("[video] 写入临时 cookies 文件失败: {e}");
            e
        })
        .ok()?;
    tracing::info!(
        "[video] 为域 {} 注入 {} 条 cookie 到 yt-dlp",
        host,
        cookies.len()
    );
    Some(path)
}

/// yt-dlp stderr 中的 cookie/登录类报错特征（用于触发 refresh 重试）。
fn is_ytdlp_cookie_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    const PATTERNS: &[&str] = &[
        "sign in",
        "log in",
        "login",
        "cookie",
        "http error 403",
        "private video",
        "members-only",
        "members only",
        "only available to",
    ];
    PATTERNS.iter().any(|p| m.contains(p))
}

/// 执行 `attempt`；遇 cookie/登录类报错时调用 `refresh`（刷新 vault 中的
/// 域 cookie）并重试一次。其它错误直接返回，不重试。
fn retry_once_on_cookie_error<T>(
    is_cookie_error: fn(&str) -> bool,
    mut refresh: impl FnMut(),
    mut attempt: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    match attempt() {
        Err(first) if is_cookie_error(&first) => {
            refresh();
            attempt()
        }
        other => other,
    }
}

fn from_url(
    cache: &RecognizerCache,
    url: &str,
    tmp: &Path,
    sink: ProgressSink,
    deps_progress: &dyn Fn(&str, u64, u64),
) -> Result<VideoSubtitleResult, String> {
    // 分阶段耗时埋点：作用域退出（含全部错误路径）时汇总输出一次。
    let mut timings = StageTimings::default();

    sink("probe", None, "探测在线视频信息…".to_string());
    let t_probe = Instant::now();
    let ytdlp = deps::ensure(BinTool::YtDlp, deps_progress)?;

    // One JSON dump gives title/duration/subtitle tracks without downloading.
    // Cookie vault: 目标域有 cookie 时注入 --cookies；遇登录类报错刷新
    // vault 重试一次。
    let force_refresh = std::cell::Cell::new(false);
    let out = retry_once_on_cookie_error(
        is_ytdlp_cookie_error,
        || force_refresh.set(true),
        || {
            let cookies_file = cookies_file_for(url, tmp, force_refresh.get());
            let mut cmd = deps::quiet_command(&ytdlp);
            cmd.arg("--no-playlist").arg("--skip-download").arg("-J");
            if let Some(ref f) = cookies_file {
                cmd.arg("--cookies").arg(f);
            }
            cmd.arg(url);
            deps::run_capture(&mut cmd, "yt-dlp 获取视频信息")
        },
    )?;
    // info JSON 落盘，后续字幕/音频下载用 --load-info-json 复用（实测无需再传
    // URL，输出直接进入 [info] 阶段，无页面重新解析），消除重复的网络请求。
    // 写盘失败仅告警，回退为直接传 URL，行为与旧版一致。
    let info_json = match std::fs::write(tmp.join("info.json"), &out.stdout) {
        Ok(()) => Some(tmp.join("info.json")),
        Err(e) => {
            tracing::warn!("[video] 写入 info.json 失败，后续 yt-dlp 调用回退为传 URL: {e}");
            None
        }
    };
    timings.record("probe", t_probe);
    let info: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("解析 yt-dlp 输出失败: {e}"))?;
    let title = info
        .get("title")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let duration_ms = info
        .get("duration")
        .and_then(|d| d.as_f64())
        .map(|d| (d * 1000.0) as i64);

    // ② Platform subtitles: official first, then auto captions.
    let t_subs = Instant::now();
    let pick = pick_subtitle_track(&info);
    if let Some((lang, is_auto)) = pick {
        sink(
            "fetch_subs",
            None,
            format!(
                "下载平台字幕（{}{}）…",
                lang,
                if is_auto { "，自动生成" } else { "" }
            ),
        );
        let mut cmd = deps::quiet_command(&ytdlp);
        cmd.arg("--no-playlist")
            .arg("--skip-download")
            .arg(if is_auto {
                "--write-auto-subs"
            } else {
                "--write-subs"
            })
            .arg("--sub-langs")
            .arg(&lang)
            .arg("--sub-format")
            .arg("srt/vtt/best")
            .arg("-o")
            .arg(tmp.join("sub.%(ext)s"));
        if let Some(f) = cookies_file_for(url, tmp, false) {
            cmd.arg("--cookies").arg(f);
        }
        // 复用 probe 落盘的 info JSON，不再重新解析页面。
        match info_json {
            Some(ref f) => {
                cmd.arg("--load-info-json").arg(f);
            }
            None => {
                cmd.arg(url);
            }
        }
        // Subtitle download failure must not be fatal — fall through to ASR.
        if deps::run_capture(&mut cmd, "yt-dlp 下载字幕").is_ok() {
            if let Some(cues) = read_subs_dir(tmp) {
                if !cues.is_empty() {
                    timings.record("fetch_subs", t_subs);
                    return Ok(build_result("platform", title, duration_ms, cues));
                }
            }
        } else {
            tracing::warn!("[video] yt-dlp subtitle download failed, falling back to ASR");
        }
    }
    timings.record("fetch_subs", t_subs);

    // ③ ASR fallback: download audio, convert, transcribe.
    sink(
        "download_audio",
        None,
        "无可用字幕，下载音频（这一步可能较慢）…".to_string(),
    );
    let t_dl = Instant::now();
    // -x 后处理在下载时直接产出 16kHz mono WAV（实测 pcm_s16le/16000Hz/1ch），
    // 省去独立 ffmpeg convert 进程；yt-dlp 需能定位 ffmpeg——绝对路径时通过
    // --ffmpeg-location 指向其所在目录，PATH 上的裸命令则不传（yt-dlp 会自行
    // 搜索 PATH；实测传错误目录会直接硬失败且无 PATH 回退）。
    let (ffmpeg, _) = deps::ensure_ffmpeg_suite(deps_progress)?;
    let force_refresh = std::cell::Cell::new(false);
    let dl = retry_once_on_cookie_error(
        is_ytdlp_cookie_error,
        || force_refresh.set(true),
        || {
            let cookies_file = cookies_file_for(url, tmp, force_refresh.get());
            let mut cmd = deps::quiet_command(&ytdlp);
            cmd.arg("--no-playlist")
                .arg("-f")
                .arg("bestaudio/best")
                .arg("-x")
                .arg("--audio-format")
                .arg("wav")
                .arg("--postprocessor-args")
                .arg("ffmpeg:-ac 1 -ar 16000")
                .arg("-o")
                .arg(tmp.join("audio_src.%(ext)s"));
            if ffmpeg.is_absolute() {
                if let Some(dir) = ffmpeg.parent() {
                    cmd.arg("--ffmpeg-location").arg(dir);
                }
            }
            if let Some(ref f) = cookies_file {
                cmd.arg("--cookies").arg(f);
            }
            // 复用 probe 落盘的 info JSON，不再重新解析页面。
            match info_json {
                Some(ref f) => {
                    cmd.arg("--load-info-json").arg(f);
                }
                None => {
                    cmd.arg(url);
                }
            }
            deps::run_capture(&mut cmd, "yt-dlp 下载音频")
        },
    );
    timings.record("download_audio", t_dl);

    match resolve_downloaded_audio(tmp, dl.map(|_| ()))? {
        // -x 直出 16k mono WAV：跳过独立 ffmpeg convert。
        DownloadedAudio::ReadyWav(wav) => {
            tracing::info!("[video] yt-dlp 后处理直出 16kHz mono WAV，跳过独立 convert");
            let t_asr = Instant::now();
            let r = asr_result(cache, &wav, title, duration_ms, sink);
            timings.record("asr", t_asr);
            r
        }
        // 回退路径：postprocessor 失败/产物缺失时仍走原独立 ffmpeg convert。
        DownloadedAudio::Raw(audio_src) => {
            sink("convert", None, "转换音频为 16kHz WAV…".to_string());
            let t_convert = Instant::now();
            let wav = tmp.join("audio.wav");
            deps::run_capture(
                deps::quiet_command(&ffmpeg)
                    .arg("-y")
                    .arg("-v")
                    .arg("error")
                    .arg("-i")
                    .arg(&audio_src)
                    .arg("-vn")
                    .arg("-ac")
                    .arg("1")
                    .arg("-ar")
                    .arg("16000")
                    .arg("-f")
                    .arg("wav")
                    .arg(&wav),
                "ffmpeg 转换音频",
            )?;
            timings.record("convert", t_convert);
            let t_asr = Instant::now();
            let r = asr_result(cache, &wav, title, duration_ms, sink);
            timings.record("asr", t_asr);
            r
        }
    }
}

/// Pick (lang, is_auto): prefer zh official → en official → any official →
/// zh auto → en auto → any auto.
fn pick_subtitle_track(info: &serde_json::Value) -> Option<(String, bool)> {
    fn pick_from(map: Option<&serde_json::Value>) -> Option<String> {
        let obj = map?.as_object()?;
        if obj.is_empty() {
            return None;
        }
        let mut langs: Vec<&String> = obj.keys().collect();
        langs.sort();
        langs
            .iter()
            .find(|l| l.starts_with("zh"))
            .or_else(|| langs.iter().find(|l| l.starts_with("en")))
            .or_else(|| langs.first())
            .map(|l| l.to_string())
    }
    if let Some(lang) = pick_from(info.get("subtitles")) {
        return Some((lang, false));
    }
    pick_from(info.get("automatic_captions")).map(|lang| (lang, true))
}

/// Find the first *.srt / *.vtt produced in `dir` and parse it.
fn read_subs_dir(dir: &Path) -> Option<Vec<Cue>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()),
                Some(ext) if ext == "srt" || ext == "vtt"
            )
        })
        .collect();
    files.sort();
    for f in files {
        if let Ok(content) = std::fs::read_to_string(&f) {
            let cues = subtitle::parse(&content);
            if !cues.is_empty() {
                return Some(cues);
            }
        }
    }
    None
}

/// URL pipeline 分阶段耗时（毫秒）。作用域退出时汇总输出一次——覆盖成功与
/// 所有错误路径（done/error 之前必然经过 Drop）。内容仅阶段名与毫秒数，
/// 不含 URL、cookie 等敏感信息。
#[derive(Default)]
struct StageTimings(Vec<(&'static str, u128)>);

impl StageTimings {
    fn record(&mut self, stage: &'static str, start: Instant) {
        self.0.push((stage, start.elapsed().as_millis()));
    }
}

impl Drop for StageTimings {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let parts: Vec<String> = self
            .0
            .iter()
            .map(|(stage, ms)| format!("{stage}={ms}ms"))
            .collect();
        let total: u128 = self.0.iter().map(|(_, ms)| ms).sum();
        tracing::info!(
            "[video] URL pipeline timings: {} total={}ms",
            parts.join(" "),
            total
        );
    }
}

/// 音频下载阶段的产物判定。
#[derive(Debug)]
enum DownloadedAudio {
    /// `-x` 后处理直出的 16kHz mono WAV —— 跳过独立 convert。
    ReadyWav(PathBuf),
    /// 原始下载媒体（postprocessor 失败/产物缺失的回退）—— 仍需独立 convert。
    Raw(PathBuf),
}

/// 根据 yt-dlp 退出状态与产物文件判定后续路径：
/// - 下载成功且有 .wav 产物 → ReadyWav（后处理已产出 16k mono WAV）
/// - postprocessor 失败但留下产物（含半成品 wav）→ Raw（回退独立 convert）
/// - 下载失败且无任何产物 → 原样返回下载错误
/// - 下载成功但无产物 → 显式报错
fn resolve_downloaded_audio(
    tmp: &Path,
    download: Result<(), String>,
) -> Result<DownloadedAudio, String> {
    let mut wav = None;
    let mut raw = None;
    if let Ok(entries) = std::fs::read_dir(tmp) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) != Some("audio_src") {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) == Some("wav") {
                wav = Some(path);
            } else {
                raw = Some(path);
            }
        }
    }
    match (download, wav, raw) {
        (Ok(()), Some(w), _) => Ok(DownloadedAudio::ReadyWav(w)),
        (Err(e), Some(w), None) => {
            // postprocessor 失败但留下 wav 半成品：交给 ffmpeg convert 尽力挽救
            tracing::warn!("[video] yt-dlp 后处理失败（{e}），尝试用残留产物走独立 convert");
            Ok(DownloadedAudio::Raw(w))
        }
        (_, _, Some(r)) => Ok(DownloadedAudio::Raw(r)),
        (Ok(()), None, None) => Err("yt-dlp 报告成功但未找到音频文件".to_string()),
        (Err(e), None, None) => Err(e),
    }
}

// ── Shared ASR tail ─────────────────────────────────────────────────────

fn extract_wav(ffmpeg: &Path, media: &str, tmp: &Path) -> Result<PathBuf, String> {
    let wav = tmp.join("audio.wav");
    deps::run_capture(
        deps::quiet_command(ffmpeg)
            .arg("-y")
            .arg("-v")
            .arg("error")
            .arg("-i")
            .arg(media)
            .arg("-vn")
            .arg("-ac")
            .arg("1")
            .arg("-ar")
            .arg("16000")
            .arg("-f")
            .arg("wav")
            .arg(&wav),
        "ffmpeg 抽取音轨",
    )?;
    Ok(wav)
}

fn asr_result(
    cache: &RecognizerCache,
    wav: &Path,
    title: Option<String>,
    duration_ms: Option<i64>,
    sink: ProgressSink,
) -> Result<VideoSubtitleResult, String> {
    sink(
        "asr",
        None,
        "本地语音识别中（SenseVoice，耗时与时长成正比）…".to_string(),
    );
    let segments = crate::speech::commands::transcribe_wav_segments(cache, wav)?;
    if segments.is_empty() {
        return Err("语音识别未产出任何内容（可能是无语音的纯画面/音乐视频）".to_string());
    }
    let cues: Vec<Cue> = segments
        .into_iter()
        .map(|s| Cue {
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text,
        })
        .collect();
    Ok(build_result("asr", title, duration_ms, cues))
}

fn build_result(
    source: &str,
    title: Option<String>,
    duration_ms: Option<i64>,
    cues: Vec<Cue>,
) -> VideoSubtitleResult {
    let mut truncated = false;
    let mut total = 0usize;
    let mut kept = cues.len();
    for (i, cue) in cues.iter().enumerate() {
        total += cue.text.chars().count() + 12; // "[mm:ss] " + newline overhead
        if total > MAX_OUTPUT_CHARS {
            kept = i;
            truncated = true;
            break;
        }
    }
    let mut cues = cues;
    cues.truncate(kept);
    VideoSubtitleResult {
        source: source.to_string(),
        title,
        duration_ms,
        cues,
        truncated,
    }
}

// ── Integration tests (real verification) ───────────────────────────────
//
// All #[ignore] — they spawn real ffmpeg/yt-dlp and (for ASR) load the
// SenseVoice model. Run explicitly:
//   cargo test -p nuphus-desktop --target-dir target\verify video::pipeline::tests -- --ignored --nocapture --test-threads=1
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_sink() -> impl Fn(&str, Option<f32>, String) {
        |stage: &str, percent: Option<f32>, msg: String| {
            eprintln!("[progress] {:?} {:?} {}", stage, percent, msg);
        }
    }

    fn test_cache() -> &'static RecognizerCache {
        static CACHE: std::sync::OnceLock<RecognizerCache> = std::sync::OnceLock::new();
        CACHE.get_or_init(RecognizerCache::default)
    }

    fn fixture_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("nuphus_video_test_fixtures");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_ffmpeg(args: &[&str]) {
        let ffmpeg = deps::locate(BinTool::Ffmpeg).expect("ffmpeg required for fixture build");
        deps::run_capture(deps::quiet_command(&ffmpeg).args(args), "fixture ffmpeg")
            .expect("fixture ffmpeg failed");
    }

    /// ① embedded: mkv with a subrip stream → export path, exact cue text.
    #[test]
    #[ignore]
    fn embedded_subtitle_local() {
        let dir = fixture_dir();
        let srt = dir.join("fixture.srt");
        std::fs::write(
            &srt,
            "1\n00:00:00,500 --> 00:00:02,000\n内嵌字幕第一句\n\n2\n00:00:02,500 --> 00:00:04,000\n内嵌字幕第二句\n",
        )
        .unwrap();
        let mkv = dir.join("fixture_embedded.mkv");
        run_ffmpeg(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:d=5:r=10",
            "-i",
            srt.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-c:s",
            "srt",
            mkv.to_str().unwrap(),
        ]);

        let r = run_core(test_cache(), mkv.to_str().unwrap(), &make_sink())
            .expect("embedded extraction failed");
        assert_eq!(r.source, "embedded");
        assert!(r.cues.len() >= 2, "expected >=2 cues, got {}", r.cues.len());
        assert_eq!(r.cues[0].text, "内嵌字幕第一句");
        assert_eq!(r.cues[0].start_ms, 500);
        eprintln!(
            "[verify] embedded OK: {} cues, title={:?}",
            r.cues.len(),
            r.title
        );
    }

    /// ③ asr: black video + live.wav speech audio → local SenseVoice segments.
    #[test]
    #[ignore]
    fn asr_local_with_speech() {
        let live = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools")
            .join("stt-proto")
            .join("live.wav");
        if !live.is_file() {
            eprintln!("[verify] live.wav missing, skipping");
            return;
        }
        let dir = fixture_dir();
        let mp4 = dir.join("fixture_asr.mp4");
        run_ffmpeg(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:r=10",
            "-i",
            live.to_str().unwrap(),
            "-shortest",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            mp4.to_str().unwrap(),
        ]);

        let r = run_core(test_cache(), mp4.to_str().unwrap(), &make_sink())
            .expect("asr extraction failed");
        assert_eq!(r.source, "asr");
        assert!(!r.cues.is_empty(), "ASR produced no segments");
        assert!(r.cues[0].end_ms > r.cues[0].start_ms);
        eprintln!(
            "[verify] asr OK: {} cues, first: [{}ms..{}ms] {}",
            r.cues.len(),
            r.cues[0].start_ms,
            r.cues[0].end_ms,
            r.cues[0].text
        );
    }

    /// Error path: video-only file (no subtitles, no audio track).
    #[test]
    #[ignore]
    fn no_audio_no_subtitles_errors_honestly() {
        let dir = fixture_dir();
        let mp4 = dir.join("fixture_mute.mp4");
        run_ffmpeg(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240:d=3:r=10",
            "-an",
            "-c:v",
            "libx264",
            mp4.to_str().unwrap(),
        ]);
        let err = run_core(test_cache(), mp4.to_str().unwrap(), &make_sink())
            .expect_err("must fail honestly");
        assert!(err.contains("音轨"), "unexpected error: {err}");
        eprintln!("[verify] no-audio error OK: {err}");
    }

    /// Error path: unrecognizable input (neither file nor URL).
    #[test]
    #[ignore]
    fn invalid_input_errors_honestly() {
        let err = run_core(
            test_cache(),
            "C:\\definitely\\not\\exists.mp4",
            &make_sink(),
        )
        .expect_err("must fail honestly");
        assert!(err.contains("无法识别的输入"), "unexpected error: {err}");
        eprintln!("[verify] invalid-input error OK: {err}");
    }

    /// Error path: URL that is not a video page.
    #[test]
    #[ignore]
    fn invalid_url_errors_honestly() {
        let err = run_core(
            test_cache(),
            "https://example.com/definitely-not-a-video",
            &make_sink(),
        )
        .expect_err("must fail honestly");
        assert!(err.contains("yt-dlp"), "unexpected error: {err}");
        eprintln!(
            "[verify] invalid-url error OK: {}",
            &err[..err.len().min(200)]
        );
    }

    /// ② platform: online video with subtitles → source=platform, fast.
    /// URL is supplied via env to avoid hardcoding a rotting link:
    ///   set NUPHUS_VIDEO_TEST_URL=https://...
    #[test]
    #[ignore]
    fn url_platform_subtitles() {
        let Ok(url) = std::env::var("NUPHUS_VIDEO_TEST_URL") else {
            eprintln!("[verify] NUPHUS_VIDEO_TEST_URL not set, skipping");
            return;
        };
        let t0 = std::time::Instant::now();
        let r = run_core(test_cache(), &url, &make_sink()).expect("platform extraction failed");
        let elapsed = t0.elapsed();
        eprintln!(
            "[verify] url OK: source={} title={:?} cues={} elapsed={:?}",
            r.source,
            r.title,
            r.cues.len(),
            elapsed
        );
        assert_eq!(r.source, "platform");
        assert!(!r.cues.is_empty());
    }

    /// Truncation: >150k chars of cues must be cut and marked.
    #[test]
    fn truncation_marks_and_caps() {
        let cues: Vec<Cue> = (0..20_000)
            .map(|i| Cue {
                start_ms: i * 1000,
                end_ms: i * 1000 + 900,
                text: "这是一段用于测试截断行为的中文字幕文本".repeat(2),
            })
            .collect();
        let r = build_result("platform", None, None, cues);
        assert!(r.truncated);
        let total: usize = r.cues.iter().map(|c| c.text.chars().count() + 12).sum();
        assert!(total <= MAX_OUTPUT_CHARS);
    }

    /// Silence sink counter — ensures done/error terminal exactly once.
    #[test]
    fn terminal_event_exactly_once() {
        static N: AtomicUsize = AtomicUsize::new(0);
        N.store(0, Ordering::SeqCst);
        let sink = |stage: &str, _p: Option<f32>, _m: String| {
            if stage == "done" || stage == "error" {
                N.fetch_add(1, Ordering::SeqCst);
            }
        };
        let _ = run_core(test_cache(), "", &sink);
        assert_eq!(N.load(Ordering::SeqCst), 1);
    }

    // ── cookie 重试逻辑 ──

    #[test]
    fn ytdlp_cookie_error_patterns() {
        assert!(is_ytdlp_cookie_error("ERROR: Sign in to confirm your age"));
        assert!(is_ytdlp_cookie_error(
            "ERROR: unable to download video: HTTP Error 403: Forbidden"
        ));
        assert!(is_ytdlp_cookie_error(
            "This video is only available to members"
        ));
        assert!(is_ytdlp_cookie_error("ERROR: Private video. Use --cookies"));
        assert!(!is_ytdlp_cookie_error("ERROR: Video unavailable"));
        assert!(!is_ytdlp_cookie_error("ERROR: Unsupported URL"));
        assert!(!is_ytdlp_cookie_error("网络连接超时"));
    }

    #[test]
    fn retry_triggers_once_on_cookie_error() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        static REFRESHES: AtomicUsize = AtomicUsize::new(0);
        ATTEMPTS.store(0, Ordering::SeqCst);
        REFRESHES.store(0, Ordering::SeqCst);

        let r: Result<i32, String> = retry_once_on_cookie_error(
            is_ytdlp_cookie_error,
            || {
                REFRESHES.fetch_add(1, Ordering::SeqCst);
            },
            || {
                let n = ATTEMPTS.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err("ERROR: Sign in to confirm".to_string())
                } else {
                    Ok(42)
                }
            },
        );
        assert_eq!(r.unwrap(), 42);
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
        assert_eq!(REFRESHES.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retry_skipped_for_non_cookie_error() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        static REFRESHES: AtomicUsize = AtomicUsize::new(0);
        ATTEMPTS.store(0, Ordering::SeqCst);
        REFRESHES.store(0, Ordering::SeqCst);

        let r: Result<i32, String> = retry_once_on_cookie_error(
            is_ytdlp_cookie_error,
            || {
                REFRESHES.fetch_add(1, Ordering::SeqCst);
            },
            || {
                ATTEMPTS.fetch_add(1, Ordering::SeqCst);
                Err("ERROR: Unsupported URL".to_string())
            },
        );
        assert!(r.is_err());
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
        assert_eq!(REFRESHES.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn retry_returns_second_error_when_still_failing() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        ATTEMPTS.store(0, Ordering::SeqCst);

        let r: Result<i32, String> = retry_once_on_cookie_error(
            is_ytdlp_cookie_error,
            || {},
            || {
                let n = ATTEMPTS.fetch_add(1, Ordering::SeqCst);
                Err(format!("cookie error attempt {}", n))
            },
        );
        let err = r.unwrap_err();
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
        assert!(err.contains("attempt 1"));
    }

    // ── 音频下载产物判定（-x 直出 WAV vs 回退独立 convert）──

    fn resolve_fixture_dir(tag: &str) -> PathBuf {
        let dir = fixture_dir().join(format!("resolve_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_ready_wav_when_download_ok() {
        let dir = resolve_fixture_dir("ready");
        std::fs::write(dir.join("audio_src.wav"), b"x").unwrap();
        match resolve_downloaded_audio(&dir, Ok(())).unwrap() {
            DownloadedAudio::ReadyWav(p) => assert_eq!(p, dir.join("audio_src.wav")),
            DownloadedAudio::Raw(_) => panic!("expected ReadyWav"),
        }
    }

    #[test]
    fn resolve_prefers_ready_wav_over_raw() {
        let dir = resolve_fixture_dir("both");
        std::fs::write(dir.join("audio_src.m4a"), b"x").unwrap();
        std::fs::write(dir.join("audio_src.wav"), b"x").unwrap();
        match resolve_downloaded_audio(&dir, Ok(())).unwrap() {
            DownloadedAudio::ReadyWav(p) => assert_eq!(p, dir.join("audio_src.wav")),
            DownloadedAudio::Raw(_) => panic!("expected ReadyWav"),
        }
    }

    #[test]
    fn resolve_raw_fallback_when_no_wav() {
        let dir = resolve_fixture_dir("raw");
        std::fs::write(dir.join("audio_src.mp4"), b"x").unwrap();
        match resolve_downloaded_audio(&dir, Ok(())).unwrap() {
            DownloadedAudio::Raw(p) => assert_eq!(p, dir.join("audio_src.mp4")),
            DownloadedAudio::ReadyWav(_) => panic!("expected Raw"),
        }
    }

    /// postprocessor 失败（yt-dlp 非零退出）但原始下载产物还在 → 回退 convert。
    #[test]
    fn resolve_raw_fallback_when_postprocessor_failed() {
        let dir = resolve_fixture_dir("pp_failed");
        std::fs::write(dir.join("audio_src.m4a"), b"x").unwrap();
        match resolve_downloaded_audio(&dir, Err("ERROR: Postprocessing".to_string())).unwrap() {
            DownloadedAudio::Raw(p) => assert_eq!(p, dir.join("audio_src.m4a")),
            DownloadedAudio::ReadyWav(_) => panic!("expected Raw"),
        }
    }

    #[test]
    fn resolve_propagates_error_when_no_artifact() {
        let dir = resolve_fixture_dir("empty");
        let err = resolve_downloaded_audio(&dir, Err("network down".to_string())).unwrap_err();
        assert!(err.contains("network down"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_errors_when_success_but_no_artifact() {
        let dir = resolve_fixture_dir("success_empty");
        let err = resolve_downloaded_audio(&dir, Ok(())).unwrap_err();
        assert!(err.contains("未找到音频文件"), "unexpected error: {err}");
    }
}
