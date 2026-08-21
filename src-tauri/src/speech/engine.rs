//! Safe-ish wrappers around the sherpa-onnx FFI layer.
//!
//! Owns native handles (recognizer / VAD), resolves model paths, and
//! exposes the lazy shared recognizer cache used by both the mic session
//! worker and the `stt_recognize_file` debug command.

use super::ffi;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Fixed sample rate of the whole pipeline (model & VAD requirement).
pub const SAMPLE_RATE: usize = 16_000;

/// Decode threads — mirrors the validated prototype baseline (RTF 0.030 on i5-4590).
pub const NUM_THREADS: i32 = 4;

/// sense-voice language tag. Fixed to zh per product decision (avoid auto → yue misdetect).
pub const SENSE_VOICE_LANGUAGE: &str = "zh";

// ── Path resolution ───────────────────────────────────────────────────

/// Model files required under the STT model dir.
#[derive(Debug)]
pub struct SttPaths {
    pub model: PathBuf,
    pub tokens: PathBuf,
    pub vad: PathBuf,
    pub dir: PathBuf,
}

/// Why STT is unavailable on this machine.
#[derive(Debug, Clone)]
pub enum SttUnavailable {
    ModelMissing { dir: PathBuf, file: &'static str },
}

impl std::fmt::Display for SttUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SttUnavailable::ModelMissing { dir, file } => {
                write!(f, "model_missing: {} not found in {}", file, dir.display())
            }
        }
    }
}

/// Candidate STT model directories in priority order.
/// Mirrors `nuphus::desktop::resolve_models_dir()` conventions:
/// env override → user data dir → exe-relative → cwd (dev).
fn stt_dir_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(dir) = std::env::var("NUPHUS_STT_MODELS_DIR") {
        v.push(PathBuf::from(dir));
    }
    if let Some(data_dir) = dirs::data_dir() {
        v.push(data_dir.join("Nuphus").join("models").join("stt"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            v.push(p.join("models").join("stt"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join("models").join("stt"));
    }
    v
}

fn check_files(dir: &Path) -> Result<SttPaths, SttUnavailable> {
    let model = dir.join("model.int8.onnx");
    let tokens = dir.join("tokens.txt");
    let vad = dir.join("silero_vad.onnx");
    for (p, name) in [
        (&model, "model.int8.onnx"),
        (&tokens, "tokens.txt"),
        (&vad, "silero_vad.onnx"),
    ] {
        if !p.is_file() {
            return Err(SttUnavailable::ModelMissing {
                dir: dir.to_path_buf(),
                file: name,
            });
        }
    }
    Ok(SttPaths {
        model,
        tokens,
        vad,
        dir: dir.to_path_buf(),
    })
}

/// Resolve the STT model directory, or report the first missing file.
pub fn resolve_stt_paths() -> Result<SttPaths, SttUnavailable> {
    let candidates = stt_dir_candidates();
    let mut last_err = None;
    for dir in &candidates {
        if !dir.is_dir() {
            continue;
        }
        match check_files(dir) {
            Ok(paths) => return Ok(paths),
            Err(e) => last_err = Some(e),
        }
    }
    // No candidate had all files. Report against the most specific dir we saw,
    // else against the primary (data-dir) location.
    if let Some(e) = last_err {
        Err(e)
    } else {
        let primary = candidates
            .get(if std::env::var("NUPHUS_STT_MODELS_DIR").is_ok() {
                0
            } else {
                candidates.len().min(1)
            })
            .cloned()
            .unwrap_or_else(|| PathBuf::from("models/stt"));
        Err(SttUnavailable::ModelMissing {
            dir: primary,
            file: "model.int8.onnx",
        })
    }
}

// ── Recognizer ────────────────────────────────────────────────────────

/// Owning wrapper for the offline recognizer handle.
///
/// Send+Sync safety: sherpa-onnx recognizers support concurrent decoding of
/// independent streams (see SherpaOnnxDecodeMultipleOfflineStreams); creation
/// and destruction are serialized through the global cache mutex.
pub struct Recognizer {
    ptr: *const ffi::SherpaOnnxOfflineRecognizer,
}

unsafe impl Send for Recognizer {}
unsafe impl Sync for Recognizer {}

impl Recognizer {
    /// Create a recognizer for the FunASR-Nano sense-voice int8 model.
    ///
    /// Config mirrors the validated prototype CLI chain
    /// (sherpa-onnx-vad-with-offline-asr): greedy_search, feature_dim=80,
    /// provider=cpu, num_threads=4. Product overrides per report-live.md:
    /// language fixed "zh", use_itn configurable (default on).
    pub fn new(paths: &SttPaths, use_itn: bool) -> Result<Self, String> {
        let model = cstring_path(&paths.model)?;
        let tokens = cstring_path(&paths.tokens)?;
        let language = CString::new(SENSE_VOICE_LANGUAGE).expect("static str has no NUL");
        let provider = CString::new("cpu").expect("static str has no NUL");
        let decoding = CString::new("greedy_search").expect("static str has no NUL");

        let mut config: ffi::SherpaOnnxOfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = SAMPLE_RATE as i32;
        config.feat_config.feature_dim = 80;
        config.model_config.sense_voice.model = model.as_ptr();
        config.model_config.sense_voice.language = language.as_ptr();
        config.model_config.sense_voice.use_itn = if use_itn { 1 } else { 0 };
        config.model_config.tokens = tokens.as_ptr();
        config.model_config.num_threads = NUM_THREADS;
        config.model_config.provider = provider.as_ptr();
        config.decoding_method = decoding.as_ptr();

        let ptr = unsafe { ffi::SherpaOnnxCreateOfflineRecognizer(&config) };
        if ptr.is_null() {
            return Err(format!(
                "SherpaOnnxCreateOfflineRecognizer returned null (model: {})",
                paths.model.display()
            ));
        }
        tracing::info!(
            "[stt] recognizer loaded: {} (threads={}, itn={}, lang={})",
            paths.model.display(),
            NUM_THREADS,
            use_itn,
            SENSE_VOICE_LANGUAGE
        );
        Ok(Self { ptr })
    }

    /// Decode one utterance (mono f32 samples at `SAMPLE_RATE`) to text.
    /// Returns empty string for empty input / empty result.
    pub fn recognize(&self, samples: &[f32]) -> String {
        if samples.is_empty() {
            return String::new();
        }
        unsafe {
            let stream = ffi::SherpaOnnxCreateOfflineStream(self.ptr);
            if stream.is_null() {
                tracing::warn!("[stt] SherpaOnnxCreateOfflineStream returned null");
                return String::new();
            }
            ffi::SherpaOnnxAcceptWaveformOffline(
                stream,
                SAMPLE_RATE as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
            ffi::SherpaOnnxDecodeOfflineStream(self.ptr, stream);
            let result = ffi::SherpaOnnxGetOfflineStreamResult(stream);
            let text = if result.is_null() || (*result).text.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*result).text)
                    .to_string_lossy()
                    .into_owned()
            };
            if !result.is_null() {
                ffi::SherpaOnnxDestroyOfflineRecognizerResult(result);
            }
            ffi::SherpaOnnxDestroyOfflineStream(stream);
            text
        }
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        unsafe { ffi::SherpaOnnxDestroyOfflineRecognizer(self.ptr) }
    }
}

