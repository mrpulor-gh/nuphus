//! 工具输出过滤器执行引擎

use super::rule::{FilterRule, RuleFilters, RuleTransforms};

/// 工具输出过滤器 — 在工具结果进入 Session 前进行轻量预处理
pub struct ToolOutputFilter {
    builtin_rules: Vec<FilterRule>,
}

impl ToolOutputFilter {
    /// 创建过滤器，加载内置规则
    pub fn new() -> Self {
        Self {
            builtin_rules: Self::builtin_rules(),
        }
    }

    /// 对工具输出应用匹配的规则（静态方法，自动创建临时过滤器）
    pub fn apply(tool_name: &str, content: &str) -> String {
        let filter = Self::new();
        let mut result = content.to_string();

        for rule in &filter.builtin_rules {
            if !rule.matches_tool(tool_name) {
                continue;
            }
            if !rule.matches_content(&result) {
                continue;
            }

            // 应用 filters
            if let Some(ref filters) = rule.filters {
                result = apply_filters(&result, filters);
            }

            // 应用 transforms
            if let Some(ref transforms) = rule.transforms {
                result = apply_transforms(&result, transforms);
            }
        }

        result
    }

    /// 内置规则预设
    fn builtin_rules() -> Vec<FilterRule> {
        vec![
            FilterRule {
                id: "file_read/default".to_string(),
                description: Some("file_read 输出截断".to_string()),
                r#match: super::rule::RuleMatch {
                    tool_names: vec!["Read".to_string()],
                    content_match: None,
                },
                filters: None,
                transforms: Some(RuleTransforms {
                    strip_ansi: false,
                    trim_empty_edges: true,
                    dedupe_adjacent: false,
                    max_lines: Some(200),
                    head_lines: Some(50),
                    tail_lines: Some(20),
                }),
            },
            FilterRule {
                id: "system_shell/default".to_string(),
                description: Some("system_shell 输出清理".to_string()),
                r#match: super::rule::RuleMatch {
                    tool_names: vec!["system_shell".to_string()],
                    content_match: None,
                },
                filters: None,
                transforms: Some(RuleTransforms {
                    strip_ansi: true,
                    trim_empty_edges: true,
                    dedupe_adjacent: false,
                    max_lines: Some(500),
                    head_lines: None,
                    tail_lines: None,
                }),
            },
            FilterRule {
                id: "search_grep/default".to_string(),
                description: Some("search_grep 输出去重截断".to_string()),
                r#match: super::rule::RuleMatch {
                    tool_names: vec!["Grep".to_string()],
                    content_match: None,
                },
                filters: None,
                transforms: Some(RuleTransforms {
                    strip_ansi: false,
                    trim_empty_edges: false,
                    dedupe_adjacent: true,
                    max_lines: Some(200),
                    head_lines: None,
                    tail_lines: None,
                }),
            },
            FilterRule {
                id: "desktop_vision/default".to_string(),
                description: Some("desktop_vision 输出清理".to_string()),
                r#match: super::rule::RuleMatch {
                    tool_names: vec!["desktop_vision".to_string()],
                    content_match: None,
                },
                filters: None,
                transforms: Some(RuleTransforms {
                    strip_ansi: false,
                    trim_empty_edges: true,
                    dedupe_adjacent: false,
                    max_lines: None,
                    head_lines: None,
                    tail_lines: None,
                }),
            },
            FilterRule {
                id: "default/catch_all".to_string(),
                description: Some("默认规则：去除 ANSI + 修剪空行".to_string()),
                r#match: super::rule::RuleMatch {
                    tool_names: vec!["*".to_string()],
                    content_match: None,
                },
                filters: None,
                transforms: Some(RuleTransforms {
                    strip_ansi: true,
                    trim_empty_edges: true,
                    dedupe_adjacent: false,
                    max_lines: None,
                    head_lines: None,
                    tail_lines: None,
                }),
            },
        ]
    }
}

impl Default for ToolOutputFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// 应用过滤操作
fn apply_filters(content: &str, filters: &RuleFilters) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = Vec::new();

    for line in lines {
        // keep_patterns 优先级高于 skip_patterns
        if !filters.keep_patterns.is_empty() {
            let should_keep = filters.keep_patterns.iter().any(|p| line.contains(p));
            if should_keep {
                result.push(line.to_string());
            }
            continue;
        }

        // skip_patterns
        let should_skip = filters.skip_patterns.iter().any(|p| line.contains(p));
        if !should_skip {
            result.push(line.to_string());
        }
    }

    result.join("\n")
}

