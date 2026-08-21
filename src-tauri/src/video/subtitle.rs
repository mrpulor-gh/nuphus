//! Subtitle parsing & normalization: SRT / WebVTT → unified `Cue` list.
//!
//! - Auto-detects format by content (WEBVTT header → vtt, else srt).
//! - Strips HTML-style tags (`<i>`, `<c.xxx>`) and ASS override blocks (`{\...}`).
//! - Merges adjacent cues with identical text (auto-caption duplication).
//! - No external crate: both formats are block-based and share the `-->`
//!   timing line, so one block parser covers both.

/// One normalized subtitle cue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Parse SRT or VTT content into normalized cues (sorted by start time,
/// empty texts dropped, adjacent duplicates merged).
pub fn parse(content: &str) -> Vec<Cue> {
    let mut cues = parse_blocks(content);
    normalize(&mut cues);
    cues
}

/// `[mm:ss]` anchor used when rendering cues for the LLM / UI.
/// Used by tests today; kept pub as the canonical anchor format for the
/// module's consumers (frontend renders the same shape).
#[allow(dead_code)]
pub fn fmt_mm_ss(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    format!("{:02}:{:02}", total / 60, total % 60)
}

// ── Block parser (shared by srt/vtt) ────────────────────────────────────

fn parse_blocks(content: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    // Normalize line endings; split into blank-line-separated blocks.
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    for block in normalized.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        // Skip WEBVTT header / NOTE / STYLE / REGION blocks.
        let upper = block.to_uppercase();
        if upper.starts_with("WEBVTT")
            || upper.starts_with("NOTE")
            || upper.starts_with("STYLE")
            || upper.starts_with("REGION")
        {
            continue;
        }
        let mut timing: Option<(i64, i64)> = None;
        let mut text_lines: Vec<&str> = Vec::new();
        for line in block.lines() {
            if timing.is_none() {
                if let Some(t) = parse_timing_line(line) {
                    timing = Some(t);
                    continue;
                }
                // srt cue index / vtt cue identifier — ignore.
                continue;
            }
            text_lines.push(line);
        }
        if let Some((start_ms, end_ms)) = timing {
            let text = clean_text(&text_lines.join("\n"));
            if !text.is_empty() {
                cues.push(Cue {
                    start_ms,
                    end_ms,
                    text,
                });
            }
        }
    }
    cues
}

/// `hh:mm:ss,mmm --> hh:mm:ss,mmm` (srt) or `hh:mm:ss.mmm --> hh:mm:ss.mmm`
/// (vtt, optional position settings after the end time).
fn parse_timing_line(line: &str) -> Option<(i64, i64)> {
    let (start, rest) = line.split_once("-->")?;
    let start_ms = parse_ts(start.trim())?;
    // vtt appends positioning settings after the end timestamp.
    let end_token = rest.split_whitespace().next()?;
    let end_ms = parse_ts(end_token)?;
    Some((start_ms, end_ms.max(start_ms)))
}

/// `hh:mm:ss.mmm` / `hh:mm:ss,mmm` / `mm:ss.mmm`
fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', ".");
    let (hms, ms) = match s.split_once('.') {
        Some((hms, ms)) => (hms, ms),
        None => (s.as_str(), "0"),
    };
    let ms: i64 = ms.trim().parse().ok()?;
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec): (i64, i64, i64) = match parts.as_slice() {
        [m, sec] => (0, m.trim().parse().ok()?, sec.trim().parse().ok()?),
        [h, m, sec] => (
            h.trim().parse().ok()?,
            m.trim().parse().ok()?,
            sec.trim().parse().ok()?,
        ),
        _ => return None,
    };
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// Strip `<...>` tags and `{\...}` ASS override blocks, collapse whitespace
/// within each line, join multi-line cue text with a single space.
fn clean_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for c2 in chars.by_ref() {
                    if c2 == '>' {
                        break;
                    }
                }
            }
            '{' => {
                if chars.peek() == Some(&'\\') {
                    for c2 in chars.by_ref() {
                        if c2 == '}' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Sort by start, drop empties, merge adjacent same-text cues (auto captions
/// repeat the rolling line every few hundred ms).
fn normalize(cues: &mut Vec<Cue>) {
    cues.sort_by_key(|c| c.start_ms);
    cues.retain(|c| !c.text.is_empty());
    let mut merged: Vec<Cue> = Vec::with_capacity(cues.len());
    for cue in cues.drain(..) {
        if let Some(prev) = merged.last_mut() {
            if prev.text == cue.text && cue.start_ms <= prev.end_ms + 1000 {
                prev.end_ms = prev.end_ms.max(cue.end_ms);
                continue;
            }
        }
        merged.push(cue);
    }
    *cues = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_srt_basic() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\n大家好，欢迎收看\n\n2\n00:00:04,000 --> 00:00:06,000\n<i>今天我们讲</i> Rust\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 3500);
        assert_eq!(cues[0].text, "大家好，欢迎收看");
        assert_eq!(cues[1].text, "今天我们讲 Rust");
    }

    #[test]
    fn parse_vtt_with_header_and_settings() {
        let vtt = "WEBVTT\nKind: captions\n\nNOTE this is a comment\n\n00:01.000 --> 00:03.000 align:start position:0%\n第一句\n\ncue-2\n00:00:04.500 --> 00:00:06.000\n第二句\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].text, "第一句");
        assert_eq!(cues[1].start_ms, 4500);
        assert_eq!(cues[1].text, "第二句");
    }

    #[test]
    fn merge_adjacent_duplicates() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\n同一句\n\n2\n00:00:02,200 --> 00:00:04,000\n同一句\n\n3\n00:00:05,000 --> 00:00:06,000\n不同句\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].end_ms, 4000);
        assert_eq!(cues[1].text, "不同句");
    }

    #[test]
    fn strips_ass_override() {
        let cues = parse("1\n00:00:01,000 --> 00:00:02,000\n{\\an8}顶部字幕\n");
        assert_eq!(cues[0].text, "顶部字幕");
    }

    #[test]
    fn fmt_anchor() {
        assert_eq!(fmt_mm_ss(0), "00:00");
        assert_eq!(fmt_mm_ss(61_500), "01:01");
        assert_eq!(fmt_mm_ss(3_661_000), "61:01");
    }
}
