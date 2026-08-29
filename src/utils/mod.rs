//! Utils module

use std::path::PathBuf;

pub mod automation_lock;
pub mod office;
pub mod proxy;
pub mod xlsx;
pub mod xlsx_write;

/// Truncate text to specified character count, add truncation marker
pub fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        format!(
            "{}...\n[output truncated, {} characters total]",
            text.chars().take(max_chars).collect::<String>(),
            text.chars().count(),
        )
    } else {
        text.to_string()
    }
}

/// Smart truncation: Read/Grep use 16000 + head-tail preservation, others use simple tail truncation.
/// Head-tail keeps 60% head + 40% tail so LLM doesn't miss bottom-of-file logic.
/// task_dispatch 豁免：Exec 报告是给 Leader 的核心交付物，截断等于砍掉工作成果。
pub fn truncate_tool_output(text: &str, max_chars: usize, tool_name: &str) -> String {
    if tool_name == "task_dispatch" {
        return text.to_string();
    }
    let is_reader = tool_name == "Read" || tool_name == "Grep";
    let limit = if is_reader { 16000 } else { max_chars };

    if text.chars().count() <= limit {
        return text.to_string();
    }

    if is_reader {
        let head_chars = (limit as f64 * 0.6) as usize;
        let tail_chars = (limit as f64 * 0.4) as usize;
        let head: String = text.chars().take(head_chars).collect();
        let tail: String = text
            .chars()
            .rev()
            .take(tail_chars)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let skipped = text.chars().count() - head_chars - tail_chars;
        format!(
            "{}\n\n[中间 {} 字符已截断，原始长度 {} 字符]\n\n{}",
            head,
            skipped,
            text.chars().count(),
            tail
        )
    } else {
        format!(
            "{}...\n[输出已截断，原始长度 {} 字符]",
            text.chars().take(limit).collect::<String>(),
            text.chars().count(),
        )
    }
}

/// Round a byte index down to the nearest valid UTF-8 character boundary.
///
/// When truncating a `&str` via raw byte slicing (e.g. `&s[start..]`), the
/// start index must fall on a char boundary — slicing in the middle of a
/// multi-byte character (CJK, emoji, etc.) will panic.
#[inline]
pub fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Remove XML/HTML tags that commonly leak from LLM output:
/// - `<think>...</think>` reasoning blocks (MiniMax/DeepSeek)
/// - `<invoke>...</invoke>` / `<parameter>...</parameter>` (tool-call XML format embedded in text)
///
/// Handles cross-chunk splits: matches `<think` (with or without `>`) and strips
/// everything until `</think>`. When no closing tag is found, strips from `<think`
/// to end of text — this is correct for the accumulated text at MessageStop where
/// `</think>` may have been removed by per-chunk stripping.
pub fn strip_think_tags(text: &str) -> String {
    strip_xml_block(text, "<think", "</think>")
}

/// Strip tool-call XML blocks from text.
///
/// Removes `<invoke>...</invoke>`, `<command>...</command>`,
/// `<parameter>...</parameter>`, and `<tool_call>...</tool_call>` blocks
/// entirely (open tag + content + close tag). These leak from MiniMax /
/// fine-tuned models that embed tool calls as XML in text content.
///
/// Pass additional tag names in `extra_tags` for provider-specific formats.
pub fn strip_tool_xml_tags_with_extra(text: &str, extra_tags: &[&str]) -> String {
    let mut text = strip_xml_block(text, "<invoke", "</invoke>");
    text = strip_xml_block(&text, "<command", "</command>");
    text = strip_xml_block(&text, "<parameter", "</parameter>");
    text = strip_xml_block(&text, "<tool_call", "</tool_call>");
    for tag in extra_tags {
        let open = format!("<{}", tag);
        let close = format!("</{}>", tag);
        text = strip_xml_block(&text, &open, &close);
    }
    text
}

/// Backward-compatible wrapper — strips built-in tags only.
pub fn strip_tool_xml_tags(text: &str) -> String {
    strip_tool_xml_tags_with_extra(text, &[])
}

/// Search `haystack` for `close_tag`, accepting optional whitespace
/// before the trailing `>`. Returns `(pos, matched_len)` where `matched_len`
/// accounts for any whitespace consumed.
///
/// Example: searching `</think>` in `"...</think >..."` returns the position
/// of `</think` with `matched_len = 9` (`</think >`).
fn close_tag_search(haystack: &str, close_tag: &str) -> Option<(usize, usize)> {
    // close_tag is e.g. "</think>", we search for the base "</think"
    let base = close_tag.trim_end_matches('>');
    let pos = haystack.find(base)?;
    // After base, skip optional whitespace and expect '>'
    let after = &haystack[pos + base.len()..];
    let ws_end = after
        .char_indices()
        .take_while(|(_, c)| c.is_whitespace())
        .last()
        .map(|(i, _)| i + 1)
        .unwrap_or(0);
    if after[ws_end..].starts_with('>') {
        Some((pos, base.len() + ws_end + 1))
    } else {
        None
    }
}

