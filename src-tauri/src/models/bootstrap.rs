//! Runtime vision-model bootstrap — auto-download PaddleOCR + YOLO on first run.
//!
//! Product principle: *every local model except STT must be obtained
//! automatically* — end users are non-technical and must never download a model
//! by hand. STT stays user-triggered (`speech/download.rs`); everything else
//! self-heals at startup via this module.
//!
//! Contract (consumed by the ModelsPage vision card via `useVisionModelDownload`):
//! - Command: `vision_models_status()` → `{ocr_ready, yolo_ready, missing[],
//!   dir, downloading}` (synchronous, no side effects).
//! - Trigger: `ensure_vision_models(&app)` — non-blocking. If every vision file
//!   is already present it returns immediately; otherwise it single-flights an
//!   OS thread `"vision-model-download"` and returns. `preload_ocr` invokes it,
//!   so `invoke('preload_ocr')` doubles as the ModelsPage "retry" entry point.
//!   A second caller while a download runs follows the global event stream
//!   (events are broadcast app-wide).
//! - Events: `models:download` with a `kind`-tagged payload:
//!   { kind:"progress", file, downloaded, total, index, count }
//!   — throttled to ~1 MiB deltas (+ once per file completion);
//!   `total` is 0 when the server sends no Content-Length.
//!   { kind:"done", ocr_ready, yolo_ready } — terminal, exactly once.
//!   { kind:"error", message }             — terminal, exactly once.
//! - Files that already exist AND meet `min_size` are skipped (anti-poisoning:
//!   an undersized file is treated as an error page and re-downloaded), so a
//!   re-run resumes at file granularity. Partial files are deleted on failure.
//! - YOLO is *optional*: a YOLO download failure degrades to OCR-only and does
//!   NOT end in a terminal error event (OCR failures still do).
//! - Mirror order mirrors `speech/download.rs`: hf-mirror.com → huggingface.co.
//!   The OCR dict uses gitee (primary) with a raw.githubusercontent fallback.
//!
//! URL/size sanity: det ~4.7 MB, rec ~10.8 MB, dict ~26 KB (verified against
//! the existing `src-tauri/desktop/models/` assets), YOLO 12.2 MB fp32
//! (onnx-community OmniParser export — official OmniParser only ships .pt).

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Download mirrors in priority order (mirrors embed.rs / speech/download.rs:
/// CN mirror first).
const MIRRORS: &[&str] = &["https://hf-mirror.com", "https://huggingface.co"];

/// Only one vision download may run at a time.
static DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);

// ── Manifest ───────────────────────────────────────────────────────────

struct ModelFile {
    name: &'static str,
    /// Full URLs in fallback order (mirrors pre-applied).
    urls: Vec<String>,
    /// Minimum plausible size — below this the payload is an error page.
    min_size: u64,
    /// `true` = OCR must succeed (terminal error on failure);
    /// `false` = YOLO, optional (degrade to OCR-only on failure).
    required: bool,
}

/// Files the runtime actually reads (kept in sync with `src/desktop/
/// paddle_ocr.rs` L119-121 and `src/desktop/yolo_detect.rs` L46).
fn vision_files() -> Vec<ModelFile> {
    let hf = |repo: &str, file: &str| -> Vec<String> {
        MIRRORS
            .iter()
            .map(|m| format!("{m}/{repo}/resolve/main/{file}"))
            .collect()
    };
    vec![
        ModelFile {
            name: "ch_PP-OCRv4_det.onnx",
            urls: hf(
                "SWHL/RapidOCR",
                "PP-OCRv4/ch_PP-OCRv4_det_infer.onnx",
            ),
            min_size: 2 * 1024 * 1024, // actual ~4.7 MB
            required: true,
        },
        ModelFile {
            name: "ch_PP-OCRv4_rec.onnx",
            urls: hf(
                "SWHL/RapidOCR",
                "PP-OCRv4/ch_PP-OCRv4_rec_infer.onnx",
            ),
            min_size: 4 * 1024 * 1024, // actual ~10.8 MB
            required: true,
        },
        ModelFile {
            name: "ch_PP-OCR_keys_v1.txt",
            urls: vec![
                // Primary: gitee (mainland CDN). Fallback: raw.githubusercontent.
                "https://gitee.com/paddlepaddle/PaddleOCR/raw/main/ppocr/utils/ppocr_keys_v1.txt"
                    .to_string(),
                "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/ppocr_keys_v1.txt"
                    .to_string(),
            ],
            min_size: 1024, // actual ~26 KB
            required: true,
        },
        ModelFile {
            name: "icon_detect.onnx",
            // onnx-community export of OmniParser icon_detect (640×640 fp32).
            urls: hf(
                "onnx-community/OmniParser-icon_detect_640x640",
                "onnx/model.onnx",
            ),
            min_size: 1024 * 1024, // actual ~12.2 MB
            required: false,
        },
    ]
}

