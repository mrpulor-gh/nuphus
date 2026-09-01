// tools/pdf.rs — PDF 内部机制命令（工具页手动调用，非 agent 工具）
//
// 移植自 nuphus-tools-rs engines/pdf.rs，修复已知缺陷：
// - 路径 to_str().unwrap() → to_string_lossy + 错误传播，杜绝 panic
// - extract_text 不再手写 Tj 玩具解析：直接用 lopdf 官方 extract_text
//   （内部处理 TJ 数组 / 十六进制串 / 引号操作符），逐页返回带页码分隔
// - 命令全部 Result<Value, String> 返回，错误信息中文可读，不 panic
//
// lopdf 0.34 与 nuphus core（src/utils/office.rs read_pdf）共用同一依赖。

use std::collections::BTreeMap;
use std::path::PathBuf;

use lopdf::{Bookmark, Dictionary, Document, Object, ObjectId, Stream};
use lopdf::xobject;

/// 单文件大小上限：防超大 PDF 加载耗尽内存（500MB，超出提示用系统程序）
const PDF_MAX_BYTES: u64 = 500 * 1024 * 1024;
/// extract_text 默认最大页数（防全本超长提取拖死命令）
const EXTRACT_MAX_PAGES: u32 = 200;
/// 图片转 PDF：单图像素上限（防超大图嵌入耗尽内存）
const IMG_MAX_PIXELS: u64 = 50_000_000;
/// 图片转 PDF：输入图片扩展名白名单（image 0.25 已开启的 decode feature）
const IMG_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];

fn ensure_input_file(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_file() {
        return Err(format!("PDF 文件不存在：{}", path));
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("读取 PDF 信息失败：{}", e))?;
    if meta.len() > PDF_MAX_BYTES {
        return Err(format!(
            "PDF 超过 {}MB 上限，请改用系统程序处理",
            PDF_MAX_BYTES / 1024 / 1024
        ));
    }
    Ok(p)
}

/// 校验输出路径：目录存在、扩展名为 .pdf（小写比较）
fn ensure_output_pdf(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "pdf" {
        return Err("输出路径扩展名必须是 .pdf".to_string());
    }
    if let Some(dir) = p.parent() {
        if !dir.as_os_str().is_empty() && !dir.is_dir() {
            return Err(format!("输出目录不存在：{}", dir.display()));
        }
    }
    Ok(p)
}