/// Search `haystack` for the next real `<think` open tag.
///
/// ⚠️ 与入口处防护一致：仅当 `<think` 后紧跟 `>`（或空白 + `>`）才视为
/// think 标签。正文中讨论标签字面量（如 "剥离 <think 标签"）不以 `>` 结尾，
/// 必须跳过——否则嵌套栈会把后续所有内容误吞进 reasoning，正文被截断。
///
/// Returns `(pos, matched_len)` where `matched_len` accounts for the trailing
/// `>` if present (or the leading whitespace consumed before `>`).
fn find_think_open(haystack: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while let Some(pos) = haystack[search_from..].find("<think") {
        let abs = search_from + pos;
        let after_open = &haystack[abs + 6..]; // skip "<think"
        let looks_like_tag = after_open.starts_with('>')
            || (after_open
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
                && after_open.trim_start().starts_with('>'));
        if looks_like_tag {
            // 找到真正标签：len 包含 `>`（或 空白+`>`）
            let after_trimmed = after_open.trim_start();
            let len = if after_open.starts_with('>') {
                7
            } else {
                6 + (after_open.len() - after_trimmed.len()) + 1
            };
            return Some((abs, len));
        }
        // 字面量（如 "剥离 <think 标签"）：跳过 "<think"，继续向后找真标签
        search_from = abs + 6;
    }
    None
}

/// Final sanitisation pass: remove residual tag fragments that survive
/// the main extraction/strip loop. Handles full orphaned close tags and
/// cross-chunk partial fragments like `</think` or `</invoke`.
pub fn clean_think_remnants(text: &str) -> String {
    let mut result = text.replace("</think>", "");
    // Close tags with spurious whitespace (e.g. </think >)
    result = result.replace("</think >", "");
    result = result.replace("</invoke>", "");
    result = result.replace("</parameter>", "");
    // Cross-chunk fragments: partial close tags missing '>'
    result = result.replace("</think", "");
    result = result.replace("</invoke", "");
    result = result.replace("</parameter", "");
    result
}

/// Strip a paired XML block (open → close), handling the case where the
/// open tag may or may not include a trailing `>`.
fn strip_xml_block(text: &str, open_tag: &str, close_tag: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find(open_tag) {
        // Keep text before the open tag
        result.push_str(&remaining[..start]);

        // Skip past the open tag
        let after_open = &remaining[start + open_tag.len()..];

        // If the tag has a trailing `>` (e.g. <think> vs <think), skip it
        let after_tag = after_open.strip_prefix('>').unwrap_or(after_open);

        // Find matching close tag (with optional whitespace before '>')
        match close_tag_search(after_tag, close_tag) {
            Some((end, close_len)) => {
                // Strip everything between open and close tags
                remaining = &after_tag[end + close_len..];
            }
            None => {
                // No matching close tag — strip from open tag to end.
                // This is correct for:
                // 1. <think> without </think> in accumulated text (orphaned by per-chunk strip)
                // 2. <think (no >) at chunk boundary — strip the partial tag
                remaining = "";
            }
        }
    }
    result.push_str(remaining);

    clean_think_remnants(&result)
}