// ── Event payloads ──────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
enum ModelsEvent {
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
    Done { ocr_ready: bool, yolo_ready: bool },
    #[serde(rename = "error")]
    Error { message: String },
}

fn emit(app: &AppHandle, ev: ModelsEvent) {
    let _ = app.emit("models:download", ev);
}

// ── Status ─────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VisionModelsStatus {
    pub ocr_ready: bool,
    pub yolo_ready: bool,
    /// Names of files below their anti-poisoning floor (missing or undersized).
    pub missing: Vec<String>,
    /// Directory downloads land in (None if data_dir is unresolvable).
    pub dir: Option<String>,
    pub downloading: bool,
}

/// Write target for downloads (mirrors desktop-api's `models_dir_for_write`:
/// `NUPHUS_MODELS_DIR` → `data_dir/Nuphus/models`), creating it if needed.
/// Kept local so this module stays self-contained and the read path
/// `nuphus::desktop::resolve_models_dir` hits the same dir.
fn models_dir_for_write() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("NUPHUS_MODELS_DIR") {
        let p = PathBuf::from(&dir);
        std::fs::create_dir_all(&p)
            .map_err(|e| format!("创建模型目录失败 {}: {e}", p.display()))?;
        return Ok(p);
    }
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "无法定位用户数据目录 (dirs::data_dir 返回 None)".to_string())?;
    let p = data_dir.join("Nuphus").join("models");
    std::fs::create_dir_all(&p).map_err(|e| format!("创建模型目录失败 {}: {e}", p.display()))?;
    Ok(p)
}

/// Read-only dir path for status display (no directory creation).
fn models_dir_hint() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("NUPHUS_MODELS_DIR") {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join("Nuphus").join("models"))
}

/// A file is "present" if it exists AND meets its anti-poisoning floor.
fn file_present(dir: &Path, name: &str) -> bool {
    let files = vision_files();
    let Some(mf) = files.iter().find(|f| f.name == name) else {
        return dir.join(name).exists();
    };
    std::fs::metadata(dir.join(name))
        .map(|m| m.len() >= mf.min_size)
        .unwrap_or(false)
}

fn scan_status() -> VisionModelsStatus {
    let dir = models_dir_hint();
    let present = |name: &str| dir.as_ref().map(|d| file_present(d, name)).unwrap_or(false);
    VisionModelsStatus {
        ocr_ready: vision_files()
            .iter()
            .filter(|f| f.required)
            .all(|f| present(f.name)),
        yolo_ready: present("icon_detect.onnx"),
        missing: dir
            .as_ref()
            .map(|d| {
                vision_files()
                    .iter()
                    .filter(|f| !file_present(d, f.name))
                    .map(|f| f.name.to_string())
                    .collect()
            })
            .unwrap_or_default(),
        dir: dir.map(|d| d.display().to_string()),
        downloading: DOWNLOAD_RUNNING.load(Ordering::SeqCst),
    }
}

/// Synchronous status query (no side effects).
#[tauri::command]
pub fn vision_models_status() -> VisionModelsStatus {
    scan_status()
}