/// 合并多个 PDF 为单个文件（lopdf 官方合并方式：renumber + 重建 Catalog/Pages）
#[tauri::command]
pub fn pdf_merge(input_paths: Vec<String>, output_path: String) -> Result<serde_json::Value, String> {
    if input_paths.is_empty() {
        return Err("至少需要选择一个 PDF 文件".to_string());
    }
    let output = ensure_output_pdf(&output_path)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }

    // 预校验所有输入
    let mut inputs = Vec::with_capacity(input_paths.len());
    for path in &input_paths {
        inputs.push(ensure_input_file(path)?);
    }

    // 加载全部文档
    let mut documents = Vec::with_capacity(inputs.len());
    for p in &inputs {
        let doc = Document::load(p).map_err(|e| format!("加载 PDF 失败（{}）：{}", p.display(), e))?;
        documents.push(doc);
    }

    // ── 合并（对齐 lopdf 官方示例与 tools-rs 实现，修掉全部 unwrap）──
    let mut max_id = 1u32;
    let mut page_num = 1u32;
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for mut doc in documents {
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        let mut first_page = true;
        for (_, object_id) in doc.get_pages() {
            if first_page {
                let bookmark = Bookmark::new(
                    format!("Page_{}", page_num),
                    [0.0, 0.0, 1.0],
                    0,
                    object_id,
                );
                document.add_bookmark(bookmark, None);
                first_page = false;
                page_num += 1;
            }
            let obj = doc
                .get_object(object_id)
                .map_err(|e| format!("读取页对象失败：{}", e))?
                .to_owned();
            documents_pages.insert(object_id, obj);
        }
        documents_objects.extend(doc.objects);
    }

    // Catalog 与 Pages 是合并后文档的强制根
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    for (object_id, object) in documents_objects.iter() {
        match object.type_name().unwrap_or("") {
            "Catalog" => {
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object { id } else { *object_id },
                    object.clone(),
                ));
            }
            "Pages" => {
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, old)) = &pages_object {
                        if let Ok(old_dict) = old.as_dict() {
                            dictionary.extend(old_dict);
                        }
                    }
                    pages_object = Some((
                        if let Some((id, _)) = pages_object { id } else { *object_id },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            "Page" => {}     // 单独处理
            "Outlines" => {} // 忽略
            "Outline" => {}  // 忽略
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    let pages_id = pages_object
        .as_ref()
        .ok_or_else(|| "输入文档中未找到 Pages 根对象".to_string())?
        .0;

    // 所有 Page 对象指向合并后的 Pages 根
    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_id);
            document
                .objects
                .insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    let catalog_id = catalog_object
        .as_ref()
        .ok_or_else(|| "输入文档中未找到 Catalog 根对象".to_string())?
        .0;

    // 重建 Pages：Count + Kids（全部合并页）
    if let Some((_, obj)) = pages_object {
        if let Ok(dictionary) = obj.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Count", documents_pages.len() as u32);
            dictionary.set(
                "Kids",
                documents_pages
                    .keys()
                    .map(|id| Object::Reference(*id))
                    .collect::<Vec<_>>(),
            );
            document.objects.insert(pages_id, Object::Dictionary(dictionary));
        }
    }

    // 重建 Catalog：Pages 指向合并根，清除旧 Outlines
    if let Some((_, obj)) = catalog_object {
        if let Ok(dictionary) = obj.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Pages", pages_id);
            dictionary.remove(b"Outlines");
            document.objects.insert(catalog_id, Object::Dictionary(dictionary));
        }
    }

    // 重建书签大纲并挂到 Catalog
    if let Some(outline_id) = document.build_outline() {
        if let Ok(Object::Dictionary(dict)) = document.get_object_mut(catalog_id) {
            dict.set("Outlines", Object::Reference(outline_id));
        }
    }

    document.compress();
    document
        .save(&output)
        .map_err(|e| format!("保存合并 PDF 失败：{}", e))?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "pages": documents_pages.len(),
        "sources": input_paths.len(),
    }))
}

/// 压缩 PDF：清理未引用对象（prune_objects）后重写
#[tauri::command]
pub fn pdf_compress(input_path: String, output_path: String) -> Result<serde_json::Value, String> {
    let input = ensure_input_file(&input_path)?;
    let output = ensure_output_pdf(&output_path)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }

    let mut doc = Document::load(&input).map_err(|e| format!("加载 PDF 失败：{}", e))?;
    let pages_before = doc.get_pages().len();
    let removed = doc.prune_objects().len();
    doc.save(&output).map_err(|e| format!("保存压缩 PDF 失败：{}", e))?;

    let size_before = std::fs::metadata(&input)
        .map(|m| m.len())
        .unwrap_or(0);
    let size_after = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "pages": pages_before,
        "removed_objects": removed,
        "size_before": size_before,
        "size_after": size_after,
        "saved_bytes": size_before.saturating_sub(size_after),
    }))
}

/// 获取 PDF 页数
#[tauri::command]
pub fn pdf_page_count(path: String) -> Result<serde_json::Value, String> {
    let input = ensure_input_file(&path)?;
    let doc = Document::load(&input).map_err(|e| format!("加载 PDF 失败：{}", e))?;
    Ok(serde_json::json!({
        "path": input.display().to_string(),
        "pages": doc.get_pages().len(),
    }))
}