/// Extract `<think>...</think>` reasoning blocks from text.
///
/// Returns `(clean_text, reasoning_text)` where:
/// - `clean_text`: text with think blocks removed (and orphaned close tags stripped)
/// - `reasoning_text`: concatenated content from all think blocks
///
/// This is the canonical function for parsing think blocks from accumulated
/// text at MessageStop/Cancelled in `process_events`.
pub fn extract_think_blocks(text: &str) -> (String, String) {
    let mut clean = String::with_capacity(text.len());
    let mut reasoning = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("<think") {
        // ⚠️ 仅当 `<think` 后紧跟 `>`（或空白 + `>`）才视为 think 标签。
        // 正文中讨论标签本身的字面量（如 "剥离 <think 标签"）不以 `>` 结尾，
        // 必须跳过，否则会把后续所有内容误吞进 reasoning，导致消息截断。
        let after_open = &remaining[start + 6..]; // skip "<think"
        let looks_like_tag = after_open.starts_with('>')
            || after_open
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
                && {
                    let after_ws = after_open.trim_start();
                    after_ws.starts_with('>')
                };
        if !looks_like_tag {
            // 字面量讨论（不是标签）：保留 `<think` 原文，继续向后查找真正的标签
            clean.push_str(&remaining[..start + 6]);
            remaining = &remaining[start + 6..];
            continue;
        }
        clean.push_str(&remaining[..start]);
        let after_tag = after_open.strip_prefix('>').unwrap_or(after_open);

        // Use stack-based matching to find the correct closing tag.
        // This handles nested <think references inside reasoning content
        // (e.g. when the model discusses "<think>" as part of its analysis).
        let mut depth: u32 = 1;
        let mut search_pos = 0;
        let end: Option<(usize, usize)> = loop {
            // Find next "<think" open (real tag only — literal "<think 标签" skipped)
            let next_open = find_think_open(&after_tag[search_pos..]);

            // Find next "</think>" (close with optional whitespace before '>')
            let next_close = close_tag_search(&after_tag[search_pos..], "</think>");

            match (next_open, next_close) {
                (Some((o, open_len)), Some((c, _))) if o < c => {
                    depth += 1;
                    search_pos += o + open_len;
                }
                (_, Some((c, close_len))) => {
                    depth -= 1;
                    if depth == 0 {
                        break Some((search_pos + c, close_len));
                    }
                    search_pos += c + close_len;
                }
                (Some(_), None) => {
                    // Unclosed nested <think — treat as reasoning
                    break None;
                }
                (None, None) => {
                    break None;
                }
            }
        };
        match end {
            Some((end, close_len)) => {
                reasoning.push_str(&after_tag[..end]);
                let right = &after_tag[end + close_len..];
                if right.is_empty() {
                    // 块在末尾：保留左侧原样（含尾随空白）
                    remaining = right;
                } else if clean.is_empty() {
                    // 块在开头：去掉右侧前导空白（" Done" → "Done"）
                    remaining = right.trim_start();
                } else if clean.ends_with(char::is_whitespace)
                    && right.starts_with(char::is_whitespace)
                {
                    // 双侧空白边界：折叠为单个空格（"Before  x" → "Before x"）
                    let collapsed = clean.trim_end().to_string();
                    clean = collapsed;
                    clean.push(' ');
                    remaining = right.trim_start();
                } else {
                    remaining = right;
                }
            }
            None => {
                // No matching closing tag — treat everything from <think onward as reasoning.
                // This handles: interrupted streaming, chunk-boundary splits where
                // </think> was processed in a previous chunk.
                reasoning.push_str(after_tag);
                remaining = "";
            }
        }
    }

    // Append any remaining text (after last </think> or if no <think found)
    clean.push_str(remaining);

    let clean = clean_think_remnants_folded(&clean);
    // 嵌套场景下 reasoning 含内层标签标记（内容保留、标记剥离）
    let reasoning = clean_think_remnants(&reasoning);
    (clean, reasoning)
}

/// `extract_think_blocks` 专用的残余清理：与 `clean_think_remnants` 相同的
/// 残余标签集合（think/invoke/parameter 完整 + 截断片段），但移除时折叠
/// 边界空白并修剪首尾——被移除的跨 chunk 残余标签处文本应自然衔接。
///
/// 流式路径（agent/common.rs）继续使用 `clean_think_remnants`，不做折叠，
/// 避免吃掉合法的流式空白。
fn clean_think_remnants_folded(text: &str) -> String {
    const FULL_TAGS: &[&str] = &["</think>", "</invoke>", "</parameter>"];
    const PARTIALS: &[&str] = &["</think", "</invoke", "</parameter"];

    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        // 最近的完整闭合标签（允许 '>' 前有空白，借 close_tag_search 处理）
        let full_hit = FULL_TAGS
            .iter()
            .filter_map(|t| close_tag_search(rest, t))
            .min_by_key(|(pos, _)| *pos);
        // 最近的截断片段（缺 '>'）
        let partial_hit = PARTIALS
            .iter()
            .filter_map(|p| rest.find(p).map(|pos| (pos, p.len())))
            .min_by_key(|(pos, _)| *pos);

        // 同位置时优先完整标签（"</think >" 应整体消费，而非只消费 "</think"）
        let hit = match (full_hit, partial_hit) {
            (Some(f), Some(p)) => Some(if f.0 <= p.0 { f } else { p }),
            (Some(f), None) => Some(f),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        };

        let Some((pos, len)) = hit else { break };
        result.push_str(&rest[..pos]);
        let right = &rest[pos + len..];
        if right.is_empty() {
            // 残余在末尾：去掉左侧尾随空白
            let trimmed = result.trim_end().to_string();
            result = trimmed;
            rest = "";
        } else if result.is_empty() {
            // 残余在开头：去掉右侧前导空白
            rest = right.trim_start();
        } else if result.ends_with(char::is_whitespace) && right.starts_with(char::is_whitespace) {
            // 双侧空白边界：折叠为单个空格
            let trimmed = result.trim_end().to_string();
            result = trimmed;
            result.push(' ');
            rest = right.trim_start();
        } else {
            rest = right;
        }
    }
    result.push_str(rest);
    result
}

