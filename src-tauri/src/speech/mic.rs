//! Microphone capture via cpal (WASAPI on Windows) + mono downmix +
//! linear resampling to the 16kHz pipeline rate.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::Sender;

/// Active capture stream; dropping stops the microphone.
pub struct MicCapture {
    /// Held for Drop semantics only — stream runs while this struct lives.
    _stream: cpal::Stream,
    sample_rate: u32,
}

/// True if a default input device exists (cheap probe for stt_status).
pub fn mic_available() -> bool {
    cpal::default_host().default_input_device().is_some()
}

impl MicCapture {
    /// Open the default input device and start pushing mono f32 chunks
    /// (at the device's native rate) into `tx`.
    pub fn start(tx: Sender<Vec<f32>>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no_microphone: no default input device".to_string())?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("no_microphone: default_input_config failed: {e}"))?;

        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let err_fn = |e| tracing::warn!("[stt] mic stream error: {e}");

        // Convert any common sample format to mono f32 in the callback.
        // Allocation per chunk is unavoidable here (channel handoff); chunk
        // rate is ~30-100/s, negligible next to decode cost.
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &supported.config(),
                    move |data: &[f32], info| {
                        let mono = downmix_f32(data, channels);
                        if !mono.is_empty() {
                            let _ = tx.send(mono);
                        }
                        let _ = info;
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("mic stream build failed: {e}"))?,
            cpal::SampleFormat::I16 => {
                let tx2 = tx.clone();
                device
                    .build_input_stream(
                        &supported.config(),
                        move |data: &[i16], _| {
                            let mono = downmix_i16(data, channels);
                            if !mono.is_empty() {
                                let _ = tx2.send(mono);
                            }
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("mic stream build failed: {e}"))?
            }
            cpal::SampleFormat::U16 => {
                let tx3 = tx.clone();
                device
                    .build_input_stream(
                        &supported.config(),
                        move |data: &[u16], _| {
                            let mono = downmix_u16(data, channels);
                            if !mono.is_empty() {
                                let _ = tx3.send(mono);
                            }
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("mic stream build failed: {e}"))?
            }
            fmt => {
                return Err(format!("unsupported mic sample format: {fmt:?}"));
            }
        };

        stream
            .play()
            .map_err(|e| format!("mic stream start failed: {e}"))?;
        tracing::info!(
            "[stt] mic capture started: {} Hz, {} ch, {:?}",
            sample_rate,
            channels,
            supported.sample_format()
        );
        Ok(Self {
            _stream: stream,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

fn downmix_f32(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn downmix_i16(data: &[i16], channels: usize) -> Vec<f32> {
    let scale = 1.0 / 32768.0;
    if channels <= 1 {
        return data.iter().map(|&s| s as f32 * scale).collect();
    }
    data.chunks_exact(channels)
        .map(|f| f.iter().map(|&s| s as f32 * scale).sum::<f32>() / channels as f32)
        .collect()
}

fn downmix_u16(data: &[u16], channels: usize) -> Vec<f32> {
    let to_f = |s: u16| (s as f32 - 32768.0) / 32768.0;
    if channels <= 1 {
        return data.iter().map(|&s| to_f(s)).collect();
    }
    data.chunks_exact(channels)
        .map(|f| f.iter().map(|&s| to_f(s)).sum::<f32>() / channels as f32)
        .collect()
}

/// Stateful linear-interpolation resampler (device rate → 16kHz).
/// Identity when rates match. Linear is sufficient for VAD + sense-voice
/// (both are robust to mild HF loss) and avoids a heavy DSP dependency.
pub struct Resampler {
    /// Input samples advanced per output sample.
    step: f64,
    /// Fractional input position of the next output sample.
    cursor: f64,
    /// Last sample of the previous chunk (for cross-chunk interpolation).
    prev: Option<f32>,
    identity: bool,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        Self {
            step: from_hz as f64 / to_hz as f64,
            cursor: 0.0,
            prev: None,
            identity: from_hz == to_hz,
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if self.identity {
            out.extend_from_slice(input);
            return;
        }
        let n = input.len() as f64;
        while self.cursor < n {
            let i = self.cursor.floor();
            let frac = (self.cursor - i) as f32;
            let idx = i as isize;
            let s0 = if idx < 0 {
                self.prev.unwrap_or(input[0])
            } else {
                input[idx as usize]
            };
            let next = (idx + 1).max(0) as usize;
            let s1 = if next < input.len() {
                input[next]
            } else {
                input[input.len() - 1]
            };
            out.push(s0 + (s1 - s0) * frac);
            self.cursor += self.step;
        }
        self.cursor -= n;
        self.prev = Some(input[input.len() - 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passthrough() {
        let mut r = Resampler::new(16000, 16000);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn downsample_48k_to_16k_length() {
        let mut r = Resampler::new(48000, 16000);
        let input = vec![0.5f32; 4800];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        // 4800 input @3.0 step → 1600 outputs
        assert_eq!(out.len(), 1600);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn downmix_stereo() {
        let stereo = [1.0f32, -1.0, 0.5, 0.5];
        let mono = downmix_f32(&stereo, 2);
        assert_eq!(mono, vec![0.0, 0.5]);
    }
}
