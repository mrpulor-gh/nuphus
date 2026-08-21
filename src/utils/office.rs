//! 办公文档读取器 — docx / pptx / xls / ods / odt / odp / pdf → 纯文本/Markdown
//!
//! 统一入口: `read_office(path) -> Option<String>`
//! 根据扩展名自动路由到对应的解析器。

use std::io::Read;

// ── 统一入口 ──

/// 读取办公文档，返回格式化文本（Markdown / 纯文本）。
/// 返回 None 表示不支持此扩展名。
pub fn read_office(path: &str) -> Option<Result<String, String>> {
    let lower = path.to_lowercase();
    if lower.ends_with(".docx") {
        Some(read_docx(path))
    } else if lower.ends_with(".pptx") {
        Some(read_pptx(path))
    } else if lower.ends_with(".xls") {
        Some(read_xls(path))
    } else if lower.ends_with(".ods") {
        Some(read_ods(path))
    } else if lower.ends_with(".odt") {
        Some(read_odt(path))
    } else if lower.ends_with(".odp") {
        Some(read_odp(path))
    } else if lower.ends_with(".pdf") {
        Some(read_pdf(path))
    } else {
        None
    }
}

// ── .docx ──

/// 读取 .docx 文件，提取纯文本
fn read_docx(path: &str) -> Result<String, String> {
    let mut archive = open_zip(path)?;
    let xml_text = read_zip_entry(&mut archive, "word/document.xml")
        .map_err(|e| format!("docx 中找不到 word/document.xml: {}", e))?;

    // 提取 <w:t> 标签之间的文本（支持段落换行）
    let text = extract_w_t_text(&xml_text);
    debug_assert!(!text.is_empty(), "docx 提取结果为空: {}", path);
    Ok(format!(
        "{} (docx parsed, {} chars)\n{}",
        path,
        text.len(),
        text
    ))
}

/// 在 `haystack` 中查找标签 `prefix`（如 "<w:tbl"），要求其后紧跟 '>' 或空白，
/// 避免误匹配 <w:tblPr> / <w:tcPr> / <w:pStyle> 等同前缀标签
fn find_tag_open(haystack: &str, prefix: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(p) = haystack[from..].find(prefix) {
        let abs = from + p;
        let after = abs + prefix.len();
        match haystack.as_bytes().get(after) {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => return Some(abs),
            _ => from = after,
        }
    }
    None
}

