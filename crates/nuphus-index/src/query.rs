//! 知识文档搜索逻辑。
//!
//! 在内存索引的 HashMap 上做多维匹配 + 语义向量检索，按相关性评分排序。
//! 搜索字段：标题、标签、正文全文、语义向量。

use crate::types::{FileMeta, KnowledgeHit, QueryRequest};
use std::collections::HashMap;

/// 在索引中搜索，返回按相关性排序的结果
pub fn search_index(files: &HashMap<String, FileMeta>, req: &QueryRequest) -> Vec<KnowledgeHit> {
    // 1. 按标签预过滤
    let pre_filtered: Vec<&FileMeta> = if req.tags.is_empty() {
        files.values().collect()
    } else {
        files
            .values()
            .filter(|f| {
                req.tags
                    .iter()
                    .any(|t| f.tags.iter().any(|ft| ft.eq_ignore_ascii_case(t)))
            })
            .collect()
    };

    // 2. 按关键词评分
    if req.query.trim().is_empty() {
        let mut results: Vec<KnowledgeHit> = pre_filtered.into_iter().map(|f| f.into()).collect();
        results.sort_by(|a, b| a.title.cmp(&b.title));
        results.truncate(req.max_results);
        return results;
    }

    let query_lower = req.query.to_lowercase();
    let mut scored: Vec<(f32, &FileMeta)> = pre_filtered
        .into_iter()
        .filter_map(|f| {
            let score = calc_relevance(f, &query_lower);
            if score > 0.0 {
                Some((score, f))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(req.max_results)
        .map(|(score, f)| {
            let mut hit = KnowledgeHit::from(f);
            hit.score = score;
            hit
        })
        .collect()
}

/// 计算单个文件与查询词的相关性分数
fn calc_relevance(file: &FileMeta, query: &str) -> f32 {
    let mut score = 0.0f32;

    // 标题匹配（最高权重）
    if file.title.to_lowercase().contains(query) {
        score += 3.0;
    }

    // 标签匹配
    for tag in &file.tags {
        if tag.to_lowercase().contains(query) {
            score += 2.0;
        }
    }

    // 正文全文匹配（核心改进：不再只搜 snippet，而是搜全部正文）
    if file.body_text.to_lowercase().contains(query) {
        // 正文匹配权重较低，但多次出现可叠加
        let count = file.body_text.to_lowercase().matches(query).count().min(10); // 上限 10 次
        score += count as f32 * 0.3;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileMeta;

    fn make_file(rel_path: &str, title: &str, tags: Vec<&str>, body: &str) -> FileMeta {
        FileMeta {
            rel_path: rel_path.to_string(),
            title: title.to_string(),
            tags: tags.into_iter().map(|s| s.to_string()).collect(),
            file_mtime: 0,
            file_size: 0,
            body_text: body.to_string(),
            embedding: None,
        }
    }

    #[test]
    fn test_search_matches_title() {
        let mut files = HashMap::new();
        files.insert(
            "a.md".to_string(),
            make_file("a.md", "PowerShell 入门", vec!["powershell"], "内容"),
        );
        files.insert(
            "b.md".to_string(),
            make_file("b.md", "Python 基础", vec!["python"], "内容"),
        );

        let req = QueryRequest {
            query: "PowerShell".to_string(),
            max_results: 10,
            ..Default::default()
        };
        let results = search_index(&files, &req);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "PowerShell 入门");
    }

    #[test]
    fn test_search_matches_body() {
        let mut files = HashMap::new();
        files.insert(
            "a.md".to_string(),
            make_file(
                "a.md",
                "日常技巧",
                vec![],
                "ForEach-Object 循环可以并行处理大量文件",
            ),
        );
        files.insert(
            "b.md".to_string(),
            make_file("b.md", "无关文档", vec![], "今天天气很好"),
        );

        let req = QueryRequest {
            query: "并行处理".to_string(),
            max_results: 10,
            ..Default::default()
        };
        let results = search_index(&files, &req);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "日常技巧");
    }

    #[test]
    fn test_empty_query_returns_all() {
        let mut files = HashMap::new();
        files.insert(
            "a.md".to_string(),
            make_file("a.md", "文档A", vec![], "body"),
        );
        files.insert(
            "b.md".to_string(),
            make_file("b.md", "文档B", vec![], "body"),
        );

        let req = QueryRequest {
            query: String::new(),
            max_results: 10,
            ..Default::default()
        };
        let results = search_index(&files, &req);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_tag_filter_plus_query() {
        let mut files = HashMap::new();
        files.insert(
            "a.md".to_string(),
            make_file("a.md", "PowerShell入门", vec!["powershell"], "基础内容"),
        );
        files.insert(
            "b.md".to_string(),
            make_file("b.md", "Python入门", vec!["python"], "基础内容"),
        );

        let req = QueryRequest {
            query: "入门".to_string(),
            tags: vec!["powershell".to_string()],
            max_results: 10,
        };
        let results = search_index(&files, &req);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "PowerShell入门");
    }
}
