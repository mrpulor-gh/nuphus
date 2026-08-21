//! Tech-word post-correction for STT output.
//!
//! Closed-vocabulary fuzzy replacement (report-live.md §5, item 2):
//! sense-voice greedy decoding has no hotword support, so accented English
//! tech words come out mis-spelled ("typecript", "COMMET"). ASCII tokens
//! within edit distance of a dictionary entry are replaced by the canonical
//! form. CJK text is never touched.
//!
//! The dictionary is a static list here (compiled in, zero I/O on the hot
//! path); an optional `tech-dict.txt` (one word per line, `#` comments)
//! in the STT model dir is merged at load time for easy extension.

use std::sync::OnceLock;

/// Canonical dictionary entries — first form is the replacement casing.
/// Keep aligned with tools/stt-proto/eval_live.py TECH list for regression parity.
const DEFAULT_DICT: &[&str] = &[
    // Nuphus domain verbs / nouns (prototype evidence: report-live.md §2)
    "dispatch",
    "commit",
    "build",
    "merge",
    "deploy",
    "workflow",
    "typescript",
    "api",
    "token",
    "rust",
    "mcp",
    "sherpa-onnx",
    "sherpa",
    "onnx",
    "nuphus",
    // Extended closed vocab (~50 total): agent / dev-tooling terms
    "agent",
    "subagent",
    "exec",
    "leader",
    "session",
    "context",
    "prompt",
    "frontend",
    "backend",
    "database",
    "server",
    "client",
    "docker",
    "kubernetes",
    "git",
    "github",
    "gitlab",
    "branch",
    "release",
    "debug",
    "compile",
    "runtime",
    "config",
    "json",
    "yaml",
    "toml",
    "http",
    "https",
    "url",
    "sdk",
    "cli",
    "gui",
    "css",
    "html",
    "javascript",
    "python",
    "java",
    "golang",
    "linux",
    "windows",
    "macos",
    "cpu",
    "gpu",
    "memory",
    "cache",
    "mutex",
    "async",
    "sync",
    "vite",
    "react",
    "tauri",
    "cargo",
    "npm",
    "ffmpeg",
    "websocket",
    "oauth",
    "jwt",
    "ssh",
    "localhost",
];

static DICT: OnceLock<Vec<String>> = OnceLock::new();

fn dict_words() -> &'static [String] {
    DICT.get_or_init(|| {
        let mut words: Vec<String> = DEFAULT_DICT.iter().map(|s| s.to_string()).collect();
        // Optional user extension file next to the models.
        if let Ok(paths) = super::engine::resolve_stt_paths() {
            let ext = paths.dir.join("tech-dict.txt");
            if let Ok(content) = std::fs::read_to_string(&ext) {
                for line in content.lines() {
                    let w = line.trim();
                    if w.is_empty() || w.starts_with('#') {
                        continue;
                    }
                    let lw = w.to_lowercase();
                    if !words.iter().any(|e| e.to_lowercase() == lw) {
                        words.push(w.to_string());
                    }
                }
                tracing::info!(
                    "[stt] tech-dict: {} entries (incl. {})",
                    words.len(),
                    ext.display()
                );
            }
        }
        words
    })
}

/// Correct tech words in recognized text. Idempotent, CJK-safe.
pub fn correct(text: &str) -> String {
    if !text.is_ascii() && !text.bytes().any(|b| b.is_ascii_alphabetic()) {
        return text.to_string();
    }
    let dict = dict_words();
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
            token.push(ch);
        } else {
            flush_token(&mut token, &mut out, dict);
            out.push(ch);
        }
    }
    flush_token(&mut token, &mut out, dict);
    out
}

fn flush_token(token: &mut String, out: &mut String, dict: &[String]) {
    if token.is_empty() {
        return;
    }
    out.push_str(&fix_token(token, dict));
    token.clear();
}

/// Fuzzy-match one ASCII token against the dictionary.
/// Threshold mirrors eval_live.py: dist<=1 for words of len<=4, else dist<=2,
/// plus a same-first-letter guard against cross-word false positives.
fn fix_token(token: &str, dict: &[String]) -> String {
    if token.len() < 3 || !token.bytes().any(|b| b.is_ascii_alphabetic()) {
        return token.to_string();
    }
    let w = token.to_lowercase();
    // Exact match (case-insensitive) → canonical casing.
    for t in dict {
        if w == t.to_lowercase() {
            return t.clone();
        }
    }
    let first = w.as_bytes()[0];
    let mut best: Option<&String> = None;
    let mut best_d = usize::MAX;
    for t in dict {
        let tl = t.to_lowercase();
        if tl.as_bytes().first().copied() != Some(first) {
            continue;
        }
        let thr = if tl.len() <= 4 { 1 } else { 2 };
        let d = levenshtein(&w, &tl);
        if d <= thr && d < best_d {
            best = Some(t);
            best_d = d;
        }
    }
    match best {
        Some(t) => t.clone(),
        None => token.to_string(),
    }
}

/// Classic Levenshtein over bytes (dictionary entries are ASCII).
/// prev[j] == lev(long[..i], short[..j]); two-row DP.
fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut prev: Vec<usize> = (0..=short.len()).collect();
    let mut cur: Vec<usize> = vec![0; short.len() + 1];
    for (i, &cb) in long.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &ca) in short.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1) // skip long char
                .min(cur[j] + 1) // skip short char
                .min(prev[j] + usize::from(ca != cb)); // match / substitute
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[short.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_sanity() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("ab", "a"), 1);
        assert_eq!(levenshtein("a", "ab"), 1);
        assert_eq!(levenshtein("typecript", "typescript"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("hello", "world"), 4);
    }

    #[test]
    fn exact_and_fuzzy() {
        // eval_live.py parity cases from report-live.md §2
        assert_eq!(correct("typecript"), "typescript");
        assert_eq!(correct("COMMET"), "commit");
        assert_eq!(correct("api"), "api");
        assert_eq!(correct("mcp"), "mcp");
        assert_eq!(correct("rust"), "rust");
        assert_eq!(correct("build"), "build");
    }

    #[test]
    fn cjk_untouched() {
        assert_eq!(
            correct("帮我查一下昨天的构建日志"),
            "帮我查一下昨天的构建日志"
        );
        assert_eq!(
            correct("用 typecript重写这个组件"),
            "用 typescript重写这个组件"
        );
    }

    #[test]
    fn unrelated_words_untouched() {
        // Distance > threshold, must not be mangled.
        assert_eq!(correct("hello world"), "hello world");
        assert_eq!(correct("把这个文件复制到桌面"), "把这个文件复制到桌面");
    }

    #[test]
    fn hyphenated_dict_word() {
        assert_eq!(correct("sherpa-onnx"), "sherpa-onnx");
        assert_eq!(correct("sherpa-onx"), "sherpa-onnx");
    }
}
