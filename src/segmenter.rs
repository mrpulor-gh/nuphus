//! Unified segmentation module
//!
//! Uses jieba for Chinese, splits by space/punctuation for English.
//! Replaces all previous n-gram tokenize implementations, eliminating CJK search noise.

use std::sync::OnceLock;

static JIEBA: OnceLock<jieba_rs::Jieba> = OnceLock::new();

fn jieba() -> &'static jieba_rs::Jieba {
    JIEBA.get_or_init(|| {
        tracing::info!("[Segmenter] initializing jieba tokenizer");
        jieba_rs::Jieba::new()
    })
}

/// Check if text contains Chinese characters
fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
            | '\u{3400}'..='\u{4DBF}' // CJK Extension A
            | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        )
    })
}

/// 停用词：区分度为零的虚词/功能词。
/// 这些词若参与 FTS 严格 AND，会把自然语言查询直接打成零结果
///（实证：「白名单是怎么实现的」→ 5 token AND → 0 命中，单词各自都有命中）。
/// 单字虚词（的/了/是/吗…）由「字符数 ≥2」规则拦截，此表只收多字词。
const STOPWORDS: &[&str] = &[
    // 中文多字虚词
    "怎么",
    "怎样",
    "什么",
    "为什么",
    "为何",
    "如何",
    "是否",
    "是不是",
    "有没有",
    "以及",
    "可以",
    "能不能",
    "能否",
    "请问",
    "一下",
    "一些",
    "哪个",
    "哪些",
    "怎么办",
    // 英文功能词
    "the",
    "an",
    "is",
    "are",
    "was",
    "were",
    "do",
    "does",
    "did",
    "how",
    "what",
    "why",
    "can",
    "could",
    "please",
    "me",
    "my",
    "you",
    "your",
    "we",
    "our",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "there",
    "here",
    "be",
    "been",
    "to",
    "of",
    "for",
    "with",
    "and",
    "or",
];

fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w)
}

/// 尾随黏着虚字：jieba 会把「的/了/吗」等黏到前一个词尾（移动端的→[移动,端的]），
/// 产生只在查询侧出现的垃圾 token，严格 AND 下必零结果。
/// 剥掉尾字后：≥2 字保留（白名单的→白名单），剩 1 字由字符数规则丢弃（端的→端→丢弃）。
/// 索引/查询两侧对称处理，对齐一致。已知代价：「目的」这类真词会被误剥——
/// 词尾为虚字的 2 字真词极少，收益远大于损失。
const TRAILING_GLUE: &[char] = &['的', '了', '吗', '呢', '吧', '啊'];

fn strip_trailing_glue(w: &str) -> String {
    let mut s = w.to_string();
    while s.chars().count() >= 2
        && s.chars()
            .last()
            .map(|c| TRAILING_GLUE.contains(&c))
            .unwrap_or(false)
    {
        s.pop();
    }
    s
}

/// Segment text into a list of independent tokens, suitable for BM25/keyword matching.
///
/// - Uses jieba for Chinese text
/// - Splits by non-alphanumeric characters for pure English/numeric text
/// - Filters single-character words（按字符数判定，非字节数——汉字 3 字节，
///   用 `len()` 会让「的/了/是」全部漏网）与停用词
pub fn segment(text: &str) -> Vec<String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return vec![];
    }

    if has_cjk(&text) {
        let jieba = jieba();
        let words = jieba.cut(&text, true); // true = HMM mode
        words
            .into_iter()
            .map(|w| strip_trailing_glue(&w.to_lowercase()))
            .filter(|w| w.chars().count() >= 2 && !is_stopword(w))
            .collect()
    } else {
        text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .map(|w| w.trim().to_lowercase())
            .filter(|w| w.len() >= 2 && !is_stopword(w))
            .collect()
    }
}

/// Segment text and join with spaces, suitable for FTS5 storage.
///
/// FTS5's unicode61 tokenizer splits each Chinese character into individual tokens,
/// after segmentation and joining with spaces, "网络问题" → "网络 问题" → FTS5 correctly indexes as two words.
pub fn segment_for_fts(text: &str) -> String {
    segment(text).join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_english() {
        let tokens = segment("hello world test");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
    }

    #[test]
    fn test_segment_chinese() {
        let tokens = segment("网络问题");
        // jieba should split into "网络" and "问题"
        assert!(tokens.contains(&"网络".to_string()), "tokens: {:?}", tokens);
        assert!(tokens.contains(&"问题".to_string()), "tokens: {:?}", tokens);
    }

    #[test]
    fn test_segment_for_fts() {
        let result = segment_for_fts("网络问题");
        assert_eq!(result, "网络  问题");
    }

    #[test]
    fn test_has_cjk() {
        assert!(has_cjk("网络问题"));
        assert!(has_cjk("hello 世界"));
        assert!(!has_cjk("hello world"));
        assert!(!has_cjk("test123"));
    }

    /// P0 回归：单字虚词（是/的/了）与多字停用词（怎么）必须被过滤，
    /// 核心词保留——否则严格 AND 把自然句式查询打成零结果
    #[test]
    fn test_segment_filters_function_words() {
        let tokens = segment("白名单是怎么实现的");
        assert!(
            tokens.contains(&"白名单".to_string()),
            "tokens: {:?}",
            tokens
        );
        assert!(tokens.contains(&"实现".to_string()), "tokens: {:?}", tokens);
        for w in ["是", "的", "怎么"] {
            assert!(
                !tokens.contains(&w.to_string()),
                "应被过滤: {} in {:?}",
                w,
                tokens
            );
        }
    }

    #[test]
    fn test_segment_english_stopwords() {
        let tokens = segment("how to fix the bug");
        assert_eq!(tokens, vec!["fix".to_string(), "bug".to_string()]);
    }

    /// 纯停用词查询 → 空 token 列表（调用方走「无有效关键词」诊断）
    #[test]
    fn test_segment_stopword_only_query() {
        assert!(segment("怎么可以").is_empty());
    }

    /// P0 回归：jieba 黏着虚字必须剥离——「移动端的」切出垃圾 token「端的」，
    /// 剥离后剩单字被丢弃；「白名单的」剥为「白名单」保留
    #[test]
    fn test_segment_strips_trailing_glue() {
        let tokens = segment("移动端的回退修复了吗");
        assert_eq!(
            tokens,
            vec!["移动".to_string(), "回退".to_string(), "修复".to_string()],
            "黏着虚字未剥离: {:?}",
            tokens
        );
        let tokens2 = segment("白名单的实现");
        assert!(
            tokens2.contains(&"白名单".to_string()),
            "tokens2: {:?}",
            tokens2
        );
    }
}
