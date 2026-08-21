//! Vision tools — find color / multi-color / OCR (Tesseract)
//!
//! All functions operate directly on BMP pixel data, zero external dependencies.
//!
//! - Find image migrated to desktop-api Locator (coarse + fine MAD matching)
//! - Dictionary system migrated to dict_ocr module
//!
//! Color finding algorithm:
//! - Compare colors pixel by pixel within specified region
//! - Color format "RRGGBB-DRDGDB" with delta tolerance
//!
//! Multi-color find:
//! - Find anchor color first, then verify offset points
//! - "!" prefix means this point should not match (negative exclusion)

use serde_json::Value;

/// Color delta parameters (per-channel tolerance 0..255)
#[derive(Debug, Clone, Copy)]
pub struct DeltaColor {
    pub dr: u8,
    pub dg: u8,
    pub db: u8,
}

impl DeltaColor {
    pub fn from_hex(s: &str) -> Self {
        let s = s.trim();
        if s.len() < 6 {
            return Self {
                dr: 0,
                dg: 0,
                db: 0,
            };
        }
        Self {
            dr: u8::from_str_radix(&s[0..2], 16).unwrap_or(0),
            dg: u8::from_str_radix(&s[2..4], 16).unwrap_or(0),
            db: u8::from_str_radix(&s[4..6], 16).unwrap_or(0),
        }
    }
    pub fn zero() -> Self {
        Self {
            dr: 0,
            dg: 0,
            db: 0,
        }
    }
}

/// Search direction
#[derive(Debug, Clone, Copy)]
pub enum SearchDir {
    LeftTop,
    RightTop,
    LeftBottom,
    RightBottom,
}

impl SearchDir {
    pub fn from_i32(n: i32) -> Self {
        match n & 3 {
            1 => Self::RightTop,
            2 => Self::LeftBottom,
            3 => Self::RightBottom,
            _ => Self::LeftTop,
        }
    }
}

fn u32_from_le(b: &[u8]) -> u32 {
    b.iter()
        .enumerate()
        .take(4)
        .fold(0u32, |a, (i, &v)| a | (v as u32) << (i * 8))
}
fn u16_from_le(b: &[u8]) -> u16 {
    b[0] as u16 | (b[1] as u16) << 8
}

pub fn load_bmp(path: &str) -> crate::Result<(u32, u32, Vec<u8>)> {
    let data = std::fs::read(path)
        .map_err(|e| crate::NuphusError::Tool(format!("read bmp failed: {e}")))?;
    if data.len() < 54 {
        return Err(crate::NuphusError::Tool("bmp too small".into()));
    }
    if &data[0..2] != b"BM" {
        return Err(crate::NuphusError::Tool("not a BMP".into()));
    }
    let w = u32_from_le(&data[18..22]);
    let h = u32_from_le(&data[22..26]);
    let bpp = u16_from_le(&data[28..30]);
    let pad = ((4 - (w * (bpp as u32 / 8) % 4)) % 4) as usize;
    let off = u32_from_le(&data[10..14]) as usize;
    let row_size = (w as usize * (bpp as usize / 8)) + pad;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for row in (0..h as usize).rev() {
        let start = off + row * row_size;
        for col in 0..w as usize {
            let idx = start + col * (bpp as usize / 8);
            if bpp == 24 || bpp == 32 {
                rgb.push(data[idx + 2]);
                rgb.push(data[idx + 1]);
                rgb.push(data[idx]);
            }
        }
    }
    Ok((w, h, rgb))
}

fn parse_color_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ))
}

fn parse_color_delta(s: &str) -> Option<((u8, u8, u8), (u8, u8, u8))> {
    let s = s.trim();
    // 尝试逗号分隔 (R,G,B 或 R,G,B,Dr,Dg,Db)
    if s.contains(',') {
        let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
        if parts.len() >= 3 {
            let c = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            let d = if parts.len() >= 6 {
                (
                    parts[3].parse().ok()?,
                    parts[4].parse().ok()?,
                    parts[5].parse().ok()?,
                )
            } else {
                (0, 0, 0)
            };
            return Some((c, d));
        }
    }
    // 尝试 HEX 格式: "3B82F6" 或 "3B82F6-050505"
    let parts: Vec<&str> = s.split('-').collect();
    let c = parse_color_hex(parts[0])?;
    let d = if parts.len() > 1 {
        parse_color_hex(parts[1]).unwrap_or((0, 0, 0))
    } else {
        (0, 0, 0)
    };
    Some((c, d))
}