/// Convert a BMP base64 data URL to PNG base64 data URL.
///
/// LLM APIs (MiniMax, etc.) reject `image/bmp`. This function decodes the BMP,
/// re-encodes as PNG, and returns a `data:image/png;base64,...` URL.
pub fn convert_bmp_data_url_to_png(data_url: &str) -> Result<String, String> {
    let b64 = data_url
        .split(',')
        .nth(1)
        .ok_or_else(|| "invalid data URL format".to_string())?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let img = image::load_from_memory(&decoded).map_err(|e| format!("image decode failed: {e}"))?;
    let mut png_buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png_buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    let png_b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
    Ok(format!("data:image/png;base64,{}", png_b64))
}

/// Process a streaming text delta chunk, routing `<think>...</think>` content to
/// reasoning (execution-panel timeline with `is_thinking: true`) and non-think
/// text to the chat bubble.
///
/// Uses `AtomicU32` to track think-block nesting depth across streaming chunks.
/// Emits thinking content delta-by-delta in real-time — no buffering.
/// Depth tracking prevents premature close when LLM discusses `` tags.
/// The frontend timeline appends same-kind entries, so each thinking chunk
/// extends the previous one naturally. Orphaned tag fragments (e.g. `</thin`
/// at chunk boundaries) are cosmetic only — `extract_think_blocks` handles
/// the final accumulated text for memory / session storage.
///
/// Returns `(reasoning_to_emit, text_to_emit)` — both owned `String`s.
/// - `reasoning_to_emit`: thinking chunk to stream immediately (Some for each delta)
/// - `text_to_emit`: non-think text; caller emits both in order (reasoning first)
pub fn process_text_delta(
    text: &str,
    think_depth: &std::sync::atomic::AtomicU32,
) -> (Option<String>, String) {
    use std::sync::atomic::Ordering;

    let depth = think_depth.load(Ordering::SeqCst);

    if depth > 0 {
        // ── Inside think block(s) — scan for tags with depth tracking ──
        let (reasoning, remaining, new_depth) = scan_think_with_depth(text, depth);
        think_depth.store(new_depth, Ordering::SeqCst);
        let text_out = if new_depth == 0 {
            strip_tool_xml_tags(&remaining)
        } else {
            String::new()
        };
        (reasoning, text_out)
    } else {
        // ── Not in a think block — look for opening `<think` tag ──
        match text.find("<think") {
            Some(start) => {
                let before = &text[..start];
                let after = &text[start + 6..];
                // ⚠️ 仅当 `<think` 后紧跟 `>`（或空白 + `>`）才视为 think 标签。
                // 正文中讨论标签字面量（如 "剥离 <think 标签"）不以 `>` 结尾——
                // 若误判为标签会把后续所有内容吞进 thinking，消息被截断。
                let looks_like_tag = after.starts_with('>')
                    || (after
                        .chars()
                        .next()
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                        && after.trim_start().starts_with('>'));
                if !looks_like_tag {
                    // 字面量讨论（如 "剥离 <think 标签"）：不是 think 标签，
                    // 整块按普通文本输出，绝不在 start 处截断。
                    return (None, strip_tool_xml_tags(text));
                }
                let after = after.strip_prefix('>').unwrap_or(after);

                // Process from here as if inside think (depth = 1)
                let (reasoning, remaining, new_depth) = scan_think_with_depth(after, 1);
                think_depth.store(new_depth, Ordering::SeqCst);
                let text_out = if new_depth == 0 {
                    // think 块在本 chunk 内完整闭合：折叠 before 尾部与 remaining 头部的
                    // 边界空白（与 extract_think_blocks 的折叠语义一致），避免
                    // "正文\n\n\n\n后续" 式多余空白泄漏进消息气泡。
                    strip_tool_xml_tags(&fold_boundary_whitespace(before, &remaining))
                } else {
                    strip_tool_xml_tags(before)
                };
                (reasoning, text_out)
            }
            None => (None, strip_tool_xml_tags(text)),
        }
    }
}

/// 折叠 think 块边界空白：before 尾部与 remaining 头部都含空白时折叠为单个空格，
/// 与 `extract_think_blocks` 的折叠语义一致（"Before  x" → "Before x"）。
fn fold_boundary_whitespace(before: &str, remaining: &str) -> String {
    if before.is_empty() {
        return remaining.trim_start().to_string();
    }
    if remaining.is_empty() {
        return before.trim_end().to_string();
    }
    let before_ends_ws = before
        .chars()
        .last()
        .map(|c| c.is_whitespace())
        .unwrap_or(false);
    let remaining_starts_ws = remaining
        .chars()
        .next()
        .map(|c| c.is_whitespace())
        .unwrap_or(false);
    if before_ends_ws && remaining_starts_ws {
        let mut out = before.trim_end().to_string();
        out.push(' ');
        out.push_str(remaining.trim_start());
        out
    } else {
        format!("{}{}", before, remaining)
    }
}

