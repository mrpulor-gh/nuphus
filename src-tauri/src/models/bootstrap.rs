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
//! - Trigger: `preload_ocr` → `ensure_vision_models_blocking(&app)` — blocking.
//!   Runs the download synchronously (inside the command's spawn_blocking) so
//!   the splash stays up through first-run downloads; `invoke('preload_ocr')`
//!   doubles as the ModelsPage "retry" entry point. Terminal state via events.
//!   A concurrent caller while a download runs follows the global event stream
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
//! - A file missing from the data dir is first *adopted* from the bundled
//!   `desktop/models` shipped next to the exe (or the dev repo layout) when
//!   present ≥ `min_size` — copied locally with no splash pct and no network.
//!   Only files absent from BOTH locations hit the network (prevents the
//!   "already bundled but still background-downloading" false positive).
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

/// Bundled ("内置") vision-models directory shipped with the app: the
/// exe-adjacent `desktop/models` (release portable layout) or the dev repo's
/// `src-tauri/desktop/models` (cwd / CARGO_MANIFEST_DIR candidates). Mirrors
/// the tail of `nuphus::desktop::resolve_models_dir` but EXCLUDES the env
/// override and data_dir candidates, so callers can tell "shipped with the
/// app" apart from "downloaded into user data".
fn bundled_models_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut base = exe.parent().map(Path::to_path_buf);
        for _ in 0..=2 {
            let Some(p) = base else { break };
            let candidate = p.join("desktop").join("models");
            if candidate.exists() {
                return Some(candidate);
            }
            base = p.parent().map(Path::to_path_buf);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut bases = vec![cwd.clone()];
        if let Some(parent) = cwd.parent() {
            bases.push(parent.to_path_buf());
        }
        for b in bases {
            let candidate = b.join("src-tauri").join("desktop").join("models");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/desktop/models");
    if dev_path.exists() {
        return Some(dev_path);
    }
    None
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

/// Splash 启动时的主动状态查询：确认「是否真的需要下载模型」。
///
/// 这是 splash 的逻辑起点——不是靠「有没有收到下载进度事件」来推断，而是
/// 前端一上来就主动问一句。事件流可能因 webview 时序丢失，状态查询不会丢：
/// 全就绪 → splash 本次会话绝不亮出下载面板/「后台下载」按钮；
/// 缺模型 → 才允许亮出下载 UI（进度事件随后驱动数值/文案）。
/// 纯本地文件检查，无副作用、无网络。覆盖嵌入模型 + 视觉模型（OCR/YOLO）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SplashBootstrapStatus {
    /// true = 有模型缺失/未达标，需要下载 → 前端才允许显示下载面板与按钮
    pub needs_download: bool,
    /// 缺失项列表（供文案/排查）
    pub missing: Vec<String>,
    /// 人类可读说明
    pub text: String,
}

#[tauri::command]
pub fn splash_bootstrap_status() -> SplashBootstrapStatus {
    let vision = scan_status();
    let embed_ok = nuphus::embed::Embedder::files_ready();
    let mut missing = vision.missing.clone();
    if !embed_ok {
        missing.insert(0, "bge-small-zh（嵌入模型）".to_string());
    }
    let needs_download = !embed_ok || !vision.ocr_ready || !vision.yolo_ready;
    let text = if needs_download {
        format!("需要下载模型：{}", missing.join("、"))
    } else {
        "模型已全部就绪，无需下载".to_string()
    };
    tracing::info!(
        "[SplashBootstrapStatus] needs_download={needs_download} embed_ok={embed_ok} ocr_ready={} yolo_ready={} missing={:?} dir={:?}",
        vision.ocr_ready,
        vision.yolo_ready,
        missing,
        vision.dir
    );
    SplashBootstrapStatus {
        needs_download,
        missing,
        text,
    }
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
    let bundled_dir = bundled_models_dir();
    tracing::info!(
        "[vision-dl] downloading vision models into {} (bundled: {:?})",
        dir.display(),
        bundled_dir.as_ref().map(|p| p.display().to_string())
    );

    // No total timeout: YOLO (12 MB) over slow links exceeds any sane fixed
    // cap; connect_timeout bounds the stall case that matters in practice.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let files = vision_files();
    let count = files.len();

    // 综合进度基准：以各文件 min_size 为代理体积累加（含已存在文件），
    // 把逐文件进度折算成整体百分比推到 splash。
    let total_bytes: u64 = files.iter().map(|f| f.min_size).sum();
    let mut done_bytes: u64 = 0u64;

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
                // ⚠️ 已存在（无需下载）不发 splash pct：splash 加载条/「后台下载」按钮
                // 只在真实下载时显示——此前即使全部本地就绪也发 pct（100%），splash.js
                // 收到 pct 即亮出加载条 + 15s 后「后台下载」按钮 → 本地已下载完仍显示
                // 「后台下载」（2026-08-26 大王实测；OpenCode 只改前端未治本）。
                // 仅累计 done_bytes 供整体进度折算（真正下载时的 on_progress 用）。
                done_bytes += mf.min_size;
                continue;
            }
            tracing::warn!(
                "[vision-dl] {} 体积异常 ({} < {}), 重新下载",
                mf.name,
                meta.len(),
                mf.min_size
            );
        }

        // AppData 缺失/不足 → 先采用随应用分发的内置模型（本地复制，不走网络）。
        // 不发 splash 数值 pct：复制是秒级本地操作、不是下载——非下载动作绝不
        // 驱动加载条与「后台下载」按钮（d99fe72 同型教训）。复制失败降级为下载。
        if let Some(bundled) = bundled_dir.as_ref() {
            if bundled != &dir {
                let src = bundled.join(mf.name);
                if let Ok(bmeta) = std::fs::metadata(&src) {
                    if bmeta.len() >= mf.min_size {
                        tracing::info!(
                            "[vision-dl] 采用内置模型: {} ← {}",
                            mf.name,
                            bundled.display()
                        );
                        crate::splash::emit_splash_progress(app, None, "正在启用内置模型…");
                        match std::fs::copy(&src, &path) {
                            Ok(n) if n >= mf.min_size => {
                                emit(
                                    app,
                                    ModelsEvent::Progress {
                                        file: mf.name.to_string(),
                                        downloaded: n,
                                        total: n,
                                        index,
                                        count,
                                    },
                                );
                                done_bytes += mf.min_size;
                                continue;
                            }
                            Ok(n) => {
                                tracing::warn!(
                                    "[vision-dl] 内置副本体积异常 ({} < {}), 改走下载",
                                    n,
                                    mf.min_size
                                );
                                let _ = std::fs::remove_file(&path);
                            }
                            Err(e) => {
                                tracing::warn!("[vision-dl] 复制内置模型失败: {e}, 改走下载");
                            }
                        }
                    }
                }
            }
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
                // 该文件折算体积：total>0 按比例，否则按 min_size 封顶
                // （文件完成后在循环尾部统一入账 done_bytes）。
                let contrib = if total > 0 {
                    (mf.min_size as u128 * downloaded as u128 / total as u128)
                        .min(mf.min_size as u128) as u64
                } else {
                    downloaded.min(mf.min_size)
                };
                emit_splash_pct(
                    app,
                    done_bytes + contrib,
                    total_bytes,
                    &format!(
                        "正在下载视觉模型… {} {}",
                        mf.name,
                        file_pct(downloaded, total)
                    ),
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

        done_bytes += mf.min_size;
        emit_splash_pct(
            app,
            done_bytes,
            total_bytes,
            &format!("视觉模型… {} 就绪", mf.name),
        );
    }

    Ok(())
}

