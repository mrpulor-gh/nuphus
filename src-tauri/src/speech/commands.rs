//! Tauri command surface for speech-to-text.
//!
//! Contract (consumed by frontend VoiceButton / ChatInputBar):
//! - Commands: `stt_start` / `stt_stop` / `stt_cancel` / `stt_status`,
//!   plus hidden debug command `stt_recognize_file` (regression pipeline).
//! - Events: `stt:partial` { text } — live in-progress utterance ("" clears);
//!   `stt:final` { text, start_ms, end_ms } — committed utterance, may arrive
//!   MULTIPLE times during recording (VAD closes segments on pauses); never
//!   implies session end. `stt:ready` () — mic is actually capturing (model/VAD
//!   load and mic open happen inside the worker AFTER stt_start returns); the
//!   frontend enters its recording phase only on this event so no leading
//!   speech is lost. `stt:done` { reason } — session fully ended, exactly
//!   once on every exit path ("stop" | "timeout" | "cancel" | "error");
//!   `stt:error` { message, recoverable }.
//! - Engine routing (cloud-first, product decision 2026-07-27): when
//!   `capabilities.stt` resolves to a provider+model (cloud.rs), the session
//!   records into the same in-memory buffer but uploads one wav on stop and
//!   emits the text as a SINGLE stt:final — no partials, no VAD segmenting
//!   (this contract already permits both; the frontend needs no changes).
//!   Otherwise the local sherpa-onnx path below runs. Cloud failure emits
//!   stt:error { recoverable: true } — never a silent fallback to local.
//! - State machine: Idle → Recording → (stop / timeout) Decoding → Idle.
//!   `stt_cancel` returns to Idle without emitting finals.
//! - Recording has a hard duration cap (MAX_SESSION_DURATION); reaching it
//!   auto-stops the session like a user stop (tail decode + done). In cloud
//!   mode the same cap doubles as API cost control.
//! - Audio lives in memory only (mpsc → history Vec); no temp wav files.
//!   Session teardown: mic dropped before tail decode, slot reset, done sent.
//! - Degradation: no mic → stt_status.available=false ("no_microphone");
//!   missing local models degrade ONLY when no cloud engine is configured
//!   ("model_missing: ..."); commands return Err, nothing panics (worker is
//!   catch_unwind-guarded).

use super::cloud::{self, CloudSttConfig};
use super::engine::{self, RecognizerCache, SAMPLE_RATE};
use super::mic::{self, MicCapture, Resampler};
use super::pipeline::{FinalSegment, Pipeline, Sink};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// sense-voice inverse text normalization (数字/标点) — product default on
/// (report-live.md §3.3: 语义等价可接受, switchable per session later).
/// pub(crate)：main.rs 启动预热需与 stt session 使用同一参数。
pub(crate) const USE_ITN: bool = true;

/// Hard cap on one recording session (product requirement: 时长限制).
/// Reaching it auto-stops like a user stop. VAD closes segments ≤20s, so
/// finals keep streaming during the cap window; 120s bounds mic occupation
/// and tail-decode latency while covering normal chat dictation.
const MAX_SESSION_DURATION: Duration = Duration::from_secs(120);

/// How a capture session ended; surfaced to the UI as stt:done { reason }.
enum SessionEnd {
    /// User clicked stop (tail decoded).
    Stopped,
    /// MAX_SESSION_DURATION reached (tail decoded).
    TimedOut,
    /// Cancelled — no finals by design.
    Cancelled,
}

impl SessionEnd {
    fn reason(&self) -> &'static str {
        match self {
            SessionEnd::Stopped => "stop",
            SessionEnd::TimedOut => "timeout",
            SessionEnd::Cancelled => "cancel",
        }
    }
}

// ── Session state ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Recording,
    Decoding,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Recording => "recording",
            Phase::Decoding => "decoding",
        }
    }
}

pub struct SessionSlot {
    phase: Phase,
    stop: Option<Arc<AtomicBool>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Default for SessionSlot {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            stop: None,
            cancel: None,
        }
    }
}