/// Scan text for `` close tags, tracking nesting depth.
/// Mimics `extract_think_blocks` stack logic — when both open and close
/// tags exist, processes the earlier one first. This prevents premature
/// close when LLM discusses `` within thinking content.
///
/// Returns (thinking_to_emit, remaining_text, final_depth).
fn scan_think_with_depth(text: &str, mut depth: u32) -> (Option<String>, String, u32) {
    let mut thinking = String::new();
    let mut search_pos = 0usize;

    while search_pos < text.len() && depth > 0 {
        let slice = &text[search_pos..];

        // Find next opening tag: `<think` (real tag only — literal "<think 标签" skipped)
        let next_open = find_think_open(slice);

        // Find next closing tag: `</think>` (supports whitespace before `>`)
        let next_close = close_tag_search(slice, "</think>");

        match (next_open, next_close) {
            (Some((o, open_len)), Some((c, _clen))) if o < c => {
                // Open comes first → depth++
                thinking.push_str(&slice[..o + open_len]);
                depth += 1;
                search_pos += o + open_len;
            }
            (_, Some((c, clen))) => {
                // Close comes first (or only close) → depth--
                thinking.push_str(&slice[..c]);
                depth -= 1;
                if depth == 0 {
                    let remaining = text[search_pos + c + clen..].to_string();
                    let reasoning = if thinking.is_empty() {
                        None
                    } else {
                        Some(thinking)
                    };
                    return (reasoning, remaining, 0);
                }
                search_pos += c + clen;
            }
            (Some((o, open_len)), None) => {
                // Only open tag, no close → everything is thinking
                thinking.push_str(&slice[..o]);
                depth += 1;
                thinking.push_str(&slice[o + open_len..]);
                search_pos = text.len();
            }
            (None, None) => {
                // No tags at all → everything is thinking
                thinking.push_str(slice);
                search_pos = text.len();
            }
        }
    }

    let remaining = if search_pos < text.len() {
        text[search_pos..].to_string()
    } else {
        String::new()
    };
    let reasoning = if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    };
    (reasoning, remaining, depth)
}

/// Resolve the Nuphus project root directory using a 4-level fallback strategy.
///
/// 1. `CARGO_MANIFEST_DIR` env var (set by cargo at compile time) — probe upward for Cargo.toml
/// 2. `current_exe` parent — probe upward for Cargo.toml
/// 3. `current_dir` — probe upward for Cargo.toml
/// 4. Fallback: from cwd probe upward for `.git` or `README.md`
///
/// Falls back to `current_dir` if nothing matches.
pub fn resolve_project_root() -> PathBuf {
    // 1. CARGO_MANIFEST_DIR: 编译时可用，向上探测找 Cargo.toml
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(dir);
        let mut probe = p.clone();
        for _ in 0..4 {
            if probe.join("Cargo.toml").exists() {
                return probe;
            }
            if let Some(parent) = probe.parent() {
                probe = parent.to_path_buf();
            } else {
                break;
            }
        }
        return p;
    }

    // 2. current_exe: 向上探测 6 层找 Cargo.toml
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.parent().unwrap_or(&exe).to_path_buf();
        for _ in 0..6 {
            if p.join("Cargo.toml").exists() {
                return p;
            }
            if let Some(parent) = p.parent() {
                p = parent.to_path_buf();
            } else {
                break;
            }
        }
    }

    // 3. cwd: 向上探测 6 层找 Cargo.toml
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut p = cwd.clone();
    for _ in 0..6 {
        if p.join("Cargo.toml").exists() {
            return p;
        }
        if let Some(parent) = p.parent() {
            p = parent.to_path_buf();
        } else {
            break;
        }
    }

    // 4. fallback: 从 cwd 向上找 .git 或 README.md 特征文件
    let mut p = cwd.clone();
    for _ in 0..6 {
        if p.join(".git").exists() || p.join("README.md").exists() {
            return p;
        }
        if let Some(parent) = p.parent() {
            p = parent.to_path_buf();
        } else {
            break;
        }
    }

    tracing::warn!(
        "[utils] could not resolve project root, falling back to current_dir: {:?}",
        cwd
    );
    cwd
}

