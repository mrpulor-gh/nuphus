//! Excel (.xlsx) 写出 — CSV/TSV/Markdown 表格 → .xlsx
//!
//! 与 `super::xlsx`（只读）互补。`rust_xlsxwriter` 只能创建新文件，不能原地修改。

use rust_xlsxwriter::*;

/// 将 CSV 文本写入 .xlsx 文件
///
/// `content` 按行拆分，逗号分隔的每个字段填入一个单元格。
/// 第一行作为表头（加粗）。如果内容不符合 CSV 格式则退化为逐行单格写入。
pub fn write_text_to_xlsx(path: &str, content: &str, sheet_name: &str) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let worksheet = workbook
        .add_worksheet()
        .set_name(sheet_name)
        .map_err(|e| format!("设置 sheet 名失败: {}", e))?;

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        // 空内容 → 空 xlsx
        workbook
            .save(path)
            .map_err(|e| format!("保存 xlsx 失败: {}", e))?;
        return Ok(());
    }

    // ── 检测分隔符：优先尝试 tab，其次逗号，其他分隔符 ──
    let delimiter = detect_delimiter(&lines);

    let bold = Format::new().set_bold();

    for (row_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 处理 Markdown 表格行（以 | 开头/结尾）
        let cells: Vec<&str> = if trimmed.starts_with('|') || trimmed.ends_with('|') {
            parse_markdown_table_row(trimmed)
        } else if delimiter != '\0' {
            // CSV/TSV 行
            trimmed.split(delimiter).map(|s| s.trim()).collect()
        } else if trimmed.starts_with("|-") || trimmed.starts_with("|:") {
            // Markdown 分隔行：跳过
            continue;
        } else {
            // 退化：一行一个单元格
            vec![trimmed]
        };

        // 在 xlsx 的行号上跳过分隔行（不占行）
        let data_row = if row_idx == 0 { 0 } else { row_idx };

        for (col_idx, cell_text) in cells.iter().enumerate() {
            let unwrapped = cell_text.trim().trim_matches('"');
            if row_idx == 0 {
                // 表头加粗
                worksheet
                    .write_with_format(data_row as u32, col_idx as u16, unwrapped, &bold)
                    .map_err(|e| format!("写入表头失败: {}", e))?;
            } else {
                // 尝试写数字
                if let Ok(n) = unwrapped.parse::<f64>() {
                    worksheet
                        .write_number(data_row as u32, col_idx as u16, n)
                        .map_err(|e| format!("写入数字失败: {}", e))?;
                } else {
                    worksheet
                        .write_string(data_row as u32, col_idx as u16, unwrapped)
                        .map_err(|e| format!("写入文本失败: {}", e))?;
                }
            }
        }
    }

    workbook
        .save(path)
        .map_err(|e| format!("保存 xlsx 失败: {}", e))?;

    Ok(())
}

/// 读取 xlsx 为 CSV 格式文本（用于 Edit 的中间表示）
pub fn xlsx_to_csv_text(path: &str) -> Result<String, String> {
    use calamine::{open_workbook, Reader, Xlsx};

    let mut workbook: Xlsx<_> =
        open_workbook(path).map_err(|e| format!("打开 xlsx 失败: {}", e))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut output = String::new();

    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| format!("读取 sheet '{}' 失败: {}", name, e))?;

        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(|c| cell_escape(c.to_string())).collect())
            .collect();

        if rows.is_empty() {
            continue;
        }

        if !output.is_empty() {
            output.push('\n');
        }

        for row in &rows {
            output.push_str(&row.join(","));
            output.push('\n');
        }
    }

    Ok(output)
}

/// 将 xlsx 内容原地编辑：读取 → CSV → 替换 → 写回
pub fn edit_xlsx(
    path: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
) -> Result<(), String> {
    // 1. 读为 CSV 文本
    let csv_text = xlsx_to_csv_text(path)?;

    // 2. 文本替换
    let modified = if replace_all {
        csv_text.replace(old_str, new_str)
    } else {
        csv_text.replacen(old_str, new_str, 1)
    };

    // 3. 写回 xlsx
    write_text_to_xlsx(path, &modified, "Sheet1")
}

// ── 辅助函数 ──

/// 检测 CSV/TSV 分隔符
fn detect_delimiter(lines: &[&str]) -> char {
    // 取前 5 行非空行
    let sample: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('|'))
        .take(5)
        .copied()
        .collect();

    if sample.is_empty() {
        return '\0';
    }

    // 计数各分隔符
    let mut tab_count = 0usize;
    let mut comma_count = 0usize;
    let mut pipe_count = 0usize;

    for line in &sample {
        tab_count += line.chars().filter(|&c| c == '\t').count();
        comma_count += line.chars().filter(|&c| c == ',').count();
        pipe_count += line.chars().filter(|&c| c == '|').count();
    }

    // 按行数取平均
    let n = sample.len();
    let tab_avg = tab_count / n;
    let comma_avg = comma_count / n;
    let pipe_avg = pipe_count / n;

    // 选择最常见的分隔符（至少每行 1 个）
    if tab_avg >= 1 && tab_avg >= comma_avg && tab_avg >= pipe_avg {
        '\t'
    } else if comma_avg >= 1 && comma_avg >= pipe_avg {
        ','
    } else if pipe_avg >= 1 {
        '|'
    } else {
        '\0'
    }
}

/// 解析 Markdown 表格行（处理 | 开头/结尾）
fn parse_markdown_table_row(line: &str) -> Vec<&str> {
    let trimmed = line.trim();
    // 去掉首尾 |
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|s| s.trim()).collect()
}

/// CSV 单元格转义：包含逗号/引号/换行时加双引号
fn cell_escape(s: String) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_delimiter() {
        assert_eq!(detect_delimiter(&["a,b,c", "1,2,3"]), ',');
        assert_eq!(detect_delimiter(&["a\tb\tc", "1\t2\t3"]), '\t');
    }

    #[test]
    fn test_parse_markdown_row() {
        let cells = parse_markdown_table_row("| A | B | C |");
        assert_eq!(cells, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_cell_escape() {
        assert_eq!(cell_escape("hello".to_string()), "hello");
        assert_eq!(cell_escape("a,b".to_string()), "\"a,b\"");
        assert_eq!(cell_escape("he\"llo".to_string()), "\"he\"\"llo\"");
    }
}