/// 提取 PDF 文本（逐页，页码分隔）。max_pages 缺省 200 页，防超长文档拖死命令。
/// 使用 lopdf 官方 extract_text：覆盖 TJ 数组 / 十六进制串 / 引号操作符，
/// 替代 tools-rs 只解析 `(...) Tj` 的玩具级实现。
#[tauri::command]
pub fn pdf_extract_text(
    path: String,
    max_pages: Option<u32>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_file(&path)?;
    let doc = Document::load(&input).map_err(|e| format!("加载 PDF 失败：{}", e))?;

    let pages = doc.get_pages();
    let limit = max_pages.unwrap_or(EXTRACT_MAX_PAGES).max(1);
    let mut text = String::new();
    let mut extracted_pages = 0u32;

    for (page_num, _) in pages.iter().take(limit as usize) {
        match doc.extract_text(&[*page_num]) {
            Ok(page_text) => {
                extracted_pages += 1;
                text.push_str(&format!("--- Page {} ---\n", page_num));
                text.push_str(page_text.trim());
                text.push('\n');
            }
            Err(e) => {
                text.push_str(&format!("--- Page {} ---\n[提取失败: {}]\n", page_num, e));
            }
        }
    }

    let truncated = pages.len() > limit as usize;
    Ok(serde_json::json!({
        "path": input.display().to_string(),
        "pages": pages.len(),
        "extracted_pages": extracted_pages,
        "truncated": truncated,
        "text": text,
    }))
}
/// 图片转 PDF：多张图片各占一页（页面尺寸 = 图片像素，1px:1pt），
/// 经 lopdf embed_image 嵌入图片流（JPEG 走 DCTDecode，其余 FlateDecode）。
#[tauri::command]
pub fn pdf_images_to_pdf(
    input_paths: Vec<String>,
    output_path: String,
) -> Result<serde_json::Value, String> {
    if input_paths.is_empty() {
        return Err("至少需要选择一张图片".to_string());
    }
    let output = ensure_output_pdf(&output_path)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }

    // 预检所有输入图片（存在 / 扩展名白名单 / 像素上限）
    let mut dims: Vec<(u32, u32)> = Vec::with_capacity(input_paths.len());
    for p in &input_paths {
        let path = PathBuf::from(p);
        if !path.is_file() {
            return Err(format!("图片文件不存在：{}", p));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !IMG_EXTS.contains(&ext.as_str()) {
            return Err(format!("不支持的图片格式：{}", ext));
        }
        let (w, h) = image::image_dimensions(&path)
            .map_err(|e| format!("读取图片尺寸失败（{}）：{}", p, e))?;
        if (w as u64) * (h as u64) > IMG_MAX_PIXELS {
            return Err(format!(
                "图片分辨率 {}×{} 超过 {} 万像素上限",
                w,
                h,
                IMG_MAX_PIXELS / 1_000_000
            ));
        }
        dims.push((w, h));
    }

    // 创建空文档：Pages root + 每图一页（空内容流占位，insert_image 会追加绘图操作）
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page_ids: Vec<ObjectId> = Vec::with_capacity(dims.len());
    for (w, h) in &dims {
        let content_id = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(pages_id));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(*w as i64),
                Object::Integer(*h as i64),
            ]),
        );
        page.set("Contents", Object::Reference(content_id));
        page_ids.push(doc.add_object(Object::Dictionary(page)));
    }

    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set(
        "Kids",
        Object::Array(
            page_ids
                .iter()
                .map(|id| Object::Reference(*id))
                .collect::<Vec<_>>(),
        ),
    );
    pages.set("Count", Object::Integer(page_ids.len() as i64));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));

    // 逐页嵌入图片（整页铺满，原点在左下）
    for (i, p) in input_paths.iter().enumerate() {
        let img = xobject::image(PathBuf::from(p))
            .map_err(|e| format!("嵌入图片失败（{}）：{}", p, e))?;
        let (w, h) = dims[i];
        doc.insert_image(page_ids[i], img, (0.0, 0.0), (w as f32, h as f32))
            .map_err(|e| format!("写入图片页失败（{}）：{}", p, e))?;
    }

    doc.compress();
    doc.save(&output)
        .map_err(|e| format!("保存 PDF 失败：{}", e))?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "pages": page_ids.len(),
        "sources": input_paths.len(),
    }))
}