/// Lazy global cache: model load is ~1-2s / 250MB, share one recognizer
/// across mic sessions and file regression runs.
pub struct RecognizerCache {
    inner: Mutex<Option<Arc<Recognizer>>>,
}

impl Default for RecognizerCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

impl RecognizerCache {
    pub fn get_or_load(&self, use_itn: bool) -> Result<Arc<Recognizer>, String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| format!("recognizer cache lock poisoned: {e}"))?;
        if let Some(rec) = guard.as_ref() {
            return Ok(Arc::clone(rec));
        }
        let paths = resolve_stt_paths().map_err(|e| e.to_string())?;
        let rec = Arc::new(Recognizer::new(&paths, use_itn)?);
        *guard = Some(Arc::clone(&rec));
        Ok(rec)
    }
}

// ── VAD ───────────────────────────────────────────────────────────────

/// Owning wrapper for the VAD handle. Stateful — one instance per session,
/// confined to a single thread (worker or test thread).
pub struct Vad {
    ptr: *const ffi::SherpaOnnxVoiceActivityDetector,
}

unsafe impl Send for Vad {}

/// A completed speech segment copied out of the VAD.
pub struct Segment {
    /// Absolute start index (input samples since VAD creation).
    pub start: usize,
    pub samples: Vec<f32>,
}

impl Vad {
    /// silero-vad with the exact parameters of the validated prototype chain.
    pub fn new(paths: &SttPaths) -> Result<Self, String> {
        let model = cstring_path(&paths.vad)?;
        let provider = CString::new("cpu").expect("static str has no NUL");

        let mut config: ffi::SherpaOnnxVadModelConfig = unsafe { std::mem::zeroed() };
        config.silero_vad.model = model.as_ptr();
        config.silero_vad.threshold = 0.5;
        config.silero_vad.min_silence_duration = 0.4;
        config.silero_vad.min_speech_duration = 0.25;
        config.silero_vad.max_speech_duration = 6.0;
        config.silero_vad.window_size = 512;
        config.sample_rate = SAMPLE_RATE as i32;
        config.num_threads = 1;
        config.provider = provider.as_ptr();

        // 60s internal buffer: max_speech_duration (20s) + pre-roll + margin.
        let ptr = unsafe { ffi::SherpaOnnxCreateVoiceActivityDetector(&config, 60.0) };
        if ptr.is_null() {
            return Err(format!(
                "SherpaOnnxCreateVoiceActivityDetector returned null (vad: {})",
                paths.vad.display()
            ));
        }
        Ok(Self { ptr })
    }

