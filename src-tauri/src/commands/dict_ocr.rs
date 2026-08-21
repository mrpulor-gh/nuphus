use serde_json::{json, Value};
use std::path::PathBuf;

fn dict_dir() -> PathBuf {
    std::env::var("NUPHUS_DICT_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var("NUPHUS_DATA_DIR")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::data_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join("Nuphus")
                });
            base.join("dicts")
        })
}

/// 校验字库名并返回其 .dict 路径，防止路径穿越（前端输入不可信）。
/// 允许 Unicode 字母数字（支持中文名）+ `_`/`-`，拒绝 `.` 与路径分隔符。
fn resolve_dict_path(dict_name: &str) -> Result<PathBuf, String> {
    if dict_name.is_empty()
        || dict_name.len() > 64
        || !dict_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("非法字库名: '{}'", dict_name));
    }
    Ok(dict_dir().join(format!("{}.dict", dict_name)))
}

fn load_image_rgb(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path).map_err(|e| format!("打开图片失败: {e}"))?;
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    Ok((rgb.into_raw(), w, h))
}

#[tauri::command]
pub async fn dict_ocr_analyze(image_path: String) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let analysis = nuphus::desktop::dict_ocr::analyze_region(&pixels, w, h);
    Ok(json!({
        "foreground": {
            "color": analysis.foreground.to_hex(),
            "r": analysis.foreground.r,
            "g": analysis.foreground.g,
            "b": analysis.foreground.b,
            "dr": analysis.foreground.dr,
            "dg": analysis.foreground.dg,
            "db": analysis.foreground.db,
        },
        "background": {
            "color": analysis.background.to_hex(),
            "r": analysis.background.r,
            "g": analysis.background.g,
            "b": analysis.background.b,
        },
        "fg_pixels": analysis.fg_pixels,
        "bg_pixels": analysis.bg_pixels,
    }))
}

#[tauri::command]
pub async fn dict_ocr_binarize_preview(
    image_path: String,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);
    let rgba = nuphus::desktop::dict_ocr::binarize_preview(&pixels, w, h, &fg);
    use base64::Engine;
    let mut png_buf = std::io::Cursor::new(Vec::new());
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("创建RGBA图像失败")?;
    img.write_to(&mut png_buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG编码失败: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
    Ok(json!({
        "width": w,
        "height": h,
        "data": b64,
        "pixel_count": (w * h),
    }))
}

#[tauri::command]
pub async fn dict_ocr_extract(
    image_path: String,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
    min_col_gap: Option<u32>,
    min_row_gap: Option<u32>,
    word_gap: Option<u32>,
    line_height: Option<u32>,
) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);
    let binary = nuphus::desktop::dict_ocr::binarize(&pixels, w, h, &fg);

    let params = nuphus::desktop::dict_ocr::SegParams {
        min_col_gap: min_col_gap.unwrap_or(2),
        min_row_gap: min_row_gap.unwrap_or(2),
        word_gap: word_gap.unwrap_or(4),
        line_height: line_height.unwrap_or(0),
    };

    let segs = nuphus::desktop::dict_ocr::segment(&binary, w, h, &params);

    let segments: Vec<Value> = segs
        .iter()
        .map(|s| {
            let packed = pack_bits(&s.data, s.width as usize);
            json!({
                "x": s.x,
                "y": s.y,
                "width": s.width,
                "height": s.height,
                "data_hex": hex::encode(&packed),
            })
        })
        .collect();

    Ok(json!({
        "width": w,
        "height": h,
        "segments": segments,
        "count": segments.len(),
    }))
}

