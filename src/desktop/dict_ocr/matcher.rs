use super::{CharSegment, CharTemplate, DictMatch, SearchResult};

/// 快速比较两个二进制位图在给定位置的匹配度（位运算优化版）
#[inline]
fn compare_bitmaps(
    seg_data: &[u8],
    seg_w: u32,
    seg_h: u32,
    seg_dx: u32,
    seg_dy: u32,
    tmpl_data: &[u8],
    tmpl_w: u32,
    tmpl_h: u32,
) -> f32 {
    let mut match_count = 0u32;
    let mut total_count = 0u32;

    let tcols = tmpl_w.div_ceil(8);
    for ty in 0..tmpl_h {
        for tx in 0..tmpl_w {
            let sx = seg_dx + tx;
            let sy = seg_dy + ty;
            if sx >= seg_w || sy >= seg_h {
                continue;
            }
            let si = (sy * seg_w + sx) as usize;
            let sv = if si < seg_data.len() { seg_data[si] } else { 0 };

            // 位打包读取：每字节存 8 个像素，高位在前，行主序
            let bi = (ty * tcols + tx / 8) as usize;
            let bit = 7 - (tx % 8) as u8;
            let ev = if bi < tmpl_data.len() {
                (tmpl_data[bi] >> bit) & 1
            } else {
                0
            };

            if sv != 0 || ev != 0 {
                total_count += 1;
                if sv != 0 && ev != 0 {
                    match_count += 1;
                }
            }
        }
    }

    if total_count > 0 {
        match_count as f32 / total_count as f32
    } else {
        0.0
    }
}