/// Nuphus 用户数据目录——运行时数据（memory 快照、plan 文件等）的写入根目录。
///
/// 优先级：
/// 1. `NUPHUS_DATA_DIR` 环境变量显式覆盖
/// 2. `dirs::data_dir()/.nuphus`（Windows: `%APPDATA%\.nuphus`，macOS:
///    `~/Library/Application Support/.nuphus`，Linux: `~/.local/share/.nuphus`）——
///    始终指向用户可写目录
/// 3. 兜底 `resolve_project_root()/.nuphus`（`data_dir` 不可用等极端情况）
///
/// 与 `resolve_project_root()` 的区别：后者探测仓库/cwd，发布版会退化到
/// `current_dir`，安装到 Program Files 等受保护目录时写入会 Access Denied。
/// 所有运行时**写入**路径应统一走这里；`resolve_project_root` 保留给路径信任
/// 边界（workspace 内/外判定）等只读场景。
pub fn nuphus_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NUPHUS_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(data_dir) = dirs::data_dir() {
        return data_dir.join(".nuphus");
    }
    resolve_project_root().join(".nuphus")
}

/// Returns the workspace root directory (cross-platform: Linux/macOS/Windows).
///
/// Nuphus layout: `workspace_root/src/` (lib crate), `workspace_root/src-tauri/` (app).
/// `CARGO_MANIFEST_DIR` for the lib crate is .../src/, so `.parent()` is the workspace root.
/// Uses `Path::parent()` and `Path::join()` — no string concatenation, natively cross-platform.
pub fn workspace_root() -> PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| manifest.to_path_buf())
}

/// Safe Writer — wraps stderr + file, silently discards on write failure
struct SafeWriter {
    file: Option<std::fs::File>,
}

impl SafeWriter {
    fn new() -> Self {
        let log_path = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("nuphus-debug.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
        SafeWriter { file }
    }
}

impl std::io::Write for SafeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Write to stderr
        let _ = std::io::stderr().write(buf);
        // Also write to file
        if let Some(ref mut f) = self.file {
            let _ = f.write(buf);
            let _ = f.flush();
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
        Ok(())
    }
}

pub fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Registry};

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(SafeWriter::new)
        .with_target(false);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(false);

    Registry::default()
        .with(
            EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
                .add_directive("chromiumoxide=WARN".parse().unwrap()),
        )
        .with(file_layer)
        .with(stderr_layer)
        .init();
}
// ── 项目标签记忆日志（memory/{tag}.md）──
//
// 记忆按「Ctrl+K→项目配置」的 project_dir 派生标签分文件存储：
// 同项目跨会话共享、不同项目互不串扰。active_project_tag 为单一事实源
// （实时读配置），系统提示词的项目注入与记忆定位永远同源。

/// 当前生效的项目标签：实时从项目目录配置派生（空串视为未配置）。
pub fn active_project_tag() -> Option<String> {
    let dir = crate::config::UserPreferences::load().project_dir;
    if dir.trim().is_empty() {
        return None;
    }
    derive_project_tag_from_dir(&dir)
}

/// 标签清洗：保留字母/数字/下划线/连字符/CJK，空格折叠 '-'，空结果回退 default。
pub fn sanitize_memory_tag(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || !ch.is_ascii() {
            out.push(if ch == ' ' { '-' } else { ch });
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed
    }
}

/// 标签 → memory 文件路径；None → default。
pub fn memory_md_path(tag: Option<&str>) -> PathBuf {
    let t = tag
        .map(sanitize_memory_tag)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());
    nuphus_data_dir().join("memory").join(format!("{t}.md"))
}

/// 当前生效的记忆文件路径——注入与工具写盘统一入口。
pub fn active_memory_md_path() -> PathBuf {
    memory_md_path(active_project_tag().as_deref())
}

/// 由项目目录派生标签：目录名截 24 字符 + 路径 8 位哈希（同名不同路径不冲突）。
pub fn derive_project_tag_from_dir(dir: &str) -> Option<String> {
    if dir.trim().is_empty() {
        return None;
    }
    let name = std::path::Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())?;
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    Some(format!(
        "{}-{:08x}",
        name.chars().take(24).collect::<String>(),
        hasher.finish() as u32
    ))
}

/// 旧版单文件迁移：memory.md 内容拷为 default 标签（原文件保留）。幂等。
pub fn migrate_legacy_memory_md() {
    let legacy = nuphus_data_dir().join("memory.md");
    if !legacy.exists() {
        return;
    }
    let target = memory_md_path(Some("default"));
    if target.exists() {
        return;
    }
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(&legacy, &target) {
        Ok(_) => tracing::info!("[memory-tag] legacy memory.md migrated to default tag"),
        Err(e) => tracing::warn!("[memory-tag] migrate failed: {e}"),
    }
}