/// 从标签片段中读取属性值（支持双/单引号），如 `w:val="Heading1"` → "Heading1"
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let pat = format!("{}=", name);
    let p = tag.find(&pat)?;
    let rest = &tag[p + pat.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// 提取片段内全部 `<w:t>` 文本并拼接（run 级，不含段落结构）
fn collect_w_t(seg: &str) -> String {
    let mut out = String::new();
    let mut rest = seg;
    while let Some(p) = find_tag_open(rest, "<w:t") {
        let content_start = match rest[p..].find('>') {
            Some(gt) => p + gt + 1,
            None => break,
        };
        if let Some(end) = rest[content_start..].find("</w:t>") {
            out.push_str(&rest[content_start..content_start + end]);
            rest = &rest[content_start + end + 6..];
        } else {
            break;
        }
    }
    out
}

/// 从段落属性（pPr）识别标题级别 1-6：
/// 优先 `<w:pStyle w:val="HeadingN"/>`，其次 `<w:outlineLvl w:val="K"/>`（0-based，+1）
fn heading_level(ppr: &str) -> Option<usize> {
    if let Some(p) = ppr.find("<w:pStyle") {
        if let Some(v) = attr_value(&ppr[p..], "w:val") {
            let lower = v.to_lowercase();
            if let Some(rest) = lower.strip_prefix("heading") {
                if let Some(n) = rest.trim().chars().next().and_then(|c| c.to_digit(10)) {
                    if (1..=6).contains(&n) {
                        return Some(n as usize);
                    }
                }
            }
        }
    }
    if let Some(p) = ppr.find("<w:outlineLvl") {
        if let Some(v) = attr_value(&ppr[p..], "w:val") {
            if let Ok(k) = v.trim().parse::<usize>() {
                if k < 6 {
                    return Some(k + 1);
                }
            }
        }
    }
    None
}

/// 渲染单个 `<w:p>` 片段为 Markdown 行；空段返回 None。
/// 标题 → `#` 层级，列表（numPr）→ `- ` 前缀，其余（含未知样式）→ 纯文本
fn render_w_paragraph(seg: &str) -> Option<String> {
    let text = collect_w_t(seg);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let ppr = match seg.find("</w:pPr>") {
        Some(e) => &seg[..e + 9],
        None => "",
    };
    if let Some(level) = heading_level(ppr) {
        return Some(format!("{} {}", "#".repeat(level), text));
    }
    if ppr.contains("<w:numPr") {
        return Some(format!("- {}", text));
    }
    Some(text.to_string())
}

/// 渲染一行管道 MD 表（不足 cols 列补空单元格）
fn md_table_row(cells: &[String], cols: usize) -> String {
    let mut s = String::from("|");
    for i in 0..cols {
        s.push(' ');
        if let Some(c) = cells.get(i) {
            s.push_str(c);
        }
        s.push_str(" |");
    }
    s
}

/// 渲染 `<w:tbl>` 片段为管道 Markdown 表：首行表头 + |---| 分隔行 + 数据行
fn render_w_table(seg: &str) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut rest = seg;
    while let Some(tr) = find_tag_open(rest, "<w:tr") {
        let row_end = match rest[tr..].find("</w:tr>") {
            Some(e) => tr + e,
            None => break,
        };
        let row_seg = &rest[tr..row_end];
        let mut cells = Vec::new();
        let mut cell_rest = row_seg;
        while let Some(tc) = find_tag_open(cell_rest, "<w:tc") {
            let cell_end = match cell_rest[tc..].find("</w:tc>") {
                Some(e) => tc + e,
                None => break,
            };
            let cell = collect_w_t(&cell_rest[tc..cell_end]);
            cells.push(cell.trim().replace('|', "\\|").replace('\n', " "));
            cell_rest = &cell_rest[cell_end + 6..];
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
        rest = &rest[row_end + 7..];
    }
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut out = md_table_row(&rows[0], cols);
    out.push('\n');
    out.push('|');
    for _ in 0..cols {
        out.push_str("---|");
    }
    for row in &rows[1..] {
        out.push('\n');
        out.push_str(&md_table_row(row, cols));
    }
    out
}

/// docx 正文 → Markdown：标题保留 `#` 层级、`<w:tbl>` 转管道表、
/// numPr 列表段加 `- ` 前缀，其余段落按纯文本输出。
/// 手工字符串扫描风格，按文档顺序逐块（段落/表格）消费。
fn extract_w_t_text(xml: &str) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let mut rest = xml;

    while !rest.is_empty() {
        let p_pos = find_tag_open(rest, "<w:p");
        let tbl_pos = find_tag_open(rest, "<w:tbl");
        // 表格必须先于段落判定：表内单元格也含 <w:p>，整表作为一个块消费
        let (kind, pos) = match (p_pos, tbl_pos) {
            (Some(p), Some(t)) if p < t => ("p", p),
            (Some(_), Some(t)) => ("tbl", t),
            (Some(p), None) => ("p", p),
            (None, Some(t)) => ("tbl", t),
            (None, None) => break,
        };
        match kind {
            "p" => {
                let end = match rest[pos..].find("</w:p>") {
                    Some(e) => pos + e,
                    None => break,
                };
                if let Some(line) = render_w_paragraph(&rest[pos..end]) {
                    blocks.push(line);
                }
                rest = &rest[end + 6..];
            }
            _ => {
                let end = match rest[pos..].find("</w:tbl>") {
                    Some(e) => pos + e,
                    None => break,
                };
                let table = render_w_table(&rest[pos..end]);
                if !table.is_empty() {
                    blocks.push(table);
                }
                rest = &rest[end + 8..];
            }
        }
    }

    blocks.join("\n")
}

/// 从 "ppt/slides/slideN.xml" 提取页码 N；解析失败返回 u32::MAX 排到末尾
fn slide_number(name: &str) -> u32 {
    name.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(u32::MAX)
}

/// 一页 slide 的图片清单条目
struct SlideImage {
    name: String,
    dims: Option<(u32, u32)>,
    size: usize,
}

/// 把 rels 中的相对 Target（如 "../notesSlides/notesSlide1.xml"）
/// 规范化为 zip 内绝对路径（基于 source 所在目录，如 "ppt/slides"）
fn normalize_zip_path(base_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// 解析 slide 的 .rels：返回（notesSlide Target, 图片 Target 列表）。
/// 关系 Type 含 "notesSlide" 为备注页，含 "/image" 为图片，其余（slideLayout 等）忽略。
fn parse_slide_rels(rels_xml: &str) -> (Option<String>, Vec<String>) {
    let mut notes = None;
    let mut images = Vec::new();
    let mut rest = rels_xml;
    while let Some(p) = rest.find("<Relationship") {
        let tag_end = match rest[p..].find('>') {
            Some(e) => p + e,
            None => break,
        };
        let tag = &rest[p..tag_end];
        let rel_type = attr_value(tag, "Type").unwrap_or_default();
        if let Some(target) = attr_value(tag, "Target") {
            if rel_type.contains("notesSlide") {
                if notes.is_none() {
                    notes = Some(target);
                }
            } else if rel_type.contains("/image") {
                images.push(target);
            }
        }
        rest = &rest[tag_end + 1..];
    }
    (notes, images)
}

/// 读取某页 slide 的备注文本与图片清单（经 ppt/slides/_rels/slideN.xml.rels 定位）。
/// rels 缺失或个别条目读取失败时降级为空，不中断整篇提取。
fn read_slide_extras(
    archive: &mut zip::ZipArchive<std::fs::File>,
    slide_num: u32,
) -> (Option<String>, Vec<SlideImage>) {
    let rels_name = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);
    let rels_xml = match read_zip_entry(archive, &rels_name) {
        Ok(x) => x,
        Err(_) => return (None, Vec::new()),
    };
    let (notes_target, image_targets) = parse_slide_rels(&rels_xml);

    let notes = notes_target
        .map(|t| normalize_zip_path("ppt/slides", &t))
        .and_then(|entry| read_zip_entry(archive, &entry).ok())
        .map(|xml| extract_a_t_text(&xml))
        .filter(|t| !t.is_empty());

    let mut images = Vec::new();
    for target in image_targets {
        let entry = normalize_zip_path("ppt/slides", &target);
        if let Ok(bytes) = read_zip_entry_bytes(archive, &entry) {
            let name = entry.rsplit('/').next().unwrap_or(&entry).to_string();
            let dims = image::load_from_memory(&bytes)
                .ok()
                .map(|img| (img.width(), img.height()));
            images.push(SlideImage {
                name,
                dims,
                size: bytes.len(),
            });
        }
    }
    (notes, images)
}