    pub fn accept_waveform(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        unsafe {
            ffi::SherpaOnnxVoiceActivityDetectorAcceptWaveform(
                self.ptr,
                samples.as_ptr(),
                samples.len() as i32,
            )
        }
    }

    pub fn is_empty(&self) -> bool {
        unsafe { ffi::SherpaOnnxVoiceActivityDetectorEmpty(self.ptr) != 0 }
    }

    pub fn detected(&self) -> bool {
        unsafe { ffi::SherpaOnnxVoiceActivityDetectorDetected(self.ptr) != 0 }
    }

    /// Take the front completed segment (copies samples out), or None.
    pub fn take_front(&mut self) -> Option<Segment> {
        unsafe {
            let seg = ffi::SherpaOnnxVoiceActivityDetectorFront(self.ptr);
            if seg.is_null() {
                return None;
            }
            let out = Segment {
                start: (*seg).start.max(0) as usize,
                samples: if (*seg).samples.is_null() || (*seg).n <= 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts((*seg).samples, (*seg).n as usize).to_vec()
                },
            };
            ffi::SherpaOnnxDestroySpeechSegment(seg);
            ffi::SherpaOnnxVoiceActivityDetectorPop(self.ptr);
            Some(out)
        }
    }

    /// Force-emit the buffered tail as a final segment (end of input).
    pub fn flush(&mut self) {
        unsafe { ffi::SherpaOnnxVoiceActivityDetectorFlush(self.ptr) }
    }

    pub fn clear(&mut self) {
        unsafe { ffi::SherpaOnnxVoiceActivityDetectorClear(self.ptr) }
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        unsafe { ffi::SherpaOnnxDestroyVoiceActivityDetector(self.ptr) }
    }
}

// ── Misc ──────────────────────────────────────────────────────────────

pub fn sherpa_version() -> Option<String> {
    unsafe {
        let p = ffi::SherpaOnnxGetVersionStr();
        if p.is_null() {
            None
        } else {
            Some(CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }
}

fn cstring_path(p: &Path) -> Result<CString, String> {
    let s = p
        .to_str()
        .ok_or_else(|| format!("non-UTF8 path: {}", p.display()))?;
    CString::new(s).map_err(|_| format!("path contains NUL: {}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Degradation path: missing model files → structured error, no panic.
    #[test]
    fn check_files_reports_missing() {
        let dir = std::env::temp_dir().join("nuphus_stt_missing_test");
        std::fs::create_dir_all(&dir).unwrap();
        let err = check_files(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("model_missing:"), "unexpected: {msg}");
        assert!(msg.contains("model.int8.onnx"), "unexpected: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 降级验收（plan 方向3-④）：把 data_dir 的 models/stt 改名后，
    /// resolve_stt_paths 必须返回结构化 model_missing 错误而非 panic，
    /// 用完自动还原。手动执行：
    ///   cargo test -p nuphus-desktop stt_degrade -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stt_degrade_missing_model_dir() {
        let Some(data_dir) = dirs::data_dir() else {
            return;
        };
        let stt = data_dir.join("Nuphus").join("models").join("stt");
        if !stt.is_dir() {
            eprintln!(
                "[stt_degrade] {} not present, nothing to test",
                stt.display()
            );
            return;
        }
        let bak = stt.with_extension("stt.bak");
        struct Restore(PathBuf, PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::rename(&self.0, &self.1);
            }
        }
        std::fs::rename(&stt, &bak).expect("rename stt dir failed");
        // env 候选指向不存在的目录，隔离其他 fallback，专测 data_dir 缺失路径
        std::env::set_var("NUPHUS_STT_MODELS_DIR", bak.with_extension("nonexistent"));
        let _guard = Restore(bak, stt);

        let result = resolve_stt_paths();
        // exe-relative / cwd 候选可能存在（热更新分发位置），两者皆可接受：
        // 关键判定是「不 panic 且能给出明确结果」。
        match &result {
            Ok(p) => eprintln!("[stt_degrade] fell back to {}", p.dir.display()),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.starts_with("model_missing:"), "unexpected: {msg}");
                eprintln!("[stt_degrade] structured error OK: {msg}");
            }
        }
        std::env::remove_var("NUPHUS_STT_MODELS_DIR");
    }
}
