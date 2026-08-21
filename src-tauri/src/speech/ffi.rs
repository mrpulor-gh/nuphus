//! Thin hand-written FFI bindings to the prebuilt sherpa-onnx C API.
//!
//! All struct layouts are transcribed 1:1 from
//! `sherpa-onnx v1.13.4 include/sherpa-onnx/c-api/c-api.h`.
//! Only the subset used by the speech module is declared:
//! offline recognizer (sense-voice), voice activity detector (silero).
//!
//! Safety contract:
//! - All structs are plain-old-data; zero-init via `mem::zeroed()` is valid
//!   (mirrors the `memset(&config, 0, sizeof(config))` pattern in c-api.h docs).
//! - `*const c_char` fields must point to NUL-terminated strings that outlive
//!   the create call (sherpa-onnx copies them internally at creation time).

#![allow(non_snake_case)]
#![allow(dead_code)]

use std::os::raw::c_char;

// ── Shared config structs ─────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxFeatureConfig {
    pub sample_rate: i32,
    pub feature_dim: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxHomophoneReplacerConfig {
    pub dict_dir: *const c_char,
    pub lexicon: *const c_char,
    pub rule_fsts: *const c_char,
}

// ── Offline model config (full layout, field order from c-api.h) ──────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTransducerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub joiner: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineParaformerModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineNemoEncDecCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWhisperModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub task: *const c_char,
    pub tail_paddings: i32,
    pub enable_token_timestamps: i32,
    pub enable_segment_timestamps: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTdnnModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineSenseVoiceModelConfig {
    pub model: *const c_char,
    pub language: *const c_char,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMoonshineModelConfig {
    pub preprocessor: *const c_char,
    pub encoder: *const c_char,
    pub uncached_decoder: *const c_char,
    pub cached_decoder: *const c_char,
    pub merged_decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineDolphinModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineZipformerCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCanaryModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub src_lang: *const c_char,
    pub tgt_lang: *const c_char,
    pub use_pnc: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWenetCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineOmnilingualAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFunASRNanoModelConfig {
    pub encoder_adaptor: *const c_char,
    pub llm: *const c_char,
    pub embedding: *const c_char,
    pub tokenizer: *const c_char,
    pub system_prompt: *const c_char,
    pub user_prompt: *const c_char,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub language: *const c_char,
    pub itn: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineQwen3ASRModelConfig {
    pub conv_frontend: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub tokenizer: *const c_char,
    pub max_total_len: i32,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCohereTranscribeModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub use_punct: i32,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineModelConfig {
    pub transducer: SherpaOnnxOfflineTransducerModelConfig,
    pub paraformer: SherpaOnnxOfflineParaformerModelConfig,
    pub nemo_ctc: SherpaOnnxOfflineNemoEncDecCtcModelConfig,
    pub whisper: SherpaOnnxOfflineWhisperModelConfig,
    pub tdnn: SherpaOnnxOfflineTdnnModelConfig,
    pub tokens: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
    pub model_type: *const c_char,
    pub modeling_unit: *const c_char,
    pub bpe_vocab: *const c_char,
    pub telespeech_ctc: *const c_char,
    pub sense_voice: SherpaOnnxOfflineSenseVoiceModelConfig,
    pub moonshine: SherpaOnnxOfflineMoonshineModelConfig,
    pub fire_red_asr: SherpaOnnxOfflineFireRedAsrModelConfig,
    pub dolphin: SherpaOnnxOfflineDolphinModelConfig,
    pub zipformer_ctc: SherpaOnnxOfflineZipformerCtcModelConfig,
    pub canary: SherpaOnnxOfflineCanaryModelConfig,
    pub wenet_ctc: SherpaOnnxOfflineWenetCtcModelConfig,
    pub omnilingual: SherpaOnnxOfflineOmnilingualAsrCtcModelConfig,
    pub medasr: SherpaOnnxOfflineMedAsrCtcModelConfig,
    pub funasr_nano: SherpaOnnxOfflineFunASRNanoModelConfig,
    pub fire_red_asr_ctc: SherpaOnnxOfflineFireRedAsrCtcModelConfig,
    pub qwen3_asr: SherpaOnnxOfflineQwen3ASRModelConfig,
    pub cohere_transcribe: SherpaOnnxOfflineCohereTranscribeModelConfig,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineLMConfig {
    pub model: *const c_char,
    pub scale: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineRecognizerConfig {
    pub feat_config: SherpaOnnxFeatureConfig,
    pub model_config: SherpaOnnxOfflineModelConfig,
    pub lm_config: SherpaOnnxOfflineLMConfig,
    pub decoding_method: *const c_char,
    pub max_active_paths: i32,
    pub hotwords_file: *const c_char,
    pub hotwords_score: f32,
    pub rule_fsts: *const c_char,
    pub rule_fars: *const c_char,
    pub blank_penalty: f32,
    pub hr: SherpaOnnxHomophoneReplacerConfig,
}

// ── VAD config ────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxSileroVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxTenVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxVadModelConfig {
    pub silero_vad: SherpaOnnxSileroVadModelConfig,
    pub sample_rate: i32,
    pub num_threads: i32,
    pub provider: *const c_char,
    pub debug: i32,
    pub ten_vad: SherpaOnnxTenVadModelConfig,
}

// ── Opaque handles & result structs ───────────────────────────────────

#[repr(C)]
pub struct SherpaOnnxOfflineRecognizer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SherpaOnnxOfflineStream {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SherpaOnnxVoiceActivityDetector {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SherpaOnnxOfflineRecognizerResult {
    pub text: *const c_char,
    pub timestamps: *mut f32,
    pub count: i32,
    // Trailing fields (tokens_arr, json, lang, emotion, event) exist in the
    // C struct but are never touched here; we only read `text`.
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxSpeechSegment {
    pub start: i32,
    pub samples: *mut f32,
    pub n: i32,
}

// ── Functions ─────────────────────────────────────────────────────────

extern "C" {
    pub fn SherpaOnnxGetVersionStr() -> *const c_char;

    pub fn SherpaOnnxCreateOfflineRecognizer(
        config: *const SherpaOnnxOfflineRecognizerConfig,
    ) -> *const SherpaOnnxOfflineRecognizer;
    pub fn SherpaOnnxDestroyOfflineRecognizer(recognizer: *const SherpaOnnxOfflineRecognizer);

    pub fn SherpaOnnxCreateOfflineStream(
        recognizer: *const SherpaOnnxOfflineRecognizer,
    ) -> *const SherpaOnnxOfflineStream;
    pub fn SherpaOnnxDestroyOfflineStream(stream: *const SherpaOnnxOfflineStream);
    pub fn SherpaOnnxAcceptWaveformOffline(
        stream: *const SherpaOnnxOfflineStream,
        sample_rate: i32,
        samples: *const f32,
        n: i32,
    );
    pub fn SherpaOnnxDecodeOfflineStream(
        recognizer: *const SherpaOnnxOfflineRecognizer,
        stream: *const SherpaOnnxOfflineStream,
    );
    pub fn SherpaOnnxGetOfflineStreamResult(
        stream: *const SherpaOnnxOfflineStream,
    ) -> *const SherpaOnnxOfflineRecognizerResult;
    pub fn SherpaOnnxDestroyOfflineRecognizerResult(r: *const SherpaOnnxOfflineRecognizerResult);

    pub fn SherpaOnnxCreateVoiceActivityDetector(
        config: *const SherpaOnnxVadModelConfig,
        buffer_size_in_seconds: f32,
    ) -> *const SherpaOnnxVoiceActivityDetector;
    pub fn SherpaOnnxDestroyVoiceActivityDetector(p: *const SherpaOnnxVoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorAcceptWaveform(
        p: *const SherpaOnnxVoiceActivityDetector,
        samples: *const f32,
        n: i32,
    );
    pub fn SherpaOnnxVoiceActivityDetectorEmpty(p: *const SherpaOnnxVoiceActivityDetector) -> i32;
    pub fn SherpaOnnxVoiceActivityDetectorDetected(
        p: *const SherpaOnnxVoiceActivityDetector,
    ) -> i32;
    pub fn SherpaOnnxVoiceActivityDetectorFront(
        p: *const SherpaOnnxVoiceActivityDetector,
    ) -> *const SherpaOnnxSpeechSegment;
    pub fn SherpaOnnxVoiceActivityDetectorPop(p: *const SherpaOnnxVoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorClear(p: *const SherpaOnnxVoiceActivityDetector);
    pub fn SherpaOnnxVoiceActivityDetectorFlush(p: *const SherpaOnnxVoiceActivityDetector);
    pub fn SherpaOnnxDestroySpeechSegment(p: *const SherpaOnnxSpeechSegment);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    /// STT 链接链冒烟：能调通 C API 即证明 build.rs 的链接配置
    /// （cargo:rustc-link-lib=sherpa-onnx-c-api）+ DLL 加载 + 符号解析全链路 OK。
    /// 需要 sherpa-onnx-c-api.dll 与 onnxruntime.dll 可在 exe 目录加载
    /// （build.rs sync_sherpa_libs 已拷到 target/<profile>/ 与 deps/）。
    #[test]
    fn sherpa_c_api_loads_and_returns_version() {
        let ptr = unsafe { SherpaOnnxGetVersionStr() };
        assert!(
            !ptr.is_null(),
            "SherpaOnnxGetVersionStr 返回 null —— DLL 加载/链接失败"
        );
        let version = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        assert!(!version.is_empty(), "版本字符串为空");
        assert!(
            version.contains("1.13"),
            "预期 sherpa v1.13，实际: {version}"
        );
        println!("sherpa-onnx C API 加载成功, version={version}");
    }
}