/// 提取 PDF 指定页：按 1-based 页码抽取到新 PDF（页面与依赖对象保留，
/// 旧 Pages/Catalog 由 prune 清理）。
#[tauri::command]
pub fn pdf_extract_pages(
    input_path: String,
    pages: Vec<u32>,
    output_path: String,
) -> Result<serde_json::Value, String> {
    if pages.is_empty() {
        return Err("请至少指定一页（如 1,3,5）".to_string());
    }
    let input = ensure_input_file(&input_path)?;
    let output = ensure_output_pdf(&output_path)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }

    let mut doc = Document::load(&input).map_err(|e| format!("加载 PDF 失败：{}", e))?;
    doc.renumber_objects_with(1);
    let all_pages = doc.get_pages();
    let max_page = all_pages.len() as u32;
    for p in &pages {
        if *p == 0 || *p > max_page {
            return Err(format!("页码 {} 超出范围（1-{}）", p, max_page));
        }
    }
    // 去重保序
    let mut uniq = pages.clone();
    uniq.sort_unstable();
    uniq.dedup();
    let selected: Vec<ObjectId> = uniq
        .iter()
        .filter_map(|n| all_pages.get(n).copied())
        .collect();
    if selected.is_empty() {
        return Err("未找到有效页面".to_string());
    }

    // 重建：拷贝全部对象（引用关系已随 renumber 一致），选中页 Parent 指向新根
    let mut out = Document::with_version("1.5");
    let pages_id = out.new_object_id();
    out.objects.extend(doc.objects.iter().map(|(k, v)| (*k, v.clone())));
    for pid in &selected {
        if let Some(Object::Dictionary(dict)) = out.objects.get_mut(pid) {
            dict.set("Parent", Object::Reference(pages_id));
        }
    }
    let mut pages_dict = Dictionary::new();
    pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
    pages_dict.set(
        "Kids",
        Object::Array(
            selected
                .iter()
                .map(|id| Object::Reference(*id))
                .collect::<Vec<_>>(),
        ),
    );
    pages_dict.set("Count", Object::Integer(selected.len() as i64));
    out.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = out.add_object(Object::Dictionary(catalog));
    out.trailer.set("Root", Object::Reference(catalog_id));

    // 清理旧 Pages/Catalog 等未引用对象后压缩保存
    out.prune_objects();
    out.compress();
    out.save(&output).map_err(|e| format!("保存 PDF 失败：{}", e))?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "pages": selected.len(),
        "requested": uniq.len(),
    }))
}

/// 旋转 PDF 页面：degrees 90/180/270（累积到现有 /Rotate）。
/// pages 缺省 = 全部页；提供时仅旋转指定 1-based 页。
#[tauri::command]
pub fn pdf_rotate(
    input_path: String,
    output_path: String,
    degrees: u32,
    pages: Option<Vec<u32>>,
) -> Result<serde_json::Value, String> {
    if degrees != 90 && degrees != 180 && degrees != 270 {
        return Err("旋转角度仅支持 90 / 180 / 270".to_string());
    }
    let input = ensure_input_file(&input_path)?;
    let output = ensure_output_pdf(&output_path)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }

    let mut doc = Document::load(&input).map_err(|e| format!("加载 PDF 失败：{}", e))?;
    let all_pages = doc.get_pages();
    let max_page = all_pages.len() as u32;

    let targets: Vec<ObjectId> = match &pages {
        Some(list) => {
            for p in list {
                if *p == 0 || *p > max_page {
                    return Err(format!("页码 {} 超出范围（1-{}）", p, max_page));
                }
            }
            list.iter().filter_map(|n| all_pages.get(n).copied()).collect()
        }
        None => all_pages.values().copied().collect(),
    };
    if targets.is_empty() {
        return Err("未找到有效页面".to_string());
    }

    let mut rotated = 0usize;
    for pid in &targets {
        if let Some(Object::Dictionary(dict)) = doc.objects.get_mut(pid) {
            let cur = dict
                .get(b"Rotate")
                .ok()
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0);
            dict.set("Rotate", Object::Integer((cur + degrees as i64) % 360));
            rotated += 1;
        }
    }
    doc.save(&output).map_err(|e| format!("保存 PDF 失败：{}", e))?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "degrees": degrees,
        "rotated_pages": rotated,
        "total_pages": all_pages.len(),
    }))
}