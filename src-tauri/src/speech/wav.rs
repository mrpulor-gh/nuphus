//! Minimal RIFF/WAVE codec for the speech module.
//! Reader: PCM 16-bit and IEEE-float 32-bit, any channel count / sample rate
//! (downmixed + resampled by the caller via `mic::Resampler`).
//! Encoder: mono f32 → PCM16 bytes, the cloud-upload payload format.

use std::path::Path;

pub struct WavData {
    pub samples: Vec<f32>, // mono
    pub sample_rate: u32,
}

pub fn read_wav(path: &Path) -> Result<WavData, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {} failed: {e}", path.display()))?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }

    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (format, channels, rate, bits)
    let mut data: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " if body_end - body_start >= 16 => {
                let b = &bytes[body_start..body_end];
                let format = u16::from_le_bytes(b[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(b[2..4].try_into().unwrap());
                let rate = u32::from_le_bytes(b[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes(b[14..16].try_into().unwrap());
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        // Chunks are word-aligned.
        pos = body_start + size + (size & 1);
    }

    let (format, channels, rate, bits) =
        fmt.ok_or_else(|| format!("{}: missing fmt chunk", path.display()))?;
    let data = data.ok_or_else(|| format!("{}: missing data chunk", path.display()))?;
    let ch = channels.max(1) as usize;

    let samples: Vec<f32> = match (format, bits) {
        (1, 16) => {
            let raw: Vec<i16> = data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            downmix(&raw, ch, |s| s as f32 / 32768.0)
        }
        (3, 32) => {
            let raw: Vec<f32> = data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            downmix(&raw, ch, |s| s)
        }
        _ => {
            return Err(format!(
            "{}: unsupported wav encoding (format={format}, bits={bits}); need PCM16 or float32",
            path.display()
        ))
        }
    };

    Ok(WavData {
        samples,
        sample_rate: rate,
    })
}

fn downmix<T: Copy>(raw: &[T], channels: usize, to_f: impl Fn(T) -> f32) -> Vec<f32> {
    if channels <= 1 {
        return raw.iter().map(|&s| to_f(s)).collect();
    }
    raw.chunks_exact(channels)
        .map(|f| f.iter().map(|&s| to_f(s)).sum::<f32>() / channels as f32)
        .collect()
}

/// Encode mono f32 samples as PCM16 RIFF/WAVE bytes — the multipart payload
/// for the cloud transcription upload (cloud.rs). Header layout mirrors the
/// reader above so `read_wav` round-trips it.
pub fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = samples.len() as u32 * 2;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal PCM16 mono wav in memory and round-trip it.
    #[test]
    fn pcm16_roundtrip() {
        let dir = std::env::temp_dir().join("nuphus_stt_wav_test.wav");
        let samples: Vec<i16> = vec![0, 16384, -16384, 32767, -32768];
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36u32 + samples.len() as u32 * 2).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&16000u32.to_le_bytes());
        buf.extend_from_slice(&32000u32.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
        for s in &samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::File::create(&dir)
            .unwrap()
            .write_all(&buf)
            .unwrap();

        let wav = read_wav(&dir).unwrap();
        assert_eq!(wav.sample_rate, 16000);
        assert_eq!(wav.samples.len(), 5);
        assert!((wav.samples[1] - 0.5).abs() < 1e-4);
        assert!((wav.samples[2] + 0.5).abs() < 1e-4);
        let _ = std::fs::remove_file(&dir);
    }

    /// encode_wav_pcm16 output must round-trip through read_wav (the cloud
    /// payload is exactly what our own reader accepts) and clamp outliers.
    #[test]
    fn encode_pcm16_roundtrip() {
        let dir = std::env::temp_dir().join("nuphus_stt_wav_encode_test.wav");
        let samples = vec![0.0f32, 0.5, -0.5, 1.5, -1.5]; // ±1.5 → clamped
        std::fs::write(&dir, encode_wav_pcm16(&samples, 16000)).unwrap();

        let wav = read_wav(&dir).unwrap();
        assert_eq!(wav.sample_rate, 16000);
        assert_eq!(wav.samples.len(), 5);
        assert!((wav.samples[1] - 0.5).abs() < 1e-4);
        assert!((wav.samples[2] + 0.5).abs() < 1e-4);
        assert!((wav.samples[3] - 1.0).abs() < 1e-4);
        assert!((wav.samples[4] + 1.0).abs() < 1e-4);
        let _ = std::fs::remove_file(&dir);
    }
}