/// 对单个 segment 做模板匹配（优化版）
/// 策略：尺寸预筛 → 中心对齐比较为主 → 局部微调 → 提前退出
pub fn match_template(seg: &CharSegment, dict: &[CharTemplate]) -> Vec<DictMatch> {
    let mut results = Vec::new();
    let mut best_overall = 0.0f32;

    for entry in dict {
        let ew = entry.width as u32;
        let eh = entry.height as u32;

        // 尺寸预筛：宽或高差异超过 2x 则跳过
        let w_ratio = seg.width.min(ew) as f32 / seg.width.max(ew) as f32;
        let h_ratio = seg.height.min(eh) as f32 / seg.height.max(eh) as f32;
        if w_ratio < 0.45 || h_ratio < 0.45 {
            continue;
        }

        let mut best = 0.0f32;

        if seg.width >= ew && seg.height >= eh {
            // 模板不超出 segment：中心对齐 + 局部 ±1px 微调
            let cx = (seg.width - ew) / 2;
            let cy = (seg.height - eh) / 2;
            let x_start = if cx > 1 { cx - 1 } else { cx };
            let y_start = if cy > 1 { cy - 1 } else { cy };
            let x_end = (seg.width - ew).min(cx + 1);
            let y_end = (seg.height - eh).min(cy + 1);

            for dy in y_start..=y_end {
                for dx in x_start..=x_end {
                    let score = compare_bitmaps(
                        &seg.data,
                        seg.width,
                        seg.height,
                        dx,
                        dy,
                        &entry.data,
                        ew,
                        eh,
                    );
                    if score > best {
                        best = score;
                    }
                }
            }
        } else {
            // 模板比 segment 大：segment 居中放在模板上比较
            let cx = if ew > seg.width {
                (ew - seg.width) / 2
            } else {
                0
            };
            let cy = if eh > seg.height {
                (eh - seg.height) / 2
            } else {
                0
            };
            if cx == 0 && cy == 0 {
                continue;
            }
            // 反转方向：把 segment 当"窗口"在模板上滑
            let x_start = if cx > 1 { cx - 1 } else { cx };
            let y_start = if cy > 1 { cy - 1 } else { cy };
            let x_end = (ew - seg.width).min(cx + 1);
            let y_end = (eh - seg.height).min(cy + 1);

            // 用模板为主，segment 为窗口，交换角色逐像素比较
            for dy in y_start..=y_end {
                for dx in x_start..=x_end {
                    let score = compare_bitmaps_small_window(
                        &seg.data,
                        seg.width,
                        seg.height,
                        &entry.data,
                        ew,
                        eh,
                        dx,
                        dy,
                    );
                    if score > best {
                        best = score;
                    }
                }
            }
        }

        results.push(DictMatch {
            char: entry.char.clone(),
            confidence: best,
            x: seg.x as i32,
            y: seg.y as i32,
            width: seg.width,
            height: seg.height,
        });

        // 提前退出：已找到高置信度匹配
        if best > best_overall {
            best_overall = best;
        }
        if best_overall > 0.78 {
            break;
        }
    }

    results.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// 当模板 > segment 时：segment 作为滑动窗口在模板上扫描
#[inline]
fn compare_bitmaps_small_window(
    seg_data: &[u8],
    seg_w: u32,
    seg_h: u32,
    tmpl_data: &[u8],
    tmpl_w: u32,
    _tmpl_h: u32,
    tmpl_dx: u32,
    tmpl_dy: u32,
) -> f32 {
    let mut match_count = 0u32;
    let mut total_count = 0u32;

    let tcols = tmpl_w.div_ceil(8);
    for sy in 0..seg_h {
        for sx in 0..seg_w {
            let bi = ((tmpl_dy + sy) * tcols + (tmpl_dx + sx) / 8) as usize;
            let bit = 7 - ((tmpl_dx + sx) % 8) as u8;
            let tv = if bi < tmpl_data.len() {
                (tmpl_data[bi] >> bit) & 1
            } else {
                0
            };

            let si = (sy * seg_w + sx) as usize;
            let sv = if si < seg_data.len() { seg_data[si] } else { 0 };

            if sv != 0 || tv != 0 {
                total_count += 1;
                if sv != 0 && tv != 0 {
                    match_count += 1;
                }
            }
        }
    }

    if total_count > 0 {
        match_count as f32 / total_count as f32
    } else {
        0.0
    }
}

/// 遍历 segments 返回每个 segment 的最佳匹配结果
pub fn match_segments(segs: &[CharSegment], dict: &[CharTemplate]) -> Vec<DictMatch> {
    segs.iter()
        .filter_map(|seg| {
            let results = match_template(seg, dict);
            results.into_iter().next()
        })
        .collect()
}

pub fn search_on_screen(
    screen_pixels: &[u8],
    screen_w: u32,
    screen_h: u32,
    template: &CharTemplate,
    fg_color: &super::ColorSpec,
    min_confidence: f32,
) -> Vec<SearchResult> {
    let tw = template.width as u32;
    let th = template.height as u32;
    if tw < 2 || th < 2 || screen_w < tw || screen_h < th {
        return vec![];
    }

    let mut results = Vec::new();

    let max_x = screen_w - tw;
    let max_y = screen_h - th;

    let tcols = tw.div_ceil(8);
    for y in 0..=max_y {
        for x in 0..=max_x {
            let mut match_count = 0u32;
            let mut total_count = 0u32;

            for ty in 0..th {
                for tx in 0..tw {
                    let si = ((y + ty) * screen_w + (x + tx)) as usize;
                    let sr = screen_pixels[si * 3];
                    let sg = screen_pixels[si * 3 + 1];
                    let sb = screen_pixels[si * 3 + 2];
                    let is_fg = fg_color.matches(sr, sg, sb);

                    let tbyte = (ty * tcols + tx / 8) as usize;
                    let tbit = 7 - (tx % 8) as usize;
                    let tval = if tbyte < template.data.len() {
                        (template.data[tbyte] >> tbit) & 1
                    } else {
                        0
                    };

                    if is_fg || tval != 0 {
                        total_count += 1;
                        if is_fg && tval != 0 {
                            match_count += 1;
                        }
                    }
                }
            }

            if total_count > 0 {
                let conf = match_count as f32 / total_count as f32;
                if conf >= min_confidence {
                    results.push(SearchResult {
                        x: x as i32,
                        y: y as i32,
                        width: tw,
                        height: th,
                        confidence: conf,
                    });
                }
            }
        }
    }

    deduplicate_overlapping(&results)
}

fn deduplicate_overlapping(results: &[SearchResult]) -> Vec<SearchResult> {
    let mut sorted = results.to_vec();
    sorted.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = Vec::new();
    for r in &sorted {
        let overlap = out.iter().any(|o: &SearchResult| {
            let dx = (r.x - o.x).unsigned_abs();
            let dy = (r.y - o.y).unsigned_abs();
            dx < r.width / 2 && dy < r.height / 2
        });
        if !overlap {
            out.push(r.clone());
        }
    }
    out
}