#[tauri::command]
pub async fn dict_ocr_recognize(
    image_path: String,
    dict_name: String,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
    _min_col_gap: Option<u32>,
    _min_row_gap: Option<u32>,
    word_gap: Option<u32>,
    _line_height: Option<u32>,
    sim: Option<f32>,
    word: Option<String>, // 要查找的词（如"系统"），非空时只查该词含的各字
) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);
    let min_sim = sim.unwrap_or(1.0);

    let dict_path = resolve_dict_path(&dict_name)?;
    if !dict_path.exists() {
        return Err(format!("字库 '{}' 不存在", dict_name));
    }
    let Ok(store) = nuphus::desktop::dict_ocr::store::DictStore::load(&dict_path) else {
        return Err("加载字库失败".to_string());
    };

    let all_templates: Vec<nuphus::desktop::dict_ocr::CharTemplate> =
        store.all().values().flat_map(|v| v.clone()).collect();
    if all_templates.is_empty() {
        return Ok(json!({ "text": "", "matches": [] }));
    }

    // 如果指定了查找词，只搜索该词包含的单字模板
    let word_chars: Option<Vec<char>> = word
        .as_ref()
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().collect());
    let templates_to_search: Vec<&nuphus::desktop::dict_ocr::CharTemplate> =
        if let Some(ref chars) = word_chars {
            all_templates
                .iter()
                .filter(|t| {
                    let tc: Vec<char> = t.char.chars().collect();
                    tc.len() == 1 && chars.contains(&tc[0])
                })
                .collect()
        } else {
            all_templates.iter().collect()
        };
    if templates_to_search.is_empty() {
        return Ok(json!({ "text": "", "matches": [] }));
    }

    // 滑动窗口匹配：在整图上滑动每个模板，不依赖字符分割
    struct Hit {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        sim: f32,
        ch: String,
    }
    let mut hits: Vec<Hit> = Vec::new();

    for tmpl in &templates_to_search {
        let results = nuphus::desktop::dict_ocr::search_screen(&pixels, w, h, tmpl, &fg, min_sim);
        for r in results {
            hits.push(Hit {
                x: r.x,
                y: r.y,
                w: r.width,
                h: r.height,
                sim: r.confidence,
                ch: tmpl.char.clone(),
            });
        }
    }

    // 去重：位置重叠的只保留最高相似度
    hits.sort_by(|a, b| {
        b.sim
            .partial_cmp(&a.sim)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut deduped: Vec<Hit> = Vec::new();
    for h in &hits {
        let overlap = deduped.iter().any(|d: &Hit| {
            let dx = (h.x - d.x).unsigned_abs();
            let dy = (h.y - d.y).unsigned_abs();
            dx < (h.w + d.w) / 4 && dy < (h.h + d.h) / 4
        });
        if !overlap {
            deduped.push(Hit {
                x: h.x,
                y: h.y,
                w: h.w,
                h: h.h,
                sim: h.sim,
                ch: h.ch.clone(),
            });
        }
    }

    // 按 x 排序
    deduped.sort_by_key(|a| a.x);

    let full_text: String = deduped.iter().map(|h| h.ch.as_str()).collect();

    // ── 词匹配模式：在去重排序后的匹配结果中扫描目标词 ──
    if let Some(ref target_chars) = word_chars {
        let word_str = word.as_deref().unwrap_or("");
        let max_x_interval = {
            let max_tmpl_w = all_templates
                .iter()
                .map(|t| t.width as u32)
                .max()
                .unwrap_or(20);
            let word_gap_val = if let Some(gap) = word_gap {
                if gap > 0 {
                    gap
                } else {
                    let binary = nuphus::desktop::dict_ocr::binarize(&pixels, w, h, &fg);
                    let (_, _, wg) = nuphus::desktop::dict_ocr::auto_detect_gaps(&binary, w, h);
                    wg
                }
            } else {
                let binary = nuphus::desktop::dict_ocr::binarize(&pixels, w, h, &fg);
                let (_, _, wg) = nuphus::desktop::dict_ocr::auto_detect_gaps(&binary, w, h);
                wg
            };
            (word_gap_val + max_tmpl_w) as i32
        };
        let matches = deduped; // reuse as word_match candidates
        let mut word_matches = Vec::new();
        let mut i = 0;
        while i + target_chars.len() <= matches.len() {
            let mut all_match = true;
            for (k, tc) in target_chars.iter().enumerate() {
                let fc: Vec<char> = matches[i + k].ch.chars().collect();
                if fc.len() != 1 || fc[0] != *tc {
                    all_match = false;
                    break;
                }
                if k > 0 {
                    let interval = matches[i + k].x - matches[i + k - 1].x;
                    if interval <= 0 || interval > max_x_interval {
                        all_match = false;
                        break;
                    }
                }
            }
            if all_match {
                let slice = &matches[i..i + target_chars.len()];
                let min_x = slice.iter().map(|m| m.x).min().unwrap_or(0);
                let min_y = slice.iter().map(|m| m.y).min().unwrap_or(0);
                let max_x = slice.iter().map(|m| m.x + m.w as i32).max().unwrap_or(0);
                let max_y = slice.iter().map(|m| m.y + m.h as i32).max().unwrap_or(0);
                let avg_sim = slice.iter().map(|m| m.sim).sum::<f32>() / slice.len() as f32;

                word_matches.push(json!({
                    "word": word_str,
                    "sim": avg_sim,
                    "x": min_x,
                    "y": min_y,
                    "width": (max_x - min_x) as u32,
                    "height": (max_y - min_y) as u32,
                    "char_count": target_chars.len(),
                }));
                i += target_chars.len();
            } else {
                i += 1;
            }
        }

        return Ok(json!({
            "text": full_text,
            "word_matches": word_matches,
            "match_count": word_matches.len(),
            "is_word_match": true,
        }));
    }

    // ── 无 word：返回所有识别到的单字 ──
    let matches: Vec<Value> = deduped
        .into_iter()
        .map(|h| {
            json!({
                "char": h.ch,
                "sim": h.sim,
                "x": h.x,
                "y": h.y,
                "width": h.w,
                "height": h.h,
            })
        })
        .collect();

    Ok(json!({
        "text": full_text,
        "matches": matches,
    }))
}

#[tauri::command]
pub async fn dict_list() -> Result<Value, String> {
    let dir = dict_dir();
    let names = nuphus::desktop::dict_ocr::list_dicts(&dir)
        .map_err(|e| format!("读取字库目录失败: {e}"))?;
    Ok(json!(names))
}

#[tauri::command]
pub async fn dict_load(dict_name: String) -> Result<Value, String> {
    let dict_path = resolve_dict_path(&dict_name)?;
    let store = nuphus::desktop::dict_ocr::load_dict(&dict_path)
        .map_err(|e| format!("加载字库失败: {e}"))?;

    let entries: Vec<Value> = store
        .all()
        .iter()
        .flat_map(|(ch, templates)| {
            templates.iter().map(move |t| {
                json!({
                    "char": ch,
                    "width": t.width,
                    "height": t.height,
                    "data_hex": hex::encode(&t.data),
                })
            })
        })
        .collect();

    Ok(json!({
        "name": store.name,
        "entries": entries,
        "count": entries.len(),
    }))
}

#[tauri::command]
pub async fn dict_delete(dict_name: String) -> Result<Value, String> {
    let path = resolve_dict_path(&dict_name)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除字库失败: {e}"))?;
    }
    Ok(json!({ "deleted": true }))
}