/// 记忆日志单文件容量上限：超出时从头丢弃最旧条目（整条目粒度）
pub const MEMORY_JOURNAL_CAP_BYTES: usize = 32 * 1024;

/// 列出**其它项目**的记忆日志路径（排除当前 active tag），
/// 供 L1 注入尾部构建跨项目索引——用户切换话题到其它项目时，
/// Leader 可直接 read 对应文件恢复项目感知，不因 tag 隔离而失联。
/// 返回 (tag, 绝对路径)，按文件修改时间新→旧排序。
pub fn other_project_memory_paths() -> Vec<(String, PathBuf)> {
    let dir = nuphus_data_dir().join("memory");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let current_tag = active_project_tag().unwrap_or_else(|| "default".to_string());
    let mut out: Vec<(String, PathBuf, std::time::SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".md")
        })
        .filter_map(|e| {
            let path = e.path();
            let tag = path.file_stem()?.to_string_lossy().to_string();
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((tag, path, mtime))
        })
        .filter(|(tag, _, _)| *tag != current_tag)
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out.into_iter().map(|(tag, path, _)| (tag, path)).collect()
}

/// 按条目切分日志（旧→新）。条目以 '[' 署名行起始、空行分隔；
/// 整条目粒度操作，杜绝 UTF-8 多字节中间截断。无署名头的旧整文件视为单条。
pub fn split_memory_journal(content: &str) -> Vec<&str> {
    let mut blocks: Vec<&str> = Vec::new();
    let len = content.len();
    let mut idx = 0usize;
    let mut start: Option<usize> = None;
    while idx < len {
        if content[idx..].starts_with('[') {
            if let Some(s) = start {
                blocks.push(content[s..idx].trim_end());
            }
            start = Some(idx);
        }
        match content[idx..].find('\n') {
            Some(nl) => idx += nl + 1,
            None => break,
        }
    }
    if let Some(s) = start {
        blocks.push(content[s..len].trim_end());
    }
    blocks.retain(|b| !b.trim().is_empty());
    blocks
}

/// 注入用：从最新（尾部）向前累计 ≤ max_chars 字符，按原时间序拼接。
pub fn memory_journal_tail(content: &str, max_chars: usize) -> String {
    let blocks = split_memory_journal(content);
    let mut picked: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for b in blocks.iter().rev() {
        let cost = b.chars().count() + 2;
        if used + cost > max_chars {
            break;
        }
        used += cost;
        picked.push(b);
    }
    picked.reverse();
    picked.join("\n\n")
}