fn match_color(r: u8, g: u8, b: u8, tr: u8, tg: u8, tb: u8, dr: u8, dg: u8, db: u8) -> bool {
    (r as i16 - tr as i16).abs() <= dr as i16
        && (g as i16 - tg as i16).abs() <= dg as i16
        && (b as i16 - tb as i16).abs() <= db as i16
}

// ══════════════════════════════════════════════════════════════
//  find_image migrated to desktop-api Locator (coarse + fine MAD matching)
//  参见 DesktopClient::find_image() 和 desktop_api::vision::locate
// ══════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════
//  Color find (enhanced)
// ══════════════════════════════════════════════════════════════

/// Find color in screenshot, supports multi-color combinations and delta tolerance
pub fn find_color(
    screenshot_path: &str,
    color: &str,
    region_x: i32,
    region_y: i32,
    region_w: u32,
    region_h: u32,
    direction: &str,
) -> crate::Result<Value> {
    let (sw, sh, pixels) = load_bmp(screenshot_path)?;
    let specs: Vec<((u8, u8, u8), (u8, u8, u8))> =
        color.split('|').filter_map(parse_color_delta).collect();
    if specs.is_empty() {
        return Ok(serde_json::json!({"found":false,"x":0,"y":0}));
    }

    let sx = if region_w > 0 {
        region_x.max(0) as u32
    } else {
        0
    };
    let sy = if region_h > 0 {
        region_y.max(0) as u32
    } else {
        0
    };
    let sw_ = if region_w > 0 {
        (region_w as i32).min(sw as i32 - sx as i32).max(0) as u32
    } else {
        sw
    };
    let sh_ = if region_h > 0 {
        (region_h as i32).min(sh as i32 - sy as i32).max(0) as u32
    } else {
        sh
    };
    let ex = (sx + sw_).min(sw);
    let ey = (sy + sh_).min(sh);

    enum D {
        LT,
        RT,
        LB,
        RB,
    }
    let d = match direction {
        "right_top" => D::RT,
        "left_bottom" => D::LB,
        "right_bottom" => D::RB,
        _ => D::LT,
    };

    let mut all: Vec<Value> = Vec::new();
    let coords: Vec<(u32, u32)> = match d {
        D::LT => {
            let mut v = Vec::new();
            for y in sy..ey {
                for x in sx..ex {
                    v.push((x, y));
                }
            }
            v
        }
        D::RT => {
            let mut v = Vec::new();
            for y in sy..ey {
                for x in (sx..ex).rev() {
                    v.push((x, y));
                }
            }
            v
        }
        D::LB => {
            let mut v = Vec::new();
            for y in (sy..ey).rev() {
                for x in sx..ex {
                    v.push((x, y));
                }
            }
            v
        }
        D::RB => {
            let mut v = Vec::new();
            for y in (sy..ey).rev() {
                for x in (sx..ex).rev() {
                    v.push((x, y));
                }
            }
            v
        }
    };

    for &(x, y) in &coords {
        let idx = ((y * sw + x) * 3) as usize;
        let (pr, pg, pb) = (pixels[idx], pixels[idx + 1], pixels[idx + 2]);
        for &((cr, cg, cb), (dr, dg, db)) in &specs {
            if match_color(pr, pg, pb, cr, cg, cb, dr, dg, db) {
                all.push(serde_json::json!({
                    "x": x, "y": y, "r": pr, "g": pg, "b": pb,
                    "target": format!("{:02X}{:02X}{:02X}", cr, cg, cb),
                }));
                break;
            }
        }
    }

    if all.is_empty() {
        Ok(serde_json::json!({"found":false,"x":0,"y":0,"matches":[]}))
    } else {
        Ok(serde_json::json!({"found":true,"x":all[0]["x"],"y":all[0]["y"],"matches":all}))
    }
}