#[tauri::command]
pub async fn dict_ocr_auto_gaps(
    image_path: String,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);
    let binary = nuphus::desktop::dict_ocr::binarize(&pixels, w, h, &fg);
    let (col_gap, row_gap, word_gap) = nuphus::desktop::dict_ocr::auto_detect_gaps(&binary, w, h);
    Ok(json!({ "col_gap": col_gap, "row_gap": row_gap, "word_gap": word_gap }))
}

#[tauri::command]
pub async fn dict_ocr_save_char(
    image_path: String,
    dict_name: String,
    char: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
) -> Result<Value, String> {
    let (pixels, img_w, img_h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);
    let binary = nuphus::desktop::dict_ocr::binarize(&pixels, img_w, img_h, &fg);

    let x = x.min(img_w - 1);
    let y = y.min(img_h - 1);
    let w = width.min(img_w - x).max(1);
    let h = height.min(img_h - y).max(1);

    let mut char_data = vec![0u8; (w * h) as usize];
    for row in 0..h {
        let src = ((y + row) * img_w + x) as usize;
        let dst = (row * w) as usize;
        if dst + w as usize <= binary.len() && src + w as usize <= binary.len() {
            char_data[dst..dst + w as usize].copy_from_slice(&binary[src..src + w as usize]);
        }
    }

    let packed = pack_bits(&char_data, w as usize);
    let template = nuphus::desktop::dict_ocr::CharTemplate {
        char: char.clone(),
        width: w as u8,
        height: h as u8,
        data: packed,
        grayscale: vec![],
    };

    let dir = dict_dir();
    let path = resolve_dict_path(&dict_name)?;
    let mut store = if path.exists() {
        nuphus::desktop::dict_ocr::store::DictStore::load(&path)
            .map_err(|e| format!("加载字库失败: {e}"))?
    } else {
        nuphus::desktop::dict_ocr::store::DictStore::new(&dict_name, &dir)
    };

    store.add(&char, vec![template]);
    store.save().map_err(|e| format!("保存字库失败: {e}"))?;

    Ok(json!({ "saved": true, "char": char, "width": w, "height": h }))
}