/// 读取 .pptx 文件，提取各 slide 文本 + 演讲者备注 + 图片清单
fn read_pptx(path: &str) -> Result<String, String> {
    let mut archive = open_zip(path)?;

    // 枚举所有 slide 文件并按页码数字排序（zip 目录序不保证 slide2 排在 slide10 前）
    let mut slide_files: Vec<(u32, String)> = archive
        .file_names()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .map(|n| (slide_number(n), n.to_string()))
        .collect();
    slide_files.sort();

    if slide_files.is_empty() {
        return Err(format!("pptx 中找不到 ppt/slides/slide*.xml: {}", path));
    }

    let mut output = String::new();
    output.push_str(&format!(
        "{} (pptx parsed, {} slides)\n\n",
        path,
        slide_files.len()
    ));

    for (num, slide_name) in &slide_files {
        let xml_text = read_zip_entry(&mut archive, slide_name)
            .map_err(|e| format!("读取 {} 失败: {}", slide_name, e))?;

        let slide_text = extract_a_t_text(&xml_text);
        let (notes, images) = read_slide_extras(&mut archive, *num);

        if slide_text.is_empty() && notes.is_none() && images.is_empty() {
            continue;
        }
        output.push_str(&format!("## Slide {}\n\n", num));
        if !slide_text.is_empty() {
            output.push_str(&slide_text);
            output.push('\n');
        }
        if let Some(n) = notes {
            output.push_str(&format!("> 备注: {}\n", n.replace('\n', "\n> ")));
        }
        for img in &images {
            match img.dims {
                Some((w, h)) => output.push_str(&format!(
                    "> 图片: {} ({}x{}, {} B)\n",
                    img.name, w, h, img.size
                )),
                None => output.push_str(&format!("> 图片: {} ({} B)\n", img.name, img.size)),
            }
        }
        output.push('\n');
    }

    Ok(output.trim().to_string())
}

/// pptx 中的文本嵌入在 `<a:t>` 标签，形状间可能换行
fn extract_a_t_text(xml: &str) -> String {
    let mut result = String::new();
    let mut rest = xml;

    loop {
        // 检测段落/换行标记 (a:p = paragraph, a:br = line break)
        if let Some(pos) = rest.find("<a:p>") {
            result.push('\n');
            rest = &rest[pos + 5..];
        } else if rest.starts_with("</a:p>") {
            result.push('\n');
            rest = &rest[6..];
        } else if let Some(pos) = rest.find("<a:br") {
            result.push('\n');
            if let Some(end) = rest[pos..].find('>') {
                rest = &rest[pos + end + 1..];
            } else {
                break;
            }
        }
        // 找文本
        else if let Some(start) = rest.find("<a:t") {
            let content_begin = match rest[start..].find('>') {
                Some(p) => start + p + 1,
                None => {
                    rest = &rest[start + 4..];
                    continue;
                }
            };
            if let Some(end) = rest[content_begin..].find("</a:t>") {
                let text = &rest[content_begin..content_begin + end];
                if !text.trim().is_empty() {
                    result.push_str(text);
                }
                rest = &rest[content_begin + end + 6..];
            } else {
                rest = &rest[content_begin..];
            }
        } else {
            break;
        }
    }

    result.trim().to_string()
}

// ── .xls (旧版 Excel) / .ods (Calc 电子表格) ──

/// 读取 .xls 文件（旧版 Excel），复用 calamine
fn read_xls(path: &str) -> Result<String, String> {
    use calamine::{open_workbook, Xls};
    let mut workbook: Xls<_> = open_workbook(path).map_err(|e| format!("打开 XLS 失败: {}", e))?;
    let sheets = collect_calamine_sheets(&mut workbook)?;
    Ok(format_sheets_markdown(path, "xls", &sheets))
}

/// 读取 .ods 文件（LibreOffice Calc 电子表格），复用 calamine
fn read_ods(path: &str) -> Result<String, String> {
    use calamine::{open_workbook, Ods};
    let mut workbook: Ods<_> = open_workbook(path).map_err(|e| format!("打开 ODS 失败: {}", e))?;
    let sheets = collect_calamine_sheets(&mut workbook)?;
    Ok(format_sheets_markdown(path, "ods", &sheets))
}

/// 从 calamine workbook 采集 sheet 数据
fn collect_calamine_sheets<R, RS>(
    workbook: &mut R,
) -> Result<Vec<(String, Vec<Vec<String>>)>, String>
where
    R: calamine::Reader<RS>,
    RS: std::io::Read + std::io::Seek,
    R::Error: std::fmt::Display,
{
    let sheet_names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::new();
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
        sheets.push((name.clone(), rows));
    }
    Ok(sheets)
}