/// STT subsystem state, owned by `AppState`. Cheap to construct — the
/// recognizer loads lazily on first use.
pub struct SpeechState {
    pub cache: Arc<RecognizerCache>,
    pub session: Arc<Mutex<SessionSlot>>,
}

impl Default for SpeechState {
    fn default() -> Self {
        Self {
            cache: Arc::new(RecognizerCache::default()),
            session: Arc::new(Mutex::new(SessionSlot::default())),
        }
    }
}

fn lock_session(
    session: &Mutex<SessionSlot>,
) -> Result<std::sync::MutexGuard<'_, SessionSlot>, String> {
    session
        .lock()
        .map_err(|e| format!("stt session lock poisoned: {e}"))
}

// ── Event payloads ────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct PartialPayload {
    text: String,
}

#[derive(Serialize, Clone)]
struct FinalPayload {
    text: String,
    start_ms: i64,
    end_ms: i64,
}

#[derive(Serialize, Clone)]
struct ErrorPayload {
    message: String,
    recoverable: bool,
}

#[derive(Serialize, Clone)]
struct DonePayload {
    reason: String,
}

struct EventSink {
    app: AppHandle,
    /// Finals emitted this session; used to guarantee a completion signal.
    finals: usize,
}

impl Sink for EventSink {
    fn on_partial(&mut self, text: &str) {
        let _ = self.app.emit(
            "stt:partial",
            PartialPayload {
                text: text.to_string(),
            },
        );
    }

    fn on_final(&mut self, seg: FinalSegment) {
        self.finals += 1;
        let _ = self.app.emit(
            "stt:final",
            FinalPayload {
                text: seg.text,
                start_ms: seg.start_ms,
                end_ms: seg.end_ms,
            },
        );
    }
}

fn emit_error(app: &AppHandle, message: String, recoverable: bool) {
    let _ = app.emit(
        "stt:error",
        ErrorPayload {
            message,
            recoverable,
        },
    );
}

// ── Status ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SttStatusResult {
    pub available: bool,
    /// Machine-readable reason when unavailable: "no_microphone" | "model_missing: <file>".
    pub reason: Option<String>,
    pub phase: String,
    pub model_dir: Option<String>,
    pub version: Option<String>,
    /// Engine that would serve the next session: "cloud" when
    /// capabilities.stt resolves (cloud-first routing), else "local".
    pub engine: String,
    /// True when capabilities.stt resolves to a provider+model — voice input
    /// then stays available even without the local model files.
    pub cloud_configured: bool,
}

#[tauri::command]
pub fn stt_status(state: State<'_, AppState>) -> SttStatusResult {
    let phase = lock_session(&state.speech.session)
        .map(|s| s.phase.as_str())
        .unwrap_or("idle");

    let model = engine::resolve_stt_paths();
    let mic_ok = mic::mic_available();
    let cloud_configured = cloud::resolve_cloud_config().is_some();

    let (available, reason) = match (&model, mic_ok, cloud_configured) {
        // Cloud configured: local model files optional; mic still required.
        (_, true, true) => (true, None),
        (_, false, true) => (false, Some("no_microphone".to_string())),
        // No cloud: original local-only semantics.
        (Ok(_), true, false) => (true, None),
        (Err(e), _, false) => (false, Some(e.to_string())),
        (Ok(_), false, false) => (false, Some("no_microphone".to_string())),
    };

    SttStatusResult {
        available,
        reason,
        phase: phase.to_string(),
        model_dir: model.ok().map(|p| p.dir.display().to_string()),
        version: engine::sherpa_version(),
        engine: if cloud_configured { "cloud" } else { "local" }.to_string(),
        cloud_configured,
    }
}

// ── Session control ───────────────────────────────────────────────────

