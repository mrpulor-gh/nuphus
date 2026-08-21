//! 从 .md 文件中提取 title、tags、body_text。
//!
//! 支持：
//! - 带 YAML frontmatter 的文档（提取 title、tags、正文）
//! - 纯 markdown 文档（从文件名取 title）

use crate::types::FileMeta;
use std::fs;
use std::path::Path;

/// 解析一个 .md 文件，返回 FileMeta
pub fn parse_md_file(rel_path: &str, abs_path: &Path) -> Option<FileMeta> {
    let content = fs::read_to_string(abs_path).ok()?;
    let metadata = fs::metadata(abs_path).ok()?;

    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_size = metadata.len();

    // 提取 frontmatter
    let (frontmatter, body_start) = extract_frontmatter(&content);
    let body = &content[body_start..];

    let title = frontmatter
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.clone())
        .or_else(|| {
            Path::new(rel_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "untitled".to_string());

    let tags = parse_tags(&frontmatter);

    Some(FileMeta {
        rel_path: rel_path.to_string(),
        title,
        tags,
        file_mtime,
        file_size,
        body_text: body.trim().to_string(),
        embedding: None,
    })
}

// ── Frontmatter 提取 ──

/// 提取 frontmatter KV 对，返回 (键值对列表, 正文起始位置)
fn extract_frontmatter(content: &str) -> (Vec<(String, String)>, usize) {
    let s = content.trim_start();
    if !s.starts_with("---") {
        return (vec![], 0);
    }

    // 找第二个 ---
    let search_start = 3; // 跳过第一个 ---
    if let Some(end) = s[search_start..].find("\n---") {
        let fm_end = search_start + end;
        let fm_text = &s[search_start..fm_end];
        let body_start = fm_end + 5; // 跳过 \n---\n (4) 或 \n---\r\n (5)

        // 处理 \r\n vs \n
        let body_start = if body_start < s.len() && s.as_bytes().get(body_start - 1) == Some(&b'\r')
        {
            body_start + 1
        } else {
            body_start
        };

        let mut kv = Vec::new();
        for line in fm_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(idx) = line.find(':') {
                let key = line[..idx].trim().to_lowercase();
                let value = line[idx + 1..].trim().to_string();
                if !key.is_empty() {
                    kv.push((key, value));
                }
            }
        }
        (kv, body_start)
    } else {
        (vec![], 0)
    }
}

/// 解析 YAML 列表格式 [a, b, c] 或 "a"
fn parse_yaml_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        s[1..s.len() - 1]
            .split(',')
            .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if !s.is_empty() {
        vec![s.trim_matches('"').trim_matches('\'').to_string()]
    } else {
        vec![]
    }
}

/// 从 frontmatter 的 tags 字段提取标签
fn parse_tags(frontmatter: &[(String, String)]) -> Vec<String> {
    frontmatter
        .iter()
        .find(|(k, _)| k == "tags")
        .map(|(_, v)| {
            parse_yaml_list(v)
                .into_iter()
                .map(|t| t.trim_start_matches('#').to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_frontmatter_kv() {
        let content = "---\ntitle: 测试文档\ntags: [rust, test]\n---\n\n# 标题\n正文内容";
        let (fm, body_start) = extract_frontmatter(content);
        assert_eq!(fm.len(), 2);
        assert_eq!(fm[0], ("title".to_string(), "测试文档".to_string()));
        assert_eq!(fm[1], ("tags".to_string(), "[rust, test]".to_string()));
        assert!(body_start > 0);
        assert!(content[body_start..].contains("正文内容"));
    }

    #[test]
    fn test_parse_yaml_list() {
        assert_eq!(parse_yaml_list("[a, b, c]"), vec!["a", "b", "c"]);
        assert_eq!(parse_yaml_list("单一值"), vec!["单一值"]);
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "# 纯文档\n\n正文内容";
        let (fm, body_start) = extract_frontmatter(content);
        assert!(fm.is_empty());
        assert_eq!(body_start, 0);
    }

    #[test]
    fn test_frontmatter_with_extra_spaces() {
        let content = "---\ntitle: 我的文档\ntags: [tag1, tag2]\n---\n\n正文";
        let (fm, _) = extract_frontmatter(content);
        assert_eq!(fm.len(), 2);
        let tags = parse_tags(&fm);
        assert_eq!(tags, vec!["tag1", "tag2"]);
    }
}