// ══════════════════════════════════════════════════════════════
//  Multi-color find (enhanced)
// ══════════════════════════════════════════════════════════════

/// Multi-color find — find anchor color first, then verify offset point colors
pub fn find_multi_color(
    screenshot_path: &str,
    anchor_color: &str,
    offset_specs: &str,
    min_match_ratio: f64,
    region_x: i32,
    region_y: i32,
    region_w: u32,
    region_h: u32,
    _direction: &str,
) -> crate::Result<Value> {
    let (sw, sh, pixels) = load_bmp(screenshot_path)?;
    let (ac, ad) = parse_color_delta(anchor_color)
        .ok_or_else(|| crate::NuphusError::Tool("invalid anchor color".into()))?;

    struct Pt {
        dx: i32,
        dy: i32,
        expect: bool,
        c: (u8, u8, u8),
        d: (u8, u8, u8),
    }
    let offsets: Vec<Pt> = offset_specs
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|seg| {
            let parts: Vec<&str> = seg.trim().splitn(3, '|').collect();
            if parts.len() < 3 {
                return None;
            }
            let dx: i32 = parts[0].trim().parse().ok()?;
            let dy: i32 = parts[1].trim().parse().ok()?;
            let cs = parts[2].trim();
            let expect = !cs.starts_with('!');
            let clean = if expect { cs } else { &cs[1..] };
            let (c, d) = parse_color_delta(clean)?;
            Some(Pt {
                dx,
                dy,
                expect,
                c,
                d,
            })
        })
        .collect();

    if offsets.is_empty() {
        return Ok(serde_json::json!({"found":false,"x":0,"y":0,"match_ratio":0.0}));
    }

    let sx = if region_w > 0 {
        region_x.max(0) as u32
    } else {
        0
    };
    let sy = if region_h > 0 {
        region_y.max(0) as u32
    } else {
        0
    };
    let sw_ = if region_w > 0 {
        (region_w as i32).min(sw as i32 - sx as i32).max(0) as u32
    } else {
        sw
    };
    let sh_ = if region_h > 0 {
        (region_h as i32).min(sh as i32 - sy as i32).max(0) as u32
    } else {
        sh
    };
    let ex = (sx + sw_).min(sw);
    let ey = (sy + sh_).min(sh);
    let olen = offsets.len() as f64;

    for y in sy..ey {
        for x in sx..ex {
            let idx = ((y * sw + x) * 3) as usize;
            let (pr, pg, pb) = (pixels[idx], pixels[idx + 1], pixels[idx + 2]);
            if !match_color(pr, pg, pb, ac.0, ac.1, ac.2, ad.0, ad.1, ad.2) {
                continue;
            }

            let mut matched = 0u32;
            for off in &offsets {
                let ox = x as i32 + off.dx;
                let oy = y as i32 + off.dy;
                if ox < 0 || ox >= sw as i32 || oy < 0 || oy >= sh as i32 {
                    if off.expect {
                        matched = 0;
                        break;
                    } else {
                        matched += 1;
                        continue;
                    }
                }
                let oi = ((oy as u32 * sw + ox as u32) * 3) as usize;
                let ok = match_color(
                    pixels[oi],
                    pixels[oi + 1],
                    pixels[oi + 2],
                    off.c.0,
                    off.c.1,
                    off.c.2,
                    off.d.0,
                    off.d.1,
                    off.d.2,
                );
                if off.expect == ok {
                    matched += 1;
                }
            }
            let ratio = matched as f64 / olen;
            if ratio >= min_match_ratio {
                return Ok(
                    serde_json::json!({"found":true,"x":x,"y":y,"match_ratio":(ratio*10000.0).round()/10000.0}),
                );
            }
        }
    }
    Ok(serde_json::json!({"found":false,"x":0,"y":0,"match_ratio":0.0}))
}
