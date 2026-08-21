//! Excel (.xlsx) 读取器 — calamine → Markdown 表格

use calamine::{open_workbook, Reader, Xlsx};

/// 读取 .xlsx 文件，返回 Markdown 表格文本
/// 每个 sheet 渲染为独立表格段
pub fn read_xlsx_to_markdown(path: &str) -> Result<String, String> {
    let mut workbook: Xlsx<_> =
        open_workbook(path).map_err(|e| format!("打开 Excel 失败: {}", e))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut output = String::new();

    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| format!("读取工作表 '{}' 失败: {}", name, e))?;

        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| {
                row.iter()
                    .map(|c| c.to_string().trim().to_string())
                    .collect()
            })
            .collect();

        if rows.is_empty() {
            continue;
        }

        // 分割标题行和数据行
        let header = &rows[0];
        let data = &rows[1..];

        if !output.is_empty() {
            output.push_str("\n\n---\n\n");
        }
        output.push_str(&format!("## {}\n\n", name));

        // 计算列宽（中文算2宽）
        let mut col_widths: Vec<usize> = header.iter().map(|h| str_width(h)).collect();
        for row in data {
            for (i, cell) in row.iter().enumerate() {
                if i >= col_widths.len() {
                    break;
                }
                let w = str_width(cell);
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }

        // 渲染表头
        output.push('|');
        for (i, h) in header.iter().enumerate() {
            output.push_str(&format!(
                " {} ",
                pad(h, col_widths.get(i).copied().unwrap_or(0))
            ));
            output.push('|');
        }
        output.push('\n');
        output.push('|');
        for w in &col_widths {
            let dashes = "-".repeat(*w);
            output.push_str(&format!("{}|", dashes));
        }
        output.push('\n');

        // 渲染数据行
        for row in data {
            output.push('|');
            for (i, cell) in row.iter().enumerate() {
                if i >= col_widths.len() {
                    break;
                }
                output.push_str(&format!(" {} ", pad(cell, col_widths[i])));
                output.push('|');
            }
            output.push('\n');
        }
    }

    Ok(output)
}

fn str_width(s: &str) -> usize {
    s.chars().map(|c| if c > '\u{7f}' { 2 } else { 1 }).sum()
}

fn pad(s: &str, width: usize) -> String {
    let w = str_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}