#[tauri::command]
pub async fn read_image_base64(image_path: String) -> Result<Value, String> {
    use base64::Engine;
    // 安全：前端可触达，仅允许图片扩展名 + 50MB 上限（聊天拖拽/OCR 预览的合法用途保留，
    // 阻止读取私钥、文档等任意文件）
    let ext = std::path::Path::new(&image_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "ico"
    ) {
        return Err(format!("仅支持图片文件: {}", image_path));
    }
    let meta = std::fs::metadata(&image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    if meta.len() > 50 * 1024 * 1024 {
        return Err("图片超过 50MB 上限".to_string());
    }
    let data = std::fs::read(&image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" => "image/bmp",
        _ => "image/png",
    };
    Ok(json!({ "base64": b64, "mime": mime }))
}

#[tauri::command]
pub async fn dict_remove_char(dict_name: String, char: String) -> Result<Value, String> {
    let path = resolve_dict_path(&dict_name)?;
    if !path.exists() {
        return Err(format!("字库 '{}' 不存在", dict_name));
    }
    let mut store = nuphus::desktop::dict_ocr::store::DictStore::load(&path)
        .map_err(|e| format!("加载字库失败: {e}"))?;
    store.remove(&char);
    store.save().map_err(|e| format!("保存字库失败: {e}"))?;
    Ok(json!({ "removed": true, "char": char }))
}

/// 列出所有已保存的字库
#[tauri::command]
pub async fn dict_ocr_list_dicts() -> Result<Value, String> {
    let dir = dict_dir();
    let dicts = nuphus::desktop::dict_ocr::list_all_dicts(&dir);
    let list: Vec<Value> = dicts
        .iter()
        .map(|(name, path)| {
            let metadata = std::fs::metadata(path).ok();
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = metadata
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    use std::time::SystemTime;
                    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
                    dur.as_secs()
                })
                .unwrap_or(0);
            json!({
                "name": name,
                "size": size,
                "modified": modified,
            })
        })
        .collect();
    Ok(json!(list))
}

