pub mod analyzer;
pub mod binarize;
pub mod matcher;
pub mod segment;
pub mod store;

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSpec {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub dr: u8,
    pub dg: u8,
    pub db: u8,
}

impl ColorSpec {
    pub fn new(r: u8, g: u8, b: u8, dr: u8, dg: u8, db: u8) -> Self {
        Self {
            r,
            g,
            b,
            dr,
            dg,
            db,
        }
    }

    pub fn to_hex(&self) -> String {
        format!(
            "{:02X}{:02X}{:02X}-{:02X}{:02X}{:02X}",
            self.r, self.g, self.b, self.dr, self.dg, self.db
        )
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 2 {
            return None;
        }
        let fg = u32::from_str_radix(parts[0], 16).ok()?;
        let dl = u32::from_str_radix(parts[1], 16).ok()?;
        Some(Self {
            r: ((fg >> 16) & 0xFF) as u8,
            g: ((fg >> 8) & 0xFF) as u8,
            b: (fg & 0xFF) as u8,
            dr: ((dl >> 16) & 0xFF) as u8,
            dg: ((dl >> 8) & 0xFF) as u8,
            db: (dl & 0xFF) as u8,
        })
    }

    pub fn matches(&self, r: u8, g: u8, b: u8) -> bool {
        let rd = r.abs_diff(self.r);
        let gd = g.abs_diff(self.g);
        let bd = b.abs_diff(self.b);
        rd <= self.dr && gd <= self.dg && bd <= self.db
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharTemplate {
    pub char: String,
    pub width: u8,
    pub height: u8,
    pub data: Vec<u8>,
    #[serde(default)]
    pub grayscale: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharSegment {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictMatch {
    pub char: String,
    pub confidence: f32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegParams {
    pub min_col_gap: u32,
    pub min_row_gap: u32,
    pub word_gap: u32,
    pub line_height: u32,
}

impl Default for SegParams {
    fn default() -> Self {
        Self {
            min_col_gap: 4,
            min_row_gap: 4,
            word_gap: 6,
            line_height: 0,
        }
    }
}

pub fn analyze_region(pixels: &[u8], width: u32, height: u32) -> analyzer::ColorAnalysis {
    analyzer::analyze_region(pixels, width, height)
}

pub fn binarize(pixels: &[u8], width: u32, height: u32, fg: &ColorSpec) -> Vec<u8> {
    binarize::binarize(pixels, width, height, fg)
}

pub fn binarize_preview(pixels: &[u8], width: u32, height: u32, fg: &ColorSpec) -> Vec<u8> {
    binarize::binarize_to_rgba(pixels, width, height, fg)
}

pub fn segment(buffer: &[u8], width: u32, height: u32, params: &SegParams) -> Vec<CharSegment> {
    segment::segment(buffer, width, height, params)
}

pub fn auto_detect_gaps(buffer: &[u8], width: u32, height: u32) -> (u32, u32, u32) {
    segment::auto_detect_gaps(buffer, width, height)
}

pub fn match_char(seg: &CharSegment, dict: &[CharTemplate]) -> Vec<DictMatch> {
    matcher::match_template(seg, dict)
}

pub fn search_screen(
    screen_pixels: &[u8],
    screen_w: u32,
    screen_h: u32,
    template: &CharTemplate,
    fg: &ColorSpec,
    min_confidence: f32,
) -> Vec<SearchResult> {
    matcher::search_on_screen(
        screen_pixels,
        screen_w,
        screen_h,
        template,
        fg,
        min_confidence,
    )
}

pub fn recognize_region(
    pixels: &[u8],
    width: u32,
    height: u32,
    fg: &ColorSpec,
    dict: &[CharTemplate],
    params: &SegParams,
) -> Vec<DictMatch> {
    let binary = binarize::binarize(pixels, width, height, fg);
    let segs = segment::segment(&binary, width, height, params);
    let mut results = Vec::new();
    for s in &segs {
        let mut matches = matcher::match_template(s, dict);
        if let Some(best) = matches.first_mut() {
            best.x = s.x as i32;
            best.y = s.y as i32;
            results.push(best.clone());
        }
    }
    results
}

pub fn load_dict(path: &Path) -> std::io::Result<store::DictStore> {
    store::DictStore::load(path)
}

pub fn list_dicts(dir: &Path) -> std::io::Result<Vec<String>> {
    store::list_dicts(dir)
}

/// 遍历所有 .dict 文件，返回 (dict_name, path) 列表
pub fn list_all_dicts(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut result = Vec::new();
    if !dir.exists() {
        return result;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("dict") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                result.push((name.to_string(), path));
            }
        }
    }
    result
}

/// 用现有 .dict 字库匹配 segments
pub fn match_segments_against_dict(
    segs: &[CharSegment],
    dict_path: &Path,
    min_confidence: f32,
) -> Option<(f32, Vec<DictMatch>)> {
    let store = match store::DictStore::load(dict_path) {
        Ok(s) => s,
        Err(_) => return None,
    };
    let all_templates: Vec<CharTemplate> = store.all().values().flat_map(|v| v.clone()).collect();
    if all_templates.is_empty() {
        return None;
    }

    let matches = matcher::match_segments(segs, &all_templates);
    if matches.is_empty() {
        return None;
    }

    // 只保留置信度达标的匹配
    let good_matches: Vec<DictMatch> = matches
        .into_iter()
        .filter(|m| m.confidence >= min_confidence)
        .collect();

    if good_matches.is_empty() {
        return None;
    }

    let avg_conf =
        good_matches.iter().map(|m| m.confidence).sum::<f32>() / good_matches.len() as f32;
    Some((avg_conf, good_matches))
}