/// 将 sheets 数据渲染为 Markdown 表格
fn format_sheets_markdown(path: &str, fmt: &str, sheets: &[(String, Vec<Vec<String>>)]) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "{} ({} parsed, {} sheets)\n\n",
        path,
        fmt,
        sheets.len()
    ));

    for (name, rows) in sheets {
        if rows.is_empty() {
            continue;
        }
        output.push_str(&format!("## {}\n\n", name));

        let header = &rows[0];
        let data = &rows[1..];

        let mut col_widths: Vec<usize> = header.iter().map(|h| display_width(h)).collect();
        for row in data {
            for (i, cell) in row.iter().enumerate() {
                if i >= col_widths.len() {
                    break;
                }
                let w = display_width(cell);
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }

        // 表头
        output.push('|');
        for (i, h) in header.iter().enumerate() {
            output.push_str(&format!(" {} ", pad_cell(h, col_widths[i])));
            output.push('|');
        }
        output.push('\n');

        // 分隔线
        output.push('|');
        for w in &col_widths {
            output.push_str(&format!("{}|", "-".repeat(*w)));
        }
        output.push('\n');

        // 数据
        for row in data {
            output.push('|');
            for (i, cell) in row.iter().enumerate() {
                if i >= col_widths.len() {
                    break;
                }
                output.push_str(&format!(" {} ", pad_cell(cell, col_widths[i])));
                output.push('|');
            }
            output.push('\n');
        }
        output.push('\n');
    }

    output.trim().to_string()
}

// ── .odt (Writer 文档) ──

/// 读取 .odt 文件，提取纯文本
fn read_odt(path: &str) -> Result<String, String> {
    let mut archive = open_zip(path)?;
    let xml_text = read_zip_entry(&mut archive, "content.xml")
        .map_err(|e| format!("odt 中找不到 content.xml: {}", e))?;
    let text = extract_odf_text(&xml_text, "<text:p>", "</text:p>");
    Ok(format!(
        "{} (odt parsed, {} chars)\n{}",
        path,
        text.len(),
        text
    ))
}

// ── .odp (Impress 演示) ──

/// 读取 .odp 文件，提取各页文本
fn read_odp(path: &str) -> Result<String, String> {
    let mut archive = open_zip(path)?;
    let xml_text = read_zip_entry(&mut archive, "content.xml")
        .map_err(|e| format!("odp 中找不到 content.xml: {}", e))?;
    let text = extract_odf_text(&xml_text, "<text:p>", "</text:p>");
    Ok(format!(
        "{} (odp parsed, {} chars)\n{}",
        path,
        text.len(),
        text
    ))
}

/// ODF 通用文本提取：取 <text:p> 段落，strip XML 标签
fn extract_odf_text(xml: &str, _paragraph_open: &str, _paragraph_close: &str) -> String {
    let mut result = String::new();
    let mut rest = xml;

    while let Some(start) = rest.find("<text:p") {
        // 跳过标签
        let after_tag = match rest[start..].find('>') {
            Some(p) => start + p + 1,
            None => {
                rest = &rest[start + 7..];
                continue;
            }
        };
        if let Some(end) = rest[after_tag..].find("</text:p>") {
            let para_xml = &rest[after_tag..after_tag + end];
            let para_text = strip_xml_tags(para_xml);
            if !para_text.trim().is_empty() {
                result.push_str(para_text.trim());
                result.push('\n');
            }
            rest = &rest[after_tag + end + 9..];
        } else {
            rest = &rest[after_tag..];
        }
    }

    result.trim().to_string()
}

/// 简单 XML 标签剥离（不处理 CDATA，ODF 中少见）
fn strip_xml_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

// ── .pdf ──

/// 单次 OCR 的页数上限：渲染与 OCR 均为线性开销，防止超大扫描件卡死调用方
const MAX_OCR_PAGES: u32 = 50;

/// pdf.js 文本提取的页数上限：整篇 PDF base64 经 eval 注入 webview、
/// 文本再逐页回传，限制页数防止超大文档撑爆载荷
const MAX_EXTRACT_PAGES: u32 = 200;

/// 逐页分类：返回（有文本层页码, 无文本层页码），均 1-based。
/// 空白（trim 后为空）的页视为无文本层，进入 OCR 候选队列。
fn classify_pdf_pages(page_texts: &[String]) -> (Vec<u32>, Vec<u32>) {
    let mut text_pages = Vec::new();
    let mut ocr_pages = Vec::new();
    for (i, t) in page_texts.iter().enumerate() {
        let page_no = i as u32 + 1;
        if t.trim().is_empty() {
            ocr_pages.push(page_no);
        } else {
            text_pages.push(page_no);
        }
    }
    (text_pages, ocr_pages)
}

/// 桥路径头部：纯文本件与含 OCR 页的混合件使用不同标记。
fn pdf_bridge_header(path: &str, chars: usize, pages: usize, ocr_pages: usize) -> String {
    if ocr_pages == 0 {
        format!("{path} (pdf parsed, {chars} chars, {pages} pages)")
    } else {
        format!("{path} (pdf mixed, {chars} chars, {pages} pages, {ocr_pages} pages OCR)")
    }
}