/// 应用变换操作
fn apply_transforms(content: &str, transforms: &RuleTransforms) -> String {
    let mut result = content.to_string();

    // 1. 去除 ANSI 转义序列
    if transforms.strip_ansi {
        result = strip_ansi_codes(&result);
    }

    // 2. 按行处理
    let mut lines: Vec<String> = result.lines().map(|s| s.to_string()).collect();

    // 3. 去除首尾空行
    if transforms.trim_empty_edges {
        while let Some(first) = lines.first() {
            if first.trim().is_empty() {
                lines.remove(0);
            } else {
                break;
            }
        }
        while let Some(last) = lines.last() {
            if last.trim().is_empty() {
                lines.pop();
            } else {
                break;
            }
        }
    }

    // 4. 合并相邻重复行
    if transforms.dedupe_adjacent && !lines.is_empty() {
        let mut deduped = vec![lines[0].clone()];
        for line in lines.into_iter().skip(1) {
            if line != deduped[deduped.len() - 1] {
                deduped.push(line);
            }
        }
        lines = deduped;
    }

    // 5. head/tail 截断
    if let (Some(head), Some(tail)) = (transforms.head_lines, transforms.tail_lines) {
        if lines.len() > head + tail {
            let mut truncated = Vec::new();
            truncated.extend(lines.iter().take(head).cloned());
            truncated.push(format!("[... {} 行已截断 ...]", lines.len() - head - tail));
            truncated.extend(lines.iter().skip(lines.len() - tail).cloned());
            lines = truncated;
        }
    }

    // 6. max_lines 截断（优先级低于 head/tail）
    if let Some(max) = transforms.max_lines {
        if lines.len() > max {
            let original_len = lines.len();
            let mut truncated: Vec<String> = lines.into_iter().take(max).collect();
            truncated.push(format!("[输出已截断，原始 {} 行]", original_len));
            lines = truncated;
        }
    }

    lines.join("\n")
}

/// 去除 ANSI 转义序列
fn strip_ansi_codes(s: &str) -> String {
    // 匹配 \x1b[...m 形式的 ANSI 序列
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").expect("static ANSI regex pattern");
    re.replace_all(s, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi() {
        let input = "\x1b[31mred\x1b[0m text";
        assert_eq!(strip_ansi_codes(input), "red text");
    }

    #[test]
    fn test_apply_filters_skip() {
        let filters = RuleFilters {
            skip_patterns: vec!["debug".to_string()],
            keep_patterns: vec![],
        };
        let input = "line1\ndebug: info\nline3";
        assert_eq!(apply_filters(input, &filters), "line1\nline3");
    }

    #[test]
    fn test_apply_filters_keep() {
        let filters = RuleFilters {
            skip_patterns: vec![],
            keep_patterns: vec!["ERROR".to_string()],
        };
        let input = "INFO: x\nERROR: y\nINFO: z";
        assert_eq!(apply_filters(input, &filters), "ERROR: y");
    }

    #[test]
    fn test_apply_transforms_head_tail() {
        let transforms = RuleTransforms {
            head_lines: Some(2),
            tail_lines: Some(1),
            ..Default::default()
        };
        let input = "a\nb\nc\nd\ne";
        let result = apply_transforms(input, &transforms);
        assert!(result.contains("a"));
        assert!(result.contains("b"));
        assert!(result.contains("e"));
        assert!(!result.contains("c"));
        assert!(result.contains("已截断"));
    }

    #[test]
    fn test_apply_transforms_dedupe() {
        let transforms = RuleTransforms {
            dedupe_adjacent: true,
            ..Default::default()
        };
        let input = "a\na\nb\nb\na";
        assert_eq!(apply_transforms(input, &transforms), "a\nb\na");
    }

    #[test]
    fn test_apply_transforms_trim_edges() {
        let transforms = RuleTransforms {
            trim_empty_edges: true,
            ..Default::default()
        };
        let input = "\n\nhello\n\n";
        assert_eq!(apply_transforms(input, &transforms), "hello");
    }

    #[test]
    fn test_tool_output_filter_file_read() {
        let lines: Vec<String> = (0..250).map(|i| format!("line {}", i)).collect();
        let input = lines.join("\n");
        let result = ToolOutputFilter::apply("Read", &input);
        assert!(result.contains("line 0"));
        assert!(result.contains("line 49")); // head 50
        assert!(result.contains("line 230")); // tail 20 starts at 230
        assert!(result.contains("已截断"));
    }

    #[test]
    fn test_tool_output_filter_shell() {
        let input = "\x1b[32mok\x1b[0m\n\n\n";
        let result = ToolOutputFilter::apply("system_shell", input);
        assert_eq!(result, "ok");
    }

    #[test]
    fn test_multiple_transforms_chain() {
        let input = "\x1b[31m\n\na\na\nb\n\n\x1b[0m";
        let result = ToolOutputFilter::apply("system_shell", input);
        // strip_ansi + trim_edges (dedupe not enabled by default for system_shell)
        assert_eq!(result, "a\na\nb");
    }
}
