//! On-demand STT model download (user-confirmed, background thread).
//!
//! Contract (consumed by frontend VoiceButton download modal + ModelsPage
//! STT card via `useSttModelDownload`; see speech/commands.rs for the
//! recording contract — this module does NOT touch it):
//! - Command: `stt_download_model` — starts a background download of the STT
//!   model files into the data-dir candidate (`%APPDATA%\Nuphus\models\stt`,
//!   the same dir `engine::stt_dir_candidates()` probes). Returns immediately;
//!   Err("stt_download_busy") if a download is already running (progress
//!   events are broadcast app-wide, so a second caller can just follow them).
//! - Events: `stt:download` with a `kind`-tagged payload:
//!   { kind:"progress", file, downloaded, total, index, count }
//!   — throttled to ~1 MiB deltas (+ once per file completion);
//!   `total` is 0 when the server sends no Content-Length.
//!   { kind:"done" }               — all files present, exactly once.
//!   { kind:"error", message }     — terminal failure, exactly once.
//! - Files that already exist AND meet `min_size` are skipped (anti-poisoning:
//!   an undersized file is treated as an error page and re-downloaded), so a
//!   re-run resumes at file granularity. Partial files are deleted on failure.
//! - Sources: hf-mirror.com → huggingface.co for the ASR model repo
//!   (embed.rs mirror order), 3 attempts per source with 1s/2s exponential
//!   backoff. silero_vad.onnx has a single known source (k2-fsa GitHub
//!   release) — no HF mirror carries sherpa-onnx's custom vad build.
//!   URLs verified 2026-07-27 (sizes matched against a known-good install):
//!   model.int8.onnx 263,531,902 B / tokens.txt 939,815 B / silero 643,854 B.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// HF repo hosting the sense-voice FunASR-Nano int8 model + tokens.
const HF_REPO: &str = "csukuangfj/sherpa-onnx-sense-voice-funasr-nano-int8-2025-12-17";
/// Download mirrors in priority order (mirrors embed.rs: CN mirror first).
const MIRRORS: &[&str] = &["https://hf-mirror.com", "https://huggingface.co"];

/// Only one download may run at a time (VoiceButton + ModelsPage both expose
/// the entry point; events are global so losers can follow the winner).
static DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);

struct ModelFile {
    name: &'static str,
    /// Full URLs in fallback order (mirrors pre-applied).
    urls: Vec<String>,
    /// Minimum plausible size — below this the payload is an error page.
    min_size: u64,
}

fn model_files() -> Vec<ModelFile> {
    let hf = |file: &str| -> Vec<String> {
        MIRRORS
            .iter()
            .map(|m| format!("{m}/{HF_REPO}/resolve/main/{file}"))
            .collect()
    };
    vec![
        ModelFile {
            name: "model.int8.onnx",
            urls: hf("model.int8.onnx"),
            min_size: 100 * 1024 * 1024, // actual 263,531,902 B
        },
        ModelFile {
            name: "tokens.txt",
            urls: hf("tokens.txt"),
            min_size: 100 * 1024, // actual 939,815 B
        },
        ModelFile {
            name: "silero_vad.onnx",
            // Single verified source: k2-fsa GitHub release (643,854 B exact
            // match). HF mirrors only carry stock silero builds (different
            // bytes) — sherpa-onnx requires its own converted file.
            urls: vec![
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"
                    .to_string(),
            ],
            min_size: 100 * 1024,
        },
    ]
}

// ── Event payloads ──────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
enum DownloadEvent {
    #[serde(rename = "progress")]
    Progress {
        file: String,
        downloaded: u64,
        total: u64,
        /// 1-based index of the file being downloaded.
        index: usize,
        /// Total file count.
        count: usize,
    },
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "error")]
    Error { message: String },
}

fn emit(app: &AppHandle, ev: DownloadEvent) {
    let _ = app.emit("stt:download", ev);
}

// ── Download worker ─────────────────────────────────────────────────────

/// Directory the model files are downloaded into. Kept in sync with
/// `engine::stt_dir_candidates()` (data_dir branch) so the engine finds the
/// files without a restart of the path resolution logic.
fn target_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir().ok_or_else(|| "cannot resolve user data dir".to_string())?;
    Ok(data_dir.join("Nuphus").join("models").join("stt"))
}

/// One HTTP attempt: stream `url` to `path`, reporting progress.
/// Returns bytes written. Caller deletes the partial file on Err.
fn download_once(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<u64, String> {
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(path).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("读取数据失败: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入文件失败: {e}"))?;
        downloaded += n as u64;
        on_progress(downloaded, total);
    }
    Ok(downloaded)
}