/// 读取 PDF 文件，提取纯文本。
///
/// 提取路径优先级：
/// 1. 桌面壳 pdf.js 桥可用 → getTextContent 逐页提取文本层（高速路径，
///    覆盖 lopdf 不支持的 CID-keyed 字体），无文本层的页单独渲染 + OCR
/// 2. 桥不可用（或执行失败）→ lopdf 直接提取
/// 3. lopdf 提取为空 → 视为扫描件，整篇走渲染 + OCR 兜底
fn read_pdf(path: &str) -> Result<String, String> {
    if crate::render_bridge::is_available() && crate::render_bridge::is_text_available() {
        // 桥已注册但执行失败（超时/前端异常）时继续走 lopdf 链路，
        // 桥错误会在 OCR 兜底链路中再次浮现，不丢诊断
        if let Ok(text) = read_pdf_via_bridge(path) {
            return Ok(text);
        }
    }

    let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {}", e))?;

    let doc = lopdf::Document::load_mem(&bytes).map_err(|e| format!("解析 PDF 失败: {}", e))?;

    let pages = doc.get_pages();
    let mut text = String::new();

    for page_num in pages.keys() {
        if let Ok(page_text) = doc.extract_text(&[*page_num]) {
            if !page_text.trim().is_empty() {
                text.push_str(&format!("--- Page {} ---\n", page_num));
                text.push_str(page_text.trim());
                text.push('\n');
            }
        }
    }

    if text.is_empty() {
        // 扫描模式 PDF（纯图片）：lopdf 抓不到文本 → 渲染为 PNG → PaddleOCR 兜底
        return read_pdf_scanned_ocr(path, pages.len());
    }

    Ok(format!(
        "{} (pdf parsed, {} chars, {} pages)\n{}",
        path,
        text.len(),
        pages.len(),
        text.trim()
    ))
}

/// pdf.js 桥路径：文本层逐页提取（--- Page N ---），无文本层的页单独
/// 渲染 + OCR（--- Page N (OCR) ---），混合件按页分流后按页码顺序输出。
fn read_pdf_via_bridge(path: &str) -> Result<String, String> {
    let page_texts = crate::render_bridge::extract_pdf_text(path, MAX_EXTRACT_PAGES)?;
    let (text_pages, ocr_pages) = classify_pdf_pages(&page_texts);

    if text_pages.is_empty() {
        // 全文无文本层：等同扫描件，走既有 OCR 兜底（上限与错误链路一致）
        return read_pdf_scanned_ocr(path, page_texts.len());
    }

    let mut segments: Vec<(u32, String)> = Vec::new();
    for &p in &text_pages {
        segments.push((
            p,
            format!(
                "--- Page {} ---\n{}\n",
                p,
                page_texts[(p - 1) as usize].trim()
            ),
        ));
    }

    let mut ocr_attempted = 0usize;
    let mut ocr_skipped = 0usize;
    if !ocr_pages.is_empty() {
        let to_ocr: Vec<u32> = ocr_pages
            .iter()
            .take(MAX_OCR_PAGES as usize)
            .copied()
            .collect();
        ocr_attempted = to_ocr.len();
        ocr_skipped = ocr_pages.len() - to_ocr.len();

        let pages_png = crate::render_bridge::render_pdf_pages(path, &to_ocr)
            .map_err(|e| format!("渲染无文本层页失败: {e}"))?;
        let mut engine = crate::desktop::paddle_ocr::PaddleOcr::new()
            .map_err(|e| format!("OCR 引擎初始化失败: {e}"))?;
        for (page_no, png) in to_ocr.iter().zip(pages_png.iter()) {
            let img = image::load_from_memory(png)
                .map_err(|e| format!("解码第 {} 页渲染结果失败: {}", page_no, e))?
                .to_rgb8();
            match engine.ocr_image(&img) {
                Ok(t) if !t.trim().is_empty() => {
                    segments.push((
                        *page_no,
                        format!("--- Page {} (OCR) ---\n{}\n", page_no, t.trim()),
                    ));
                }
                Ok(_) => {}
                // 单页失败不中止整篇：保留失败标记，让其余页文本可用
                Err(e) => {
                    segments.push((
                        *page_no,
                        format!("--- Page {} (OCR 失败: {}) ---\n", page_no, e),
                    ));
                }
            }
        }
    }

    segments.sort_by_key(|(page_no, _)| *page_no);
    let mut body = String::new();
    for (_, seg) in &segments {
        body.push_str(seg);
    }
    if ocr_skipped > 0 {
        body.push_str(&format!(
            "--- 其余 {} 页无文本层，超出单次 OCR 上限（{} 页），未处理 ---\n",
            ocr_skipped, MAX_OCR_PAGES
        ));
    }

    Ok(format!(
        "{}\n{}",
        pdf_bridge_header(path, body.len(), page_texts.len(), ocr_attempted),
        body.trim()
    ))
}