// ── Download worker ─────────────────────────────────────────────────────

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
    let dir = models_dir_for_write()?;
    tracing::info!(
        "[vision-dl] downloading vision models into {}",
        dir.display()
    );

    // No total timeout: YOLO (12 MB) over slow links exceeds any sane fixed
    // cap; connect_timeout bounds the stall case that matters in practice.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let files = vision_files();
    let count = files.len();

    for (idx, mf) in files.iter().enumerate() {
        let path = dir.join(mf.name);
        let index = idx + 1;

        // Skip files that already exist with a plausible size (resumable
        // re-run); undersized files are treated as poisoned and re-fetched.
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() >= mf.min_size {
                tracing::info!("[vision-dl] 跳过已存在: {} ({} bytes)", mf.name, meta.len());
                emit(
                    app,
                    ModelsEvent::Progress {
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
                "[vision-dl] {} 体积异常 ({} < {}), 重新下载",
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
                    ModelsEvent::Progress {
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
            // mirrors speech/download.rs.
            for attempt in 0..3u32 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                }
                tracing::info!(
                    "[vision-dl] 下载 {} ← {} (第 {} 次)",
                    mf.name,
                    url,
                    attempt + 1
                );
                match download_once(&client, url, &path, &mut on_progress) {
                    Ok(size) if size >= mf.min_size => {
                        tracing::info!("[vision-dl] 完成: {} ({} bytes)", mf.name, size);
                        downloaded = true;
                        break 'sources;
                    }
                    Ok(size) => {
                        last_err = format!(
                            "{} 体积异常 ({} bytes < 期望至少 {} bytes)",
                            mf.name, size, mf.min_size
                        );
                        tracing::warn!("[vision-dl] {last_err}");
                        let _ = std::fs::remove_file(&path);
                        break; // poisoned payload — retrying this source won't help
                    }
                    Err(e) => {
                        last_err = format!("{url}: {e}");
                        tracing::warn!("[vision-dl] 第 {} 次下载失败: {}", attempt + 1, last_err);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }

        if !downloaded {
            if mf.required {
                return Err(format!("{} 下载失败: {last_err}", mf.name));
            }
            // YOLO is optional: degrade to OCR-only, never a terminal error.
            tracing::warn!(
                "[vision-dl] {} 下载失败（可选，已跳过）: {last_err}",
                mf.name
            );
        }
    }

    Ok(())
}

/// Non-blocking entry point. Returns immediately (download runs on a named OS
/// thread); callers that want the result follow the `models:download` events.
pub fn ensure_vision_models(app: &AppHandle) -> Result<(), String> {
    let status = scan_status();
    if status.ocr_ready && status.yolo_ready {
        tracing::info!("[vision-dl] all vision models ready, skip");
        return Ok(());
    }
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        tracing::info!("[vision-dl] download already running, following global events");
        return Ok(());
    }

    let worker_app = app.clone();
    let spawn = std::thread::Builder::new()
        .name("vision-model-download".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_download(&worker_app)
            }));
            DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
            match result {
                Ok(Ok(())) => {
                    let s = scan_status();
                    tracing::info!(
                        "[vision-dl] done (ocr_ready={}, yolo_ready={})",
                        s.ocr_ready,
                        s.yolo_ready
                    );
                    emit(
                        &worker_app,
                        ModelsEvent::Done {
                            ocr_ready: s.ocr_ready,
                            yolo_ready: s.yolo_ready,
                        },
                    );
                }
                Ok(Err(e)) => {
                    tracing::warn!("[vision-dl] download failed: {e}");
                    emit(&worker_app, ModelsEvent::Error { message: e });
                }
                Err(_) => {
                    tracing::warn!("[vision-dl] download worker panicked");
                    emit(
                        &worker_app,
                        ModelsEvent::Error {
                            message: "download worker panicked (see stderr)".to_string(),
                        },
                    );
                }
            }
        });
    if let Err(e) = spawn {
        DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
        return Err(format!("failed to spawn vision download worker: {e}"));
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest must stay in sync with the exact filenames the runtime
    /// reads (src/desktop/paddle_ocr.rs L119-121, yolo_detect.rs L46) — drift
    /// here silently breaks auto-healing.
    #[test]
    fn manifest_covers_runtime_files() {
        let files = vision_files();
        let names: Vec<&str> = files.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            [
                "ch_PP-OCRv4_det.onnx",
                "ch_PP-OCRv4_rec.onnx",
                "ch_PP-OCR_keys_v1.txt",
                "icon_detect.onnx",
            ]
        );
        // OCR must be required; YOLO optional.
        assert_eq!(files.iter().filter(|f| f.required).count(), 3);
        assert_eq!(files.iter().filter(|f| !f.required).count(), 1);
        for f in &files {
            assert!(!f.urls.is_empty(), "{} has no download source", f.name);
            assert!(f.min_size > 0, "{} has no anti-poisoning floor", f.name);
            for u in &f.urls {
                assert!(u.starts_with("https://"), "non-https source: {u}");
            }
        }
    }

    /// Write target must be the same dir `resolve_models_dir` reads (step 2:
    /// data_dir/Nuphus/models), otherwise downloads land where the runtime
    /// never looks.
    #[test]
    fn write_dir_matches_read_dir() {
        let dir = models_dir_for_write().expect("data_dir resolvable on dev machine");
        assert!(
            dir.ends_with("Nuphus\\models") || dir.ends_with("Nuphus/models"),
            "unexpected dir: {}",
            dir.display()
        );
    }

    #[test]
    fn scan_status_reports_missing_in_empty_dir() {
        // Override env so the scan points at a guaranteed-empty temp dir.
        let tmp = std::env::temp_dir().join("nuphus_vision_scan_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("NUPHUS_MODELS_DIR", &tmp);
        let s = scan_status();
        assert!(!s.ocr_ready);
        assert!(!s.yolo_ready);
        assert_eq!(s.missing.len(), 4);
        assert_eq!(s.dir.as_deref(), Some(tmp.to_str().unwrap()));
        std::env::remove_var("NUPHUS_MODELS_DIR");
    }

    #[test]
    fn scan_status_ready_when_files_present() {
        let tmp = std::env::temp_dir().join("nuphus_vision_scan_ready_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        for f in &vision_files() {
            // Write a file large enough to pass the anti-poisoning floor.
            let buf = vec![0u8; (f.min_size + 1) as usize];
            std::fs::write(tmp.join(f.name), buf).unwrap();
        }
        std::env::set_var("NUPHUS_MODELS_DIR", &tmp);
        let s = scan_status();
        assert!(s.ocr_ready);
        assert!(s.yolo_ready);
        assert!(s.missing.is_empty());
        std::env::remove_var("NUPHUS_MODELS_DIR");
    }
}