fn run_download(app: &AppHandle) -> Result<(), String> {
    let dir = target_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模型目录失败: {e}"))?;
    tracing::info!("[stt-dl] downloading STT models into {}", dir.display());

    // No total timeout: 250 MiB over slow links exceeds any sane fixed cap
    // (embed.rs's 300s request timeout would abort big-file downloads).
    // connect_timeout bounds the stall case that matters in practice.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let files = model_files();
    let count = files.len();

    for (idx, mf) in files.iter().enumerate() {
        let path = dir.join(mf.name);
        let index = idx + 1;

        // Skip files that already exist with a plausible size (resumable
        // re-run); undersized files are treated as poisoned and re-fetched.
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() >= mf.min_size {
                tracing::info!("[stt-dl] 跳过已存在: {} ({} bytes)", mf.name, meta.len());
                emit(
                    app,
                    DownloadEvent::Progress {
                        file: mf.name.to_string(),
                        downloaded: meta.len(),
                        total: meta.len(),
                        index,
                        count,
                    },
                );
                continue;
            }
            tracing::warn!(
                "[stt-dl] {} 体积异常 ({} < {}), 重新下载",
                mf.name,
                meta.len(),
                mf.min_size
            );
        }

        // Progress events are throttled to ~1 MiB deltas to avoid flooding
        // the webview; file completion always emits.
        let mut last_emit = 0u64;
        let mut on_progress = |downloaded: u64, total: u64| {
            if downloaded == total || downloaded - last_emit >= 1024 * 1024 {
                last_emit = downloaded;
                emit(
                    app,
                    DownloadEvent::Progress {
                        file: mf.name.to_string(),
                        downloaded,
                        total,
                        index,
                        count,
                    },
                );
            }
        };

        let mut last_err = String::new();
        let mut downloaded = false;
        'sources: for url in &mf.urls {
            // Each source: 3 attempts (first + 2 retries, backoff 1s/2s) —
            // mirrors embed.rs.
            for attempt in 0..3u32 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                }
                tracing::info!(
                    "[stt-dl] 下载 {} ← {} (第 {} 次)",
                    mf.name,
                    url,
                    attempt + 1
                );
                match download_once(&client, url, &path, &mut on_progress) {
                    Ok(size) if size >= mf.min_size => {
                        tracing::info!("[stt-dl] 完成: {} ({} bytes)", mf.name, size);
                        downloaded = true;
                        break 'sources;
                    }
                    Ok(size) => {
                        last_err = format!(
                            "{} 体积异常 ({} bytes < 期望至少 {} bytes)",
                            mf.name, size, mf.min_size
                        );
                        tracing::warn!("[stt-dl] {last_err}");
                        let _ = std::fs::remove_file(&path);
                        break; // poisoned payload — retrying this source won't help
                    }
                    Err(e) => {
                        last_err = format!("{url}: {e}");
                        tracing::warn!("[stt-dl] 第 {} 次下载失败: {}", attempt + 1, last_err);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }

        if !downloaded {
            return Err(format!("{} 下载失败: {last_err}", mf.name));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn stt_download_model(app: AppHandle) -> Result<(), String> {
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("stt_download_busy".to_string());
    }
    let worker_app = app.clone();
    let spawn = std::thread::Builder::new()
        .name("stt-download".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_download(&worker_app)
            }));
            DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
            match result {
                Ok(Ok(())) => {
                    tracing::info!("[stt-dl] all model files ready");
                    emit(&worker_app, DownloadEvent::Done);
                }
                Ok(Err(e)) => {
                    tracing::warn!("[stt-dl] download failed: {e}");
                    emit(&worker_app, DownloadEvent::Error { message: e });
                }
                Err(_) => {
                    tracing::warn!("[stt-dl] download worker panicked");
                    emit(
                        &worker_app,
                        DownloadEvent::Error {
                            message: "download worker panicked (see stderr)".to_string(),
                        },
                    );
                }
            }
        });
    if let Err(e) = spawn {
        DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
        return Err(format!("failed to spawn stt download worker: {e}"));
    }
    Ok(())
}
// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The download manifest must stay in sync with the files
    /// `engine::check_files` requires — drift here silently breaks STT.
    #[test]
    fn manifest_covers_engine_required_files() {
        let files = model_files();
        let names: Vec<&str> = files.iter().map(|f| f.name).collect();
        assert_eq!(names, ["model.int8.onnx", "tokens.txt", "silero_vad.onnx"]);
        for f in &files {
            assert!(!f.urls.is_empty(), "{} has no download source", f.name);
            assert!(f.min_size > 0, "{} has no anti-poisoning floor", f.name);
            for u in &f.urls {
                assert!(u.starts_with("https://"), "non-https source: {u}");
            }
        }
    }

    /// target_dir must match engine::stt_dir_candidates' data_dir branch,
    /// otherwise downloaded files land where the engine never looks.
    #[test]
    fn target_dir_matches_engine_candidate() {
        let dir = target_dir().expect("data_dir resolvable on dev machine");
        assert!(dir.ends_with("Nuphus\\models\\stt") || dir.ends_with("Nuphus/models/stt"));
    }
}
