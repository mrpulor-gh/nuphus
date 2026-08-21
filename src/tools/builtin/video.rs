//! video — 视频字幕获取工具（video_subtitle_extract）
//!
//! 薄封装：实际管线在桌面壳（src-tauri/src/video/，四级降级实现前三级：
//! 内嵌字幕流 → 平台字幕 → 本地 ASR），经 crate::video_bridge 注入的
//! fn 指针调用（单进程，无 IPC）。本工具只返回带时间戳的字幕全文，
//! 内容理解由 LLM 在对话内完成。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

impl ToolRegistry {
    pub(crate) fn register_video_subtitle_extract(&mut self) {
        self.register(ToolDef {
            name: "video_subtitle_extract".to_string(),
            description: "当用户给出视频链接（B站/YouTube 等在线视频）或本地视频文件路径，并要求了解视频内容/总结/讲了什么时使用。返回带 [mm:ss] 时间戳的字幕/转录全文（来源自动降级：内嵌字幕流 > 平台字幕 > 本地语音识别 ASR，能拿字幕绝不识别）。你应基于返回内容回答用户，并在讲解中引用 [mm:ss] 时间锚点。本工具只获取字幕，不做内容理解；无字幕视频需下载音频并本地识别，耗时数分钟属正常现象。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "视频 URL（http/https）或本地视频文件绝对路径"
                    }
                },
                "required": ["input"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let input = params
                    .get("input")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if input.is_empty() {
                    return Ok(ToolResult::failure("input cannot be empty"));
                }
                match crate::video_bridge::extract(&input) {
                    Ok(json) => Ok(ToolResult::success(format_tool_output(&json))),
                    Err(e) => Ok(ToolResult::failure(format!("视频字幕获取失败：{e}"))),
                }
            },
            depends_on: vec![],
        });
    }
}

// ── Result rendering ────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct BridgeCue {
    start_ms: i64,
    /// Reserved for future segment-range display; kept for schema parity
    /// with the shell's VideoSubtitleResult.
    #[allow(dead_code)]
    end_ms: i64,
    text: String,
}

#[derive(serde::Deserialize)]
struct BridgeResult {
    source: String,
    title: Option<String>,
    duration_ms: Option<i64>,
    cues: Vec<BridgeCue>,
    #[serde(default)]
    truncated: bool,
}

/// Render bridge JSON → header + "[mm:ss] text" lines for the LLM.
/// Malformed JSON passes through raw (never silently dropped).
pub fn format_tool_output(json: &str) -> String {
    let parsed: BridgeResult = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(_) => return json.to_string(),
    };
    let src_label = match parsed.source.as_str() {
        "embedded" => "内嵌字幕",
        "platform" => "平台字幕",
        "asr" => "本地语音识别",
        other => other,
    };
    let mut out = String::new();
    if let Some(t) = &parsed.title {
        out.push_str(&format!("标题：{}\n", t));
    }
    if let Some(d) = parsed.duration_ms {
        out.push_str(&format!("时长：{}\n", fmt_mm_ss(d)));
    }
    out.push_str(&format!(
        "字幕来源：{}（{} 段）\n\n",
        src_label,
        parsed.cues.len()
    ));
    for cue in &parsed.cues {
        out.push_str(&format!("[{}] {}\n", fmt_mm_ss(cue.start_ms), cue.text));
    }
    if parsed.truncated {
        out.push_str("\n[字幕过长已截断，以上为前部分内容]\n");
    }
    out
}

fn fmt_mm_ss(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bridge_json() {
        let json = r#"{"source":"platform","title":"测试视频","duration_ms":65000,"cues":[{"start_ms":1000,"end_ms":3000,"text":"第一句"},{"start_ms":61500,"end_ms":63000,"text":"第二句"}],"truncated":false}"#;
        let out = format_tool_output(json);
        assert!(out.contains("标题：测试视频"));
        assert!(out.contains("时长：01:05"));
        assert!(out.contains("字幕来源：平台字幕（2 段）"));
        assert!(out.contains("[00:01] 第一句"));
        assert!(out.contains("[01:01] 第二句"));
    }

    #[test]
    fn malformed_json_passthrough() {
        assert_eq!(format_tool_output("not json"), "not json");
    }

    #[test]
    fn truncated_marker() {
        let json = r#"{"source":"asr","title":null,"duration_ms":null,"cues":[],"truncated":true}"#;
        assert!(format_tool_output(json).contains("已截断"));
    }

    #[test]
    fn bridge_cue_end_ms_deserialized() {
        // end_ms is reserved for schema parity; verify it deserializes correctly.
        let json = r#"{"start_ms":1000,"end_ms":3000,"text":"hello"}"#;
        let cue: BridgeCue = serde_json::from_str(json).unwrap();
        assert_eq!(cue.start_ms, 1000);
        assert_eq!(cue.end_ms, 3000);
        assert_eq!(cue.text, "hello");
    }
}