/// 整体进度百分比（0..=100），基于代理总体积；total 为 0 时视为完成。
fn splash_pct(done: u64, total: u64) -> u8 {
    if total == 0 {
        100
    } else {
        ((done as f64 / total as f64) * 100.0).min(100.0).round() as u8
    }
}

/// 单文件进度文案：total 已知显示百分比，未知（无 Content-Length）显示已下载量。
fn file_pct(downloaded: u64, total: u64) -> String {
    if total > 0 {
        let p = ((downloaded as f64 / total as f64) * 100.0)
            .min(100.0)
            .round() as u8;
        format!("{p}%")
    } else {
        format!("{} bytes", downloaded)
    }
}

/// 折算整体进度并推送 `splash:progress`（splash 已关闭时静默）。
fn emit_splash_pct(app: &AppHandle, done: u64, total: u64, text: &str) {
    let pct = splash_pct(done, total);
    crate::splash::emit_splash_progress(app, Some(pct), text);
}

/// Blocking entry point (first-run splash). Runs the download synchronously on
/// the calling thread and returns only when all vision models are ready or a
/// required download terminally failed — this is what keeps the splash window
/// alive through the first-run download. Progress is pushed to `splash:progress`
/// (from inside `run_download`) and terminal state to `models:download`.
///
/// 调用方必须放在 spawn_blocking / 独立线程，避免阻塞主线程事件循环导致
/// 进度事件无法投递（`run_download` 内部使用 reqwest::blocking）。
pub fn ensure_vision_models_blocking(app: &AppHandle) -> Result<(), String> {
    let status = scan_status();
    // 诊断：目录视图一行汇总——排障「为何要下载 / 为何误报后台下载」的关键证据
    //（splash 判定只看 data_dir；bundled 是随应用分发的 desktop/models）。
    if let Ok(env_dir) = std::env::var("NUPHUS_MODELS_DIR") {
        tracing::warn!("[vision-dl] NUPHUS_MODELS_DIR 覆盖生效: {env_dir}");
    }
    tracing::info!(
        "[vision-dl] scan: dir={:?} ocr={} yolo={} missing={:?} bundled={:?}",
        models_dir_hint().map(|p| p.display().to_string()),
        status.ocr_ready,
        status.yolo_ready,
        status.missing,
        bundled_models_dir().map(|p| p.display().to_string()),
    );
    if status.ocr_ready && status.yolo_ready {
        tracing::info!("[vision-dl] all vision models ready, skip (blocking)");
        return Ok(());
    }
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        tracing::info!("[vision-dl] download already running, following global events");
        return Ok(());
    }

    // catch_unwind：下载线程 panic 仅表现为 Err，不污染 DOWNLOAD_RUNNING 状态。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_download(app)));
    DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
    match result {
        Ok(Ok(())) => {
            let s = scan_status();
            tracing::info!(
                "[vision-dl] done (blocking) ocr_ready={}, yolo_ready={}",
                s.ocr_ready,
                s.yolo_ready
            );
            emit(
                app,
                ModelsEvent::Done {
                    ocr_ready: s.ocr_ready,
                    yolo_ready: s.yolo_ready,
                },
            );
            Ok(())
        }
        Ok(Err(e)) => {
            tracing::warn!("[vision-dl] download failed: {e}");
            emit(app, ModelsEvent::Error { message: e.clone() });
            Err(e)
        }
        Err(_) => {
            tracing::warn!("[vision-dl] download worker panicked");
            emit(
                app,
                ModelsEvent::Error {
                    message: "download worker panicked (see stderr)".to_string(),
                },
            );
            Err("download worker panicked (see stderr)".to_string())
        }
    }
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