/// 自动识别（类似大漠 FindStr 模式）：先 Ocr 全区域识字，再输出匹配结果
#[tauri::command]
pub async fn dict_ocr_auto_match(
    image_path: String,
    r: u8,
    g: u8,
    b: u8,
    dr: u8,
    dg: u8,
    db: u8,
) -> Result<Value, String> {
    let (pixels, w, h) = load_image_rgb(&image_path)?;
    let fg = nuphus::desktop::dict_ocr::ColorSpec::new(r, g, b, dr, dg, db);

    let dir = dict_dir();
    let dicts = nuphus::desktop::dict_ocr::list_all_dicts(&dir);
    if dicts.is_empty() {
        return Err("没有找到已保存的字库".to_string());
    }

    let total = dicts.len();
    let mut scored: Vec<(f32, String, serde_json::Value)> = Vec::new();

    for (dict_name, dict_path) in &dicts {
        let Ok(store) = nuphus::desktop::dict_ocr::store::DictStore::load(dict_path) else {
            continue;
        };

        let all_templates: Vec<nuphus::desktop::dict_ocr::CharTemplate> =
            store.all().values().flat_map(|v| v.clone()).collect();
        if all_templates.is_empty() {
            continue;
        }

        // 滑动窗口匹配每个模板（不依赖分割）
        struct Hit {
            x: i32,
            w: u32,
            sim: f32,
            ch: String,
        }
        let mut hits = Vec::new();

        for tmpl in &all_templates {
            let results = nuphus::desktop::dict_ocr::search_screen(&pixels, w, h, tmpl, &fg, 1.0);
            for r in results {
                hits.push(Hit {
                    x: r.x,
                    w: r.width,
                    sim: r.confidence,
                    ch: tmpl.char.clone(),
                });
            }
        }

        // 去重（重叠位置保留最高相似度）
        hits.sort_by(|a, b| {
            b.sim
                .partial_cmp(&a.sim)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut deduped: Vec<Hit> = Vec::new();
        for h in &hits {
            let overlap = deduped
                .iter()
                .any(|d| (h.x - d.x).unsigned_abs() < (h.w + d.w) / 4);
            if !overlap {
                deduped.push(Hit {
                    x: h.x,
                    w: h.w,
                    sim: h.sim,
                    ch: h.ch.clone(),
                });
            }
        }

        if deduped.is_empty() {
            continue;
        }

        deduped.sort_by_key(|a| a.x);

        let text: String = deduped.iter().map(|h| h.ch.as_str()).collect();
        if text.is_empty() {
            continue;
        }

        let avg_sim = deduped.iter().map(|h| h.sim).sum::<f32>() / deduped.len() as f32;
        scored.push((
            avg_sim,
            dict_name.clone(),
            json!({
                "matched": true,
                "dict_name": dict_name,
                "text": text,
                "avg_sim": avg_sim,
                "total_dicts": total,
                "match_count": deduped.len(),
            }),
        ));
    }

    // 按相似度降序返回最佳结果
    scored.sort_by(|(a, _, _), (b, _, _)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((_conf, _name, result)) = scored.into_iter().next() {
        return Ok(result);
    }

    Ok(json!({
        "matched": false,
        "text": "",
        "dicts_tried": total,
        "total_dicts": total,
    }))
}

fn pack_bits(data: &[u8], row_width: usize) -> Vec<u8> {
    let cols = row_width.div_ceil(8);
    let rows = data.len() / row_width;
    let mut packed = vec![0u8; cols * rows];
    for y in 0..rows {
        for x in 0..row_width {
            if y * row_width + x < data.len() && data[y * row_width + x] != 0 {
                let byte_idx = y * cols + x / 8;
                if byte_idx < packed.len() {
                    packed[byte_idx] |= 1 << (7 - (x % 8));
                }
            }
        }
    }
    packed
}

/// 用当前字库匹配已提取的分段（像素级对比，用于提取后自动预填字符名）
#[tauri::command]
pub async fn dict_ocr_identify_segments(
    dict_name: String,
    segments: Vec<MatchSegmentParam>,
) -> Result<Value, String> {
    let dict_path = dict_dir().join(format!("{}.dict", dict_name));
    if !dict_path.exists() {
        return Ok(json!({ "matches": [] }));
    }
    let store = nuphus::desktop::dict_ocr::load_dict(&dict_path)
        .map_err(|e| format!("加载字库失败: {e}"))?;
    let all_templates: Vec<nuphus::desktop::dict_ocr::CharTemplate> =
        store.all().values().flat_map(|v| v.clone()).collect();
    if all_templates.is_empty() {
        return Ok(json!({ "matches": [] }));
    }

    let mut results = Vec::new();
    for seg in &segments {
        let unpacked = unpack_packed_hex(&seg.data_hex, seg.width, seg.height)?;
        let char_seg = nuphus::desktop::dict_ocr::CharSegment {
            x: seg.x,
            y: seg.y,
            width: seg.width,
            height: seg.height,
            data: unpacked,
        };
        let matches = nuphus::desktop::dict_ocr::match_char(&char_seg, &all_templates);
        let best = matches.into_iter().next();
        results.push(json!({
            "index": seg.index,
            "char": best.as_ref().map(|m| m.char.as_str()).unwrap_or(""),
            "sim": best.as_ref().map(|m| m.confidence).unwrap_or(0.0),
        }));
    }

    Ok(json!({ "matches": results }))
}

#[derive(serde::Deserialize)]
pub struct MatchSegmentParam {
    pub index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data_hex: String,
}

/// 将前端 hex 打包数据解包为逐像素格式（1字节/像素，0或1）
fn unpack_packed_hex(hex_str: &str, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let packed = hex::decode(hex_str).map_err(|e| format!("Hex解码失败: {e}"))?;
    let total = (width * height) as usize;
    let mut unpacked = vec![0u8; total];
    let cols = (width as usize).div_ceil(8);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let byte_idx = y * cols + x / 8;
            let bit = 7 - (x % 8) as u8;
            let val = if byte_idx < packed.len() {
                (packed[byte_idx] >> bit) & 1
            } else {
                0
            };
            unpacked[y * width as usize + x] = val;
        }
    }
    Ok(unpacked)
}

#[tauri::command]
pub async fn save_temp_image(image_b64: String) -> Result<Value, String> {
    use base64::Engine;
    let img_bytes = base64::engine::general_purpose::STANDARD
        .decode(&image_b64)
        .map_err(|e| format!("Base64 解码失败: {e}"))?;

    let temp_dir = std::env::temp_dir().join("nuphus_dict");
    let _ = std::fs::create_dir_all(&temp_dir);
    let file_name = format!(
        "render_{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let temp_path = temp_dir.join(&file_name);
    std::fs::write(&temp_path, &img_bytes).map_err(|e| format!("保存临时图片失败: {e}"))?;

    Ok(json!({
        "temp_path": temp_path.to_string_lossy().to_string(),
    }))
}