/// 扫描件 PDF 兜底：经 render_bridge（桌面壳 pdf.js 渲染服务）把每页渲染为
/// PNG（全程内存，不落临时文件），再逐页跑 PaddleOCR 输出文本。
///
/// 桥未注入（纯 lib/测试环境）或渲染失败时，保留原错误信息并追加原因。
fn read_pdf_scanned_ocr(path: &str, total_pages: usize) -> Result<String, String> {
    let original_err = || format!("PDF 不含可提取文本（可能是纯扫描件）: {}", path);

    let pages_png = match crate::render_bridge::render_pdf(path, MAX_OCR_PAGES) {
        Ok(p) => p,
        Err(e) => return Err(format!("{}\nOCR 兜底不可用: {}", original_err(), e)),
    };

    let mut engine = match crate::desktop::paddle_ocr::PaddleOcr::new() {
        Ok(e) => e,
        Err(e) => {
            return Err(format!(
                "{}\nOCR 兜底不可用: 引擎初始化失败: {}",
                original_err(),
                e
            ));
        }
    };

    let mut out = String::new();
    let mut any_text = false;
    for (i, png) in pages_png.iter().enumerate() {
        let img = image::load_from_memory(png)
            .map_err(|e| format!("解码第 {} 页渲染结果失败: {}", i + 1, e))?
            .to_rgb8();
        match engine.ocr_image(&img) {
            Ok(t) if !t.trim().is_empty() => {
                any_text = true;
                out.push_str(&format!("--- Page {} (OCR) ---\n", i + 1));
                out.push_str(t.trim());
                out.push('\n');
            }
            Ok(_) => {}
            // 单页失败不中止整篇：保留失败标记，让其余页文本可用
            Err(e) => {
                out.push_str(&format!("--- Page {} (OCR 失败: {}) ---\n", i + 1, e));
            }
        }
    }

    if !any_text {
        return Err(format!(
            "{}\nOCR 兜底执行完成但未识别到任何文本",
            original_err()
        ));
    }

    let rendered = pages_png.len();
    let scope_note = if total_pages > rendered {
        format!(", 共 {} 页仅处理前 {} 页", total_pages, rendered)
    } else {
        String::new()
    };
    Ok(format!(
        "{} (pdf OCR, {} chars, {} pages{})\n{}",
        path,
        out.len(),
        rendered,
        scope_note,
        out.trim()
    ))
}

// ── ZIP 工具 ──

fn open_zip(path: &str) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
    zip::ZipArchive::new(file)
        .map_err(|e| format!("ZIP 解压失败（可能不是有效的 Office 文档）: {}", e))
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<String, String> {
    let mut file = archive
        .by_name(name)
        .map_err(|e| format!("ZIP 中找不到 '{}': {}", name, e))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|e| format!("读取 '{}' 失败: {}", name, e))?;
    Ok(buf)
}