/// 容量裁剪：超 cap_bytes 时从头丢弃最旧条目。
pub fn trim_memory_journal_to_cap(content: &str, cap_bytes: usize) -> String {
    if content.len() <= cap_bytes {
        return content.to_string();
    }
    let blocks = split_memory_journal(content);
    let mut kept: Vec<&str> = Vec::new();
    let mut total = 0usize;
    for b in blocks.iter().rev() {
        total += b.len() + 2;
        if total > cap_bytes {
            break;
        }
        kept.push(b);
    }
    kept.reverse();
    kept.join("\n\n")
}
#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_think_blocks ──────────────────────────────────────────

    #[test]
    fn normal_think_block() {
        let (clean, reasoning) =
            extract_think_blocks("Before <think>I am thinking</think> responseAfter");
        assert_eq!(clean, "Before responseAfter");
        assert_eq!(reasoning, "I am thinking");
    }

    #[test]
    fn no_think_block() {
        let (clean, reasoning) = extract_think_blocks("Plain text");
        assert_eq!(clean, "Plain text");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn multiple_think_blocks() {
        let (clean, reasoning) =
            extract_think_blocks("A <think>first</think> <think>second</think>Finally");
        assert_eq!(clean, "A Finally");
        assert_eq!(reasoning, "firstsecond");
    }

    #[test]
    fn nested_think_block() {
        // Model discusses think tags inside reasoning
        let (clean, reasoning) = extract_think_blocks(
            "<think>outer model says <think>inner</think> message outer</think> Done",
        );
        // The inner <think> block should nest, everything between outer <think> is reasoning
        assert_eq!(clean, "Done");
        assert!(reasoning.contains("model says"));
        assert!(reasoning.contains("inner"));
        // The inner tags shouldn't appear in clean
        assert!(!reasoning.contains("</think>"));
    }

    #[test]
    fn close_tag_with_whitespace() {
        // Model outputs </think > with a space before >
        let (clean, reasoning) =
            extract_think_blocks("Before <think>I am thinking</think > responseAfter");
        assert_eq!(clean, "Before responseAfter");
        assert_eq!(reasoning, "I am thinking");
    }

    #[test]
    fn close_tag_with_multiple_spaces() {
        let (clean, reasoning) =
            extract_think_blocks("Before <think>I am thinking</think  > responseAfter");
        assert_eq!(clean, "Before responseAfter");
        assert_eq!(reasoning, "I am thinking");
    }

    #[test]
    fn bare_thinking_keyword_not_tag() {
        // "thinking" without angle brackets should pass through
        let (clean, reasoning) =
            extract_think_blocks("I am thinking about this. It makes me think.");
        assert_eq!(clean, "I am thinking about this. It makes me think.");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn orphaned_close_tag() {
        // Close tag without open tag (cross-chunk artifact)
        let (clean, reasoning) = extract_think_blocks("Some text </think> without think block");
        assert_eq!(clean, "Some text without think block");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn partial_close_tag_fragment() {
        // Cross-chunk fragment: </think (no >)
        let (clean, reasoning) = extract_think_blocks("Text with broken tag </think");
        assert_eq!(clean, "Text with broken tag");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn unclosed_think_tag() {
        // Opening <think> without closing — treats as reasoning
        let (clean, reasoning) = extract_think_blocks("Intro <think>unfinished");
        assert_eq!(clean, "Intro ");
        assert_eq!(reasoning, "unfinished");
    }

    #[test]
    fn empty_think_block() {
        let (clean, reasoning) = extract_think_blocks("Before <think></think>");
        assert_eq!(clean, "Before ");
        assert_eq!(reasoning, "");
    }

    // ── clean_think_remnants ──────────────────────────────────────────

    #[test]
    fn clean_removes_full_close_tag() {
        assert_eq!(clean_think_remnants("text</think>"), "text");
    }

    #[test]
    fn clean_removes_close_with_space() {
        assert_eq!(clean_think_remnants("text</think >"), "text");
    }

    #[test]
    fn clean_removes_partial_close() {
        assert_eq!(clean_think_remnants("text</think"), "text");
    }

    #[test]
    fn clean_removes_invoke_tags() {
        assert_eq!(clean_think_remnants("a</invoke>b</parameter>c"), "abc");
    }

    // ── self-reference 防护：正文讨论 <think 字面量不截断 ──────────────

    #[test]
    fn literal_think_text_not_truncated() {
        // 回复正文中出现 "剥离 <think 标签" 这类字面量（不以 > 结尾），
        // 不能把后续所有内容误吞进 reasoning —— 曾经导致消息在 "剥离 <think" 处截断。
        let input = "完整证据链：为什么今天之前从未出现\n剥离 <think 标签时后续内容必须保留";
        let (clean, reasoning) = extract_think_blocks(input);
        assert_eq!(clean, input);
        assert_eq!(reasoning, "");
    }

    #[test]
    fn literal_think_with_real_block() {
        // 既有字面量又有真实 think 块：字面量保留、真实块剥离
        let input = "正文讨论 <think 字面量 <think>真实思考</think> 后续正文";
        let (clean, reasoning) = extract_think_blocks(input);
        assert_eq!(clean, "正文讨论 <think 字面量 后续正文");
        assert_eq!(reasoning, "真实思考");
    }

    #[test]
    fn process_delta_literal_think_not_truncated() {
        use std::sync::atomic::AtomicU32;
        let depth = AtomicU32::new(0);
        let (reasoning, text_out) =
            process_text_delta("剥离 <think 标签时后续内容必须保留", &depth);
        assert!(reasoning.is_none());
        assert_eq!(text_out, "剥离 <think 标签时后续内容必须保留");
        assert_eq!(depth.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    // ── 标签闭合：think 块内讨论 <think 字面量不应破坏闭合 ──────────────

    #[test]
    fn nested_literal_think_keeps_close() {
        // think 块内部讨论 "<think 标签"（不以 > 结尾）：不能被当作嵌套 open，
        // 否则栈永不归零，close 之后的正文本会被吞进 reasoning。
        let input = "前文 <think>思考：不要剥离 <think 标签 否则坏</think> 后文";
        let (clean, reasoning) = extract_think_blocks(input);
        assert_eq!(clean, "前文 后文");
        assert!(reasoning.contains("思考：不要剥离 <think 标签 否则坏"));
        assert!(!reasoning.contains("后文"));
    }

    #[test]
    fn process_delta_nested_literal_think_keeps_close() {
        use std::sync::atomic::AtomicU32;
        let depth = AtomicU32::new(1); // 已在 think 块内（跨 chunk 场景）
        let (reasoning, text_out) =
            process_text_delta("思考：不要剥离 <think 标签 否则坏</think> 后文", &depth);
        assert!(reasoning.is_some());
        let r = reasoning.unwrap();
        assert!(r.contains("思考：不要剥离 <think 标签 否则坏"));
        assert_eq!(text_out.trim(), "后文");
        assert_eq!(depth.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}