#[tauri::command]
pub fn stt_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Availability gate first — cheap filesystem + device probes.
    // Cloud-first routing: capabilities.stt resolvable → cloud engine (local
    // model files not required); otherwise local sherpa-onnx.
    let cloud = cloud::resolve_cloud_config();
    if cloud.is_none() {
        engine::resolve_stt_paths().map_err(|e| e.to_string())?;
    }
    if !mic::mic_available() {
        return Err("no_microphone".to_string());
    }

    let session = Arc::clone(&state.speech.session);
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = lock_session(&session)?;
        if slot.phase != Phase::Idle {
            return Err(format!("stt_busy: session is {}", slot.phase.as_str()));
        }
        slot.phase = Phase::Recording;
        slot.stop = Some(Arc::clone(&stop));
        slot.cancel = Some(Arc::clone(&cancel));
    }

    let cache = Arc::clone(&state.speech.cache);
    let worker_app = app.clone();
    std::thread::Builder::new()
        .name("stt-session".to_string())
        .spawn(move || run_session(worker_app, cache, session, stop, cancel, cloud))
        .map_err(|e| format!("failed to spawn stt worker: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn stt_stop(state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = lock_session(&state.speech.session)?;
    match slot.phase {
        Phase::Recording => {
            if let Some(stop) = &slot.stop {
                stop.store(true, Ordering::SeqCst);
            }
            slot.phase = Phase::Decoding;
            Ok(())
        }
        Phase::Idle => Err("stt_idle: no active session".to_string()),
        Phase::Decoding => Ok(()), // idempotent — already finishing
    }
}

#[tauri::command]
pub fn stt_cancel(state: State<'_, AppState>) -> Result<(), String> {
    let slot = lock_session(&state.speech.session)?;
    if let Some(cancel) = &slot.cancel {
        cancel.store(true, Ordering::SeqCst);
    }
    if let Some(stop) = &slot.stop {
        stop.store(true, Ordering::SeqCst);
    }
    // Worker resets the slot when it observes the flags (≤100ms poll).
    Ok(())
}

/// Worker thread body: mic → resample → engine (cloud upload / local VAD
/// pipeline) → events. Never panics outward; all failures surface as
/// stt:error + phase reset.
fn run_session(
    app: AppHandle,
    cache: Arc<RecognizerCache>,
    session: Arc<Mutex<SessionSlot>>,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    cloud: Option<CloudSttConfig>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session_body(&app, &cache, &session, &stop, &cancel, cloud.as_ref())
    }));
    let end = match result {
        Ok(Ok(end)) => Some(end),
        Ok(Err(e)) => {
            tracing::warn!("[stt] session failed: {e}");
            emit_error(&app, e, true);
            None
        }
        Err(_) => {
            tracing::warn!("[stt] session failed: worker panicked");
            emit_error(&app, "stt worker panicked (see stderr)".to_string(), true);
            None
        }
    };
    // Reset slot for the next session (清场：相位/标志归还，mic 已在
    // session_body 尾部解码前 drop；音频全程驻留内存，无临时文件).
    if let Ok(mut slot) = session.lock() {
        slot.phase = Phase::Idle;
        slot.stop = None;
        slot.cancel = None;
    }
    // Completion signal: exactly once on EVERY exit path — the frontend
    // leaves recording/decoding only on stt:done (or stt:error).
    let reason = end.as_ref().map(SessionEnd::reason).unwrap_or("error");
    let _ = app.emit(
        "stt:done",
        DonePayload {
            reason: reason.to_string(),
        },
    );
    tracing::info!("[stt] session ended (reason={reason})");
}