/// 二进制读取 zip 条目（图片等非文本资源）
fn read_zip_entry_bytes(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut file = archive
        .by_name(name)
        .map_err(|e| format!("ZIP 中找不到 '{}': {}", name, e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("读取 '{}' 失败: {}", name, e))?;
    Ok(buf)
}

// ── 文本格式化工具 ──

fn display_width(s: &str) -> usize {
    s.chars().map(|c| if c > '\u{7f}' { 2 } else { 1 }).sum()
}

fn pad_cell(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 全局唯一临时文件后缀：pid + 原子序号 + 纳秒时间戳。
    /// Rust 测试默认并行执行，多个测试若共用 `nuphus_test_{pid}.pptx` 固定名会互相覆盖
    /// （实测 PPTX 测试族并行时偶发 "Could not find EOCD" flaky）。此函数保证每个
    /// 调用产生不同路径，彻底消除共享临时文件竞态。
    fn unique_test_file(prefix: &str, ext: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("{prefix}_{}_{n}_{ts}.{ext}", std::process::id()))
    }

    #[test]
    fn test_strip_xml_tags() {
        assert_eq!(strip_xml_tags("<text:span>Hello</text:span>"), "Hello");
        assert_eq!(strip_xml_tags("A &amp; B"), "A &amp; B");
        assert_eq!(strip_xml_tags(""), "");
    }

    #[test]
    fn test_extract_w_t_text() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p></w:body></w:document>"#;
        let text = extract_w_t_text(xml);
        eprintln!("DEBUG extract_w_t_text result: {:?}", text);
        assert!(text.contains("Hello"), "Expected 'Hello' in: {:?}", text);
        assert!(text.contains("World"), "Expected 'World' in: {:?}", text);
    }

    #[test]
    fn test_extract_a_t_text() {
        let xml =
            r#"<p:sp><a:txBody><a:p><a:r><a:t>Slide Text</a:t></a:r></a:p></a:txBody></p:sp>"#;
        let text = extract_a_t_text(xml);
        assert!(text.contains("Slide Text"));
    }

    // ── docx 结构化提取 ──

    #[test]
    fn test_extract_w_t_text_heading_levels() {
        let xml = r#"<w:document><w:body>\
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>第一章</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>小节</w:t></w:r></w:p>\
<w:p><w:pPr><w:outlineLvl w:val="2"/></w:pPr><w:r><w:t>更深层</w:t></w:r></w:p>\
</w:body></w:document>"#;
        let text = extract_w_t_text(xml);
        assert!(text.contains("# 第一章"), "in: {:?}", text);
        assert!(text.contains("## 小节"), "in: {:?}", text);
        assert!(text.contains("### 更深层"), "in: {:?}", text);
    }

    #[test]
    fn test_extract_w_t_text_table() {
        let xml = r#"<w:document><w:body><w:tbl><w:tblPr/><w:tblGrid/>\
<w:tr><w:tc><w:p><w:r><w:t>名称</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>数量</w:t></w:r></w:p></w:tc></w:tr>\
<w:tr><w:tc><w:p><w:r><w:t>苹果</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>3</w:t></w:r></w:p></w:tc></w:tr>\
</w:tbl></w:body></w:document>"#;
        let text = extract_w_t_text(xml);
        assert!(text.contains("| 名称 | 数量 |"), "in: {:?}", text);
        assert!(text.contains("|---|---|"), "in: {:?}", text);
        assert!(text.contains("| 苹果 | 3 |"), "in: {:?}", text);
    }

    #[test]
    fn test_extract_w_t_text_list_and_unknown_style() {
        let xml = r#"<w:document><w:body>\
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>列表项</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val="WhateverCustom"/></w:pPr><w:r><w:t>普通段落</w:t></w:r></w:p>\
</w:body></w:document>"#;
        let text = extract_w_t_text(xml);
        assert!(text.contains("- 列表项"), "in: {:?}", text);
        // 未知样式降级纯文本：不加 # 也不加 -
        assert!(text.lines().any(|l| l == "普通段落"), "in: {:?}", text);
    }

    #[test]
    fn test_extract_w_t_text_mixed_order_and_empty_para() {
        let xml = r#"<w:document><w:body>\
<w:p><w:r><w:t>开头</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr></w:p>\
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>H</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
<w:p><w:r><w:t>结尾</w:t></w:r></w:p>\
</w:body></w:document>"#;
        let text = extract_w_t_text(xml);
        let lines: Vec<&str> = text.lines().collect();
        // 空标题段不产生空行，块按文档顺序排列
        assert_eq!(
            lines,
            vec!["开头", "| H |", "|---|", "结尾"],
            "in: {:?}",
            text
        );
    }

    // ── pptx 备注 / 图片清单 ──

    #[test]
    fn test_parse_slide_rels() {
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>\
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>\
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>\
</Relationships>"#;
        let (notes, images) = parse_slide_rels(rels);
        assert_eq!(notes.as_deref(), Some("../notesSlides/notesSlide1.xml"));
        assert_eq!(images, vec!["../media/image1.png".to_string()]);
    }

    #[test]
    fn test_normalize_zip_path() {
        assert_eq!(
            normalize_zip_path("ppt/slides", "../notesSlides/notesSlide1.xml"),
            "ppt/notesSlides/notesSlide1.xml"
        );
        assert_eq!(
            normalize_zip_path("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            normalize_zip_path("ppt/slides", "/ppt/media/x.png"),
            "ppt/media/x.png"
        );
    }

    #[test]
    fn test_slide_number() {
        assert_eq!(slide_number("ppt/slides/slide2.xml"), 2);
        assert_eq!(slide_number("ppt/slides/slide10.xml"), 10);
        assert!(slide_number("ppt/slides/slide2.xml") < slide_number("ppt/slides/slide10.xml"));
    }

    /// 生成 w×h 的黑色 PNG 字节（image crate 已有 png 特性，合成 fixture 不依赖外部文件）
    fn make_png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 构造合成 pptx：slide1（纯文本，无 rels）、slide2（备注+图片）、slide10（纯文本），
    /// 故意按 slide2 → slide10 → slide1 写入 zip，验证数字排序。调用方负责删除。
    fn make_synthetic_pptx() -> std::path::PathBuf {
        use std::io::Write;
        let path = unique_test_file("nuphus_test", "pptx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        let slide = |text: &str| {
            format!(
                r#"<p:sld><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
                text
            )
        };

        zw.start_file("ppt/slides/slide2.xml", opts).unwrap();
        zw.write_all(slide("第二页").as_bytes()).unwrap();
        zw.start_file("ppt/slides/_rels/slide2.xml.rels", opts)
            .unwrap();
        zw.write_all(br#"<Relationships><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#).unwrap();
        zw.start_file("ppt/notesSlides/notesSlide2.xml", opts)
            .unwrap();
        zw.write_all(r#"<p:notes><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>记得喝水</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#.as_bytes()).unwrap();
        zw.start_file("ppt/media/image1.png", opts).unwrap();
        zw.write_all(&make_png_bytes(3, 2)).unwrap();

        zw.start_file("ppt/slides/slide10.xml", opts).unwrap();
        zw.write_all(slide("第十页").as_bytes()).unwrap();

        zw.start_file("ppt/slides/slide1.xml", opts).unwrap();
        zw.write_all(slide("第一页").as_bytes()).unwrap();

        zw.finish().unwrap();
        path
    }

    #[test]
    fn test_read_pptx_notes_images_and_numeric_order() {
        let path = make_synthetic_pptx();
        let out = read_pptx(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);

        // header 格式不变
        assert!(out.contains("(pptx parsed, 3 slides)"), "in: {:?}", out);
        // 数字排序：Slide 1 → Slide 2 → Slide 10（zip 写入序为 2/10/1）
        let i1 = out.find("## Slide 1\n").unwrap();
        let i2 = out.find("## Slide 2\n").unwrap();
        let i10 = out.find("## Slide 10\n").unwrap();
        assert!(i1 < i2 && i2 < i10, "in: {:?}", out);
        // 备注与图片附在 Slide 2 段内
        assert!(out.contains("> 备注: 记得喝水"), "in: {:?}", out);
        assert!(out.contains("> 图片: image1.png (3x2,"), "in: {:?}", out);
    }

    #[test]
    fn test_read_pptx_slide_without_rels_no_quote_lines() {
        let path = make_synthetic_pptx();
        let out = read_pptx(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);

        // 无 rels 的页（Slide 1 / Slide 10）不产生 '> 备注:' / '> 图片:' 引用行
        let s1_start = out.find("## Slide 1\n").unwrap();
        let s1_end = out.find("## Slide 2\n").unwrap();
        let section1 = &out[s1_start..s1_end];
        assert!(!section1.contains("> 备注:"), "in: {:?}", section1);
        assert!(!section1.contains("> 图片:"), "in: {:?}", section1);
        assert!(section1.contains("第一页"), "in: {:?}", section1);
    }

    #[test]
    fn test_extract_odf_text() {
        let xml = r#"<office:text><text:p><text:span>Para 1</text:span></text:p><text:p>Para 2</text:p></office:text>"#;
        let text = extract_odf_text(xml, "<text:p>", "</text:p>");
        assert!(text.contains("Para 1"));
        assert!(text.contains("Para 2"));
    }

    /// 构造一个无文本层的单页空白 PDF（模拟扫描件的文本提取结果），
    /// 返回文件路径。调用方负责删除。
    fn make_blank_pdf() -> std::path::PathBuf {
        use lopdf::{Dictionary, Document, Object};
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let page_id = doc.new_object_id();
        let catalog_id = doc.add_object(Dictionary::from_iter(vec![
            ("Type", Object::Name(b"Catalog".to_vec())),
            ("Pages", Object::Reference(pages_id)),
        ]));
        doc.objects.insert(
            pages_id,
            Object::Dictionary(Dictionary::from_iter(vec![
                ("Type", Object::Name(b"Pages".to_vec())),
                ("Kids", Object::Array(vec![Object::Reference(page_id)])),
                ("Count", Object::Integer(1)),
            ])),
        );
        doc.objects.insert(
            page_id,
            Object::Dictionary(Dictionary::from_iter(vec![
                ("Type", Object::Name(b"Page".to_vec())),
                ("Parent", Object::Reference(pages_id)),
                (
                    "MediaBox",
                    Object::Array(vec![
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Integer(612),
                        Object::Integer(792),
                    ]),
                ),
            ])),
        );
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let path = unique_test_file("nuphus_test_blank_pdf", "pdf");
        doc.save(&path).expect("写出空白测试 PDF 失败");
        path
    }

    /// 桥未注入（纯 lib/测试环境）时，扫描件路径应降级为
    /// 「原错误 + OCR 兜底不可用: 原因」，不得静默成功或丢失原因。
    #[test]
    fn test_read_pdf_scanned_fallback_bridge_absent() {
        let path = make_blank_pdf();
        let result = read_pdf(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        let err = result.expect_err("空白 PDF 应走扫描件兜底并失败（桥未注入）");
        assert!(err.contains("纯扫描件"), "err: {err}");
        assert!(err.contains("OCR 兜底不可用"), "err: {err}");
    }

    // ── pdf.js 桥路径：页分类与头部格式（纯函数，不触碰 OnceLock 桥）──

    #[test]
    fn test_classify_pdf_pages_all_text() {
        let texts = vec!["第一页".to_string(), "page two".to_string()];
        let (text, ocr) = classify_pdf_pages(&texts);
        assert_eq!(text, vec![1, 2]);
        assert!(ocr.is_empty());
    }

    #[test]
    fn test_classify_pdf_pages_all_blank() {
        let texts = vec![String::new(), "  \n ".to_string(), "\t".to_string()];
        let (text, ocr) = classify_pdf_pages(&texts);
        assert!(text.is_empty());
        assert_eq!(ocr, vec![1, 2, 3]);
    }

    #[test]
    fn test_classify_pdf_pages_mixed() {
        let texts = vec![
            "hello".to_string(),
            String::new(),
            "world".to_string(),
            "   ".to_string(),
        ];
        let (text, ocr) = classify_pdf_pages(&texts);
        assert_eq!(text, vec![1, 3]);
        assert_eq!(ocr, vec![2, 4]);
    }

    #[test]
    fn test_classify_pdf_pages_empty_input() {
        let (text, ocr) = classify_pdf_pages(&[]);
        assert!(text.is_empty());
        assert!(ocr.is_empty());
    }

    #[test]
    fn test_pdf_bridge_header() {
        assert_eq!(
            pdf_bridge_header("a.pdf", 100, 3, 0),
            "a.pdf (pdf parsed, 100 chars, 3 pages)"
        );
        assert_eq!(
            pdf_bridge_header("a.pdf", 100, 3, 1),
            "a.pdf (pdf mixed, 100 chars, 3 pages, 1 pages OCR)"
        );
    }
}
