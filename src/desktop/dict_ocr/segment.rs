use super::{CharSegment, SegParams};

pub fn segment(buffer: &[u8], width: u32, height: u32, params: &SegParams) -> Vec<CharSegment> {
    if buffer.len() < (width * height) as usize {
        return vec![];
    }

    let col_proj = project_columns(buffer, width, height);
    let row_proj = project_rows(buffer, width, height);

    let col_segments = find_gap_ranges(&col_proj, params.min_col_gap, params.word_gap);
    let row_segments = find_gap_ranges(&row_proj, params.min_row_gap, params.line_height.max(1));

    let mut chars = Vec::new();

    for &(cy, ch) in &row_segments {
        for &(cx, cw) in &col_segments {
            let seg_data = extract_block(buffer, width, cx, cy, cw, ch);

            let inner = trim_border(&seg_data, cw, ch);
            if inner.width < 2 || inner.height < 2 {
                continue;
            }

            chars.push(CharSegment {
                x: cx + inner.x,
                y: cy + inner.y,
                width: inner.width,
                height: inner.height,
                data: inner.data,
            });
        }
    }

    chars
}

fn project_columns(buffer: &[u8], w: u32, h: u32) -> Vec<u32> {
    let mut proj = vec![0u32; w as usize];
    for y in 0..h {
        for x in 0..w {
            if buffer[(y * w + x) as usize] != 0 {
                proj[x as usize] += 1;
            }
        }
    }
    proj
}

fn project_rows(buffer: &[u8], w: u32, h: u32) -> Vec<u32> {
    let mut proj = vec![0u32; h as usize];
    for y in 0..h {
        for x in 0..w {
            if buffer[(y * w + x) as usize] != 0 {
                proj[y as usize] += 1;
            }
        }
    }
    proj
}

fn find_gap_ranges(proj: &[u32], min_gap: u32, min_size: u32) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut start: Option<u32> = None;
    let mut zero_run = 0u32;

    for (i, &val) in proj.iter().enumerate() {
        let i = i as u32;
        if val > 0 {
            if start.is_none() {
                start = Some(i);
            }
            zero_run = 0;
        } else {
            zero_run += 1;
            if start.is_some() && zero_run >= min_gap {
                let seg_end = i - zero_run + 1;
                let s = start.expect("guard already checked is_some");
                let seg_w = seg_end - s;
                if seg_w >= min_size {
                    ranges.push((s, seg_w));
                }
                start = None;
                zero_run = 0;
            }
        }
    }
    let len = proj.len() as u32;
    if let Some(s) = start {
        if len - s >= min_size {
            ranges.push((s, len - s));
        }
    }

    if ranges.is_empty() {
        ranges.push((0, len));
    }

    ranges
}

fn extract_block(buf: &[u8], full_w: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut block = vec![0u8; (w * h) as usize];
    for row in 0..h {
        let src_off = ((y + row) * full_w + x) as usize;
        let dst_off = (row * w) as usize;
        block[dst_off..dst_off + w as usize].copy_from_slice(&buf[src_off..src_off + w as usize]);
    }
    block
}

fn trim_border(buf: &[u8], w: u32, h: u32) -> CharSegment {
    let mut x1 = w;
    let mut y1 = h;
    let mut x2 = 0u32;
    let mut y2 = 0u32;

    for y in 0..h {
        for x in 0..w {
            if buf[(y * w + x) as usize] != 0 {
                if x < x1 {
                    x1 = x;
                }
                if x > x2 {
                    x2 = x;
                }
                if y < y1 {
                    y1 = y;
                }
                if y > y2 {
                    y2 = y;
                }
            }
        }
    }

    if x1 > x2 || y1 > y2 {
        return CharSegment {
            x: 0,
            y: 0,
            width: w,
            height: h,
            data: buf.to_vec(),
        };
    }

    let nw = x2 - x1 + 1;
    let nh = y2 - y1 + 1;
    let ndata = extract_block(buf, w, x1, y1, nw, nh);

    CharSegment {
        x: x1,
        y: y1,
        width: nw,
        height: nh,
        data: ndata,
    }
}

pub fn column_gap_search(proj: &[u32], threshold: u32) -> Vec<(u32, u32)> {
    let mut segs = Vec::new();
    let mut start = None;
    for (i, &v) in proj.iter().enumerate() {
        if v > threshold {
            if start.is_none() {
                start = Some(i as u32);
            }
        } else {
            if let Some(s) = start.take() {
                segs.push((s, i as u32 - s));
            }
        }
    }
    if let Some(s) = start {
        segs.push((s, proj.len() as u32 - s));
    }
    segs
}

/// 自动检测字符间距参数（根据投影分析）
pub fn auto_detect_gaps(buffer: &[u8], width: u32, height: u32) -> (u32, u32, u32) {
    let col_proj = project_columns(buffer, width, height);
    let row_proj = project_rows(buffer, width, height);

    let col_gap = measure_typical_gap(&col_proj);
    let row_gap = measure_typical_gap(&row_proj);

    // word_gap ≈ col_gap * 3~4，至少比 col_gap 大 2
    let word_gap = (col_gap * 3).max(col_gap + 2).min(20);

    // CJK 字符最小间隙通常 ≥4px，低于此值的多是偏旁内部空隙
    (col_gap.max(4), row_gap.max(4), word_gap.max(6))
}

fn measure_typical_gap(proj: &[u32]) -> u32 {
    let mut gaps = Vec::new();
    let mut zero_run = 0u32;
    for &v in proj.iter() {
        if v == 0 {
            zero_run += 1;
        } else if zero_run > 0 {
            gaps.push(zero_run);
            zero_run = 0;
        }
    }
    // 忽略首尾空白（边距）
    if gaps.len() > 1 {
        gaps.remove(0);
    }
    if gaps.len() > 1 {
        gaps.pop();
    }

    // 跳过 1px 间隙（通常是笔画内部空隙），只保留 ≥2px 的
    gaps.retain(|&g| g > 1);
    if gaps.is_empty() {
        return 3;
    }

    // 取中位数
    gaps.sort_unstable();
    let mid = gaps.len() / 2;
    let v = if gaps.len() % 2 == 0 {
        (gaps[mid - 1] + gaps[mid]) / 2
    } else {
        gaps[mid]
    };
    v.clamp(2, 8)
}