fn session_body(
    app: &AppHandle,
    cache: &RecognizerCache,
    session: &Mutex<SessionSlot>,
    stop: &AtomicBool,
    cancel: &AtomicBool,
    cloud: Option<&CloudSttConfig>,
) -> Result<SessionEnd, String> {
    if let Some(cloud) = cloud {
        return cloud_session_body(app, cloud, session, stop, cancel);
    }
    let recognizer = cache.get_or_load(USE_ITN)?;
    let paths = engine::resolve_stt_paths().map_err(|e| e.to_string())?;
    let vad = engine::Vad::new(&paths)?;

    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let mic = MicCapture::start(tx)?;
    // Mic is live only NOW (model + VAD load above can take seconds on cold
    // start): signal the frontend to enter its recording phase from this point,
    // so the user speaks only when audio is actually being captured.
    let _ = app.emit("stt:ready", ());
    let mut resampler = Resampler::new(mic.sample_rate(), SAMPLE_RATE as u32);
    let mut pipeline = Pipeline::new(vad, recognizer);
    let mut sink = EventSink {
        app: app.clone(),
        finals: 0,
    };
    let mut resampled = Vec::new();
    let started = std::time::Instant::now();
    let mut timed_out = false;

    loop {
        if cancel.load(Ordering::SeqCst) {
            pipeline.abort();
            break;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if started.elapsed() >= MAX_SESSION_DURATION {
            // 时长上限：等同用户点停止，走正常尾部解码
            timed_out = true;
            tracing::info!("[stt] max session duration reached, auto-stop");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                resampled.clear();
                resampler.process(&chunk, &mut resampled);
                pipeline.feed(&resampled, &mut sink);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Capture stops before tail decoding (frees the mic for other apps).
    drop(mic);

    if cancel.load(Ordering::SeqCst) {
        // Clear any partial shown in the UI; no finals.
        sink.on_partial("");
        Ok(SessionEnd::Cancelled)
    } else {
        if let Ok(mut slot) = session.lock() {
            if slot.phase == Phase::Recording {
                slot.phase = Phase::Decoding;
            }
        }
        pipeline.finish(&mut sink);
        // Completion guarantee: sessions with zero recognized speech (silence,
        // stop during model/VAD load, VAD miss) produce no finals, and the
        // frontend exits its decoding phase only on stt:final / stt:error.
        // Emit an empty final as the end-of-session signal (UI ignores empty
        // text but resets to idle).
        if sink.finals == 0 {
            sink.on_final(FinalSegment {
                text: String::new(),
                start_ms: 0,
                end_ms: 0,
            });
        }
        // Signal the UI to clear the partial ghost after tail decoding.
        sink.on_partial("");
        Ok(if timed_out {
            SessionEnd::TimedOut
        } else {
            SessionEnd::Stopped
        })
    }
}

/// Cloud session body: mic → in-memory 16kHz buffer (no VAD, no partials) →
/// on stop, one wav upload via the OpenAI-compatible transcriptions API; the
/// recognized text arrives as a single stt:final. Failure surfaces as
/// stt:error (recoverable) — never a silent fallback to the local engine.
fn cloud_session_body(
    app: &AppHandle,
    cloud: &CloudSttConfig,
    session: &Mutex<SessionSlot>,
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<SessionEnd, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let mic = MicCapture::start(tx)?;
    // Same contract as the local path: stt:ready marks the mic-live moment.
    let _ = app.emit("stt:ready", ());
    let mut resampler = Resampler::new(mic.sample_rate(), SAMPLE_RATE as u32);
    let mut history: Vec<f32> = Vec::new();
    let mut resampled = Vec::new();
    let started = std::time::Instant::now();
    let mut timed_out = false;

    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if started.elapsed() >= MAX_SESSION_DURATION {
            // 时长上限：等同用户点停止；云端模式下同时是 API 成本控制
            timed_out = true;
            tracing::info!("[stt] max session duration reached, auto-stop (cloud)");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                resampled.clear();
                resampler.process(&chunk, &mut resampled);
                history.extend_from_slice(&resampled);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Capture stops before the upload (frees the mic for other apps).
    drop(mic);

    if cancel.load(Ordering::SeqCst) {
        // Cloud mode never emitted partials — nothing to clear, no finals.
        return Ok(SessionEnd::Cancelled);
    }

    // Mirror the local path: mark Decoding while the upload is in flight
    // (stt_stop is idempotent in this phase; auto-stop never called it).
    if let Ok(mut slot) = session.lock() {
        if slot.phase == Phase::Recording {
            slot.phase = Phase::Decoding;
        }
    }

    let end = if timed_out {
        SessionEnd::TimedOut
    } else {
        SessionEnd::Stopped
    };
    if history.is_empty() {
        return Ok(end);
    }

    let duration_ms = history.len() as i64 * 1000 / SAMPLE_RATE as i64;
    let wav_bytes = super::wav::encode_wav_pcm16(&history, SAMPLE_RATE as u32);
    let text = cloud::transcribe(cloud, wav_bytes)?.trim().to_string();
    // Completion guarantee mirrors the local path: exactly one final per
    // cloud session; empty text when nothing was recognized (UI ignores the
    // text but uses the event to leave the decoding phase).
    let _ = app.emit(
        "stt:final",
        FinalPayload {
            text,
            start_ms: 0,
            end_ms: duration_ms,
        },
    );
    Ok(end)
}

// ── Hidden debug command: file regression ─────────────────────────────

/// Recognize a wav file through the exact production pipeline
/// (same VAD params, pre-roll, dict correction). Output format matches
/// tools/stt-proto/eval_live.py expectations: "start -- end: text" per line.
#[tauri::command]
pub async fn stt_recognize_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let cache = Arc::clone(&state.speech.cache);
    tauri::async_runtime::spawn_blocking(move || recognize_file_impl(&cache, &path))
        .await
        .map_err(|e| format!("stt_recognize_file task failed: {e}"))?
}

/// Format segments as the legacy `stt_recognize_file` text lines
/// ("start.sec -- end.sec: text") — output contract unchanged.
fn format_segments_as_lines(segments: Vec<FinalSegment>) -> String {
    segments
        .into_iter()
        .map(|seg| {
            format!(
                "{:.3} -- {:.3}: {}",
                seg.start_ms as f64 / 1000.0,
                seg.end_ms as f64 / 1000.0,
                seg.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn recognize_file_impl(cache: &RecognizerCache, path: &str) -> Result<String, String> {
    Ok(format_segments_as_lines(transcribe_wav_segments(
        cache,
        std::path::Path::new(path),
    )?))
}

struct SegmentSink {
    segs: Vec<FinalSegment>,
}

impl Sink for SegmentSink {
    fn on_partial(&mut self, _text: &str) {}
    fn on_final(&mut self, seg: FinalSegment) {
        self.segs.push(seg);
    }
}

/// Structured variant of `recognize_file_impl`: same decode chain (wav →
/// resample 16k → VAD → Pipeline), but returns the `FinalSegment`s with
/// timestamps instead of pre-formatted text lines. Added for the video
/// subtitle pipeline (src-tauri/src/video/) — `stt_recognize_file`'s output
/// contract is unchanged.
pub fn transcribe_wav_segments(
    cache: &RecognizerCache,
    path: &std::path::Path,
) -> Result<Vec<FinalSegment>, String> {
    let wav = super::wav::read_wav(path)?;
    let recognizer = cache.get_or_load(USE_ITN)?;
    let paths = engine::resolve_stt_paths().map_err(|e| e.to_string())?;
    let vad = engine::Vad::new(&paths)?;

    // Resample to 16k if needed, then feed in VAD-window chunks (512 samples)
    // mirroring the validated CLI chain.
    let mut resampler = Resampler::new(wav.sample_rate, SAMPLE_RATE as u32);
    let mut audio = Vec::with_capacity(wav.samples.len());
    resampler.process(&wav.samples, &mut audio);

    let mut pipeline = Pipeline::new(vad, recognizer);
    let mut sink = SegmentSink { segs: Vec::new() };
    for chunk in audio.chunks(512) {
        pipeline.feed(chunk, &mut sink);
    }
    pipeline.finish(&mut sink);
    Ok(sink.segs)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Live regression: tools/stt-proto/live.wav through the production
    /// pipeline. Asset-gated (skips silently when missing). Run explicitly:
    ///   cargo test -p nuphus-desktop stt_regress -- --ignored --nocapture
    /// Then score: python tools/stt-proto/eval_live.py live-result-integration.txt -integration
    #[test]
    #[ignore]
    fn stt_regress_live_wav() {
        let proto = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools")
            .join("stt-proto");
        let wav_path = proto.join("live.wav");
        if !wav_path.is_file() {
            eprintln!("[stt_regress] live.wav not found, skipping");
            return;
        }
        let cache = RecognizerCache::default();
        let out = recognize_file_impl(&cache, &wav_path.to_string_lossy())
            .expect("recognize live.wav failed");
        assert!(!out.trim().is_empty(), "pipeline produced no segments");

        let out_path = proto.join("live-result-integration.txt");
        std::fs::write(&out_path, &out).expect("write regression result failed");
        eprintln!("[stt_regress] wrote {}", out_path.display());
        eprintln!("{out}");
    }
}
