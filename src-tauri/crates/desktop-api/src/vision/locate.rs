//! 查找实现 - 找图/找字/找色 + 降级策略

use crate::core::*;
use crate::vision::{DeltaColor, FindMethod, FindResult, Query, ScanDirection};

pub struct Locator;

impl Default for Locator {
    fn default() -> Self {
        Self
    }
}

impl Locator {
    pub fn new() -> Self {
        Self
    }

    /// 查找主入口 - 带降级策略
    pub async fn find_with_fallback(&self, frame: &Frame, query: &Query) -> Result<FindResult> {
        match query {
            Query::Image(img) => self.find_image(frame, img).await,
            Query::Text(_text) => {
                // OCR 引擎暂不可用，等待 PaddleOCR 集成
                Ok(FindResult {
                    found: false,
                    rect: None,
                    confidence: 0.0,
                    method: FindMethod::TextOcr,
                })
            }
            Query::Color { target, tolerance } => self.find_color(frame, *target, *tolerance),
            Query::TextThenColor {
                text: _,
                color,
                tolerance,
            } => {
                // 降级: 直接找色
                self.find_color(frame, *color, *tolerance)
            }
            Query::ImageThenText { image, text: _ } => {
                // 策略1: 先找图
                if let Ok(r) = self.find_image(frame, image).await {
                    if r.found {
                        return Ok(r);
                    }
                }
                Ok(FindResult {
                    found: false,
                    rect: None,
                    confidence: 0.0,
                    method: FindMethod::Fallback,
                })
            }
        }
    }

    /// 找图 - 静态图片模板匹配（按模板原始尺寸，不缩放）
    ///
    /// 策略：金字塔降采样粗扫（多级）→ Top-N 候选 → 原图精扫 → 差分提前终止。
    /// - 多格式解码：image crate 支持 PNG / JPEG / BMP / GIF 等
    /// - 尺寸随原图：模板多大就在帧里找多大，不做任何缩放匹配
    /// - 失败诊断：未命中时仍返回最接近候选的位置与置信度（供上层返回给 Agent）
    async fn find_image(&self, frame: &Frame, template: &[u8]) -> Result<FindResult> {
        const THRESHOLD: f32 = 30.0;
        const TOP_N: usize = 6;

        // 1. 解码模板（多格式）
        let template_img = match image::load_from_memory(template) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                tracing::warn!("[find_image] 模板解码失败: {}", e);
                return Err(
                    anyhow::anyhow!("模板图片解码失败: {} (支持 PNG/JPG/BMP/GIF)", e).into(),
                );
            }
        };

        let (tw, th) = (template_img.width(), template_img.height());
        let (fw, fh) = (frame.width, frame.height);

        if tw == 0 || th == 0 || tw > fw || th > fh {
            return Ok(FindResult {
                found: false,
                rect: None,
                confidence: 0.0,
                method: FindMethod::ImageMatch,
            });
        }

        let tpl_pixels = template_img.as_raw();

        // 2. 金字塔层级自适应：模板越大，粗扫降采样倍数越高
        //    （粗扫只做候选定位，最终仍回到原图原尺寸精扫）
        let max_side = tw.max(th);
        let scale: u32 = if max_side >= 64 {
            4
        } else if max_side >= 24 {
            2
        } else {
            1
        };

        // 3. 构建降采样帧与降采样模板（2x2 平均，与帧同比例）
        let (coarse_frame, cw, ch) = match scale {
            4 => {
                let (h1, w1, hh1) = Self::downsample_avg(&frame.pixels, fw, fh);
                Self::downsample_avg(&h1, w1, hh1)
            }
            2 => Self::downsample_avg(&frame.pixels, fw, fh),
            _ => (frame.pixels.clone(), fw, fh),
        };
        let (coarse_tpl, ctw, cth) = match scale {
            4 => {
                let (h1, w1, hh1) = Self::downsample_avg(tpl_pixels, tw, th);
                Self::downsample_avg(&h1, w1, hh1)
            }
            2 => Self::downsample_avg(tpl_pixels, tw, th),
            _ => (tpl_pixels.to_vec(), tw, th),
        };
        // 降采样后模板过小（<4px）则退回原图扫描
        let (coarse_frame, cw, ch, coarse_tpl, ctw, cth) = if ctw < 4 || cth < 4 {
            (frame.pixels.clone(), fw, fh, tpl_pixels.to_vec(), tw, th)
        } else {
            (coarse_frame, cw, ch, coarse_tpl, ctw, cth)
        };

        // 4. 粗扫（降采样图，步长=2）→ Top-N 候选
        let cy_max = ch.saturating_sub(cth) + 1;
        let cx_max = cw.saturating_sub(ctw) + 1;
        let mut candidates: Vec<(u32, u32, f32)> = Vec::with_capacity(TOP_N + 4);
        let mut worst_in_cands = f32::MAX;

        for cy in (0..cy_max).step_by(2) {
            for cx in (0..cx_max).step_by(2) {
                let diff = Self::window_diff(
                    &coarse_frame,
                    cw,
                    &coarse_tpl,
                    ctw,
                    cth,
                    cx,
                    cy,
                    worst_in_cands,
                );
                if candidates.len() < TOP_N {
                    candidates.push((cx, cy, diff));
                    if diff < worst_in_cands {
                        worst_in_cands = diff;
                    }
                    continue;
                }
                if diff < worst_in_cands {
                    // 替换当前最差候选
                    let mut wi = 0usize;
                    for (i, c) in candidates.iter().enumerate() {
                        if c.2 > candidates[wi].2 {
                            wi = i;
                        }
                    }
                    candidates[wi] = (cx, cy, diff);
                    worst_in_cands = candidates.iter().map(|c| c.2).fold(f32::MIN, f32::max);
                }
            }
        }
        candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        // 5. 原图精扫：候选映射回原图坐标，邻域 ±(2*scale+2) 逐像素
        let fine_radius = 2 * scale + 2;
        let mut best_diff = f32::MAX;
        let mut best_x = 0u32;
        let mut best_y = 0u32;

        let start = std::time::Instant::now();
        for &(ccx, ccy, _) in &candidates {
            let ox = ccx * scale;
            let oy = ccy * scale;
            let sx = ox.saturating_sub(fine_radius);
            let sy = oy.saturating_sub(fine_radius);
            let ex = (ox + fine_radius).min(fw.saturating_sub(tw));
            let ey = (oy + fine_radius).min(fh.saturating_sub(th));
            for y in sy..=ey {
                for x in sx..=ex {
                    let diff =
                        Self::window_diff(&frame.pixels, fw, tpl_pixels, tw, th, x, y, best_diff);
                    if diff < best_diff {
                        best_diff = diff;
                        best_x = x;
                        best_y = y;
                    }
                }
            }
        }

        let elapsed = start.elapsed().as_millis();
        if elapsed > 500 {
            tracing::warn!(
                "[find_image] 搜索耗时 {}ms ({}x{} 帧, {}x{} 模板, scale={})",
                elapsed,
                fw,
                fh,
                tw,
                th,
                scale
            );
        }

        // 6. 判定 + 失败诊断（未命中也返回最接近候选）
        let confidence = Self::confidence_from_diff(best_diff, THRESHOLD);
        if best_diff < THRESHOLD {
            Ok(FindResult {
                found: true,
                rect: Some(Rect {
                    x: best_x as i32,
                    y: best_y as i32,
                    w: tw,
                    h: th,
                }),
                confidence,
                method: FindMethod::ImageMatch,
            })
        } else {
            Ok(FindResult {
                found: false,
                rect: Some(Rect {
                    x: best_x as i32,
                    y: best_y as i32,
                    w: tw,
                    h: th,
                }),
                confidence,
                method: FindMethod::ImageMatch,
            })
        }
    }

    /// diff → confidence（0..1），diff 越小置信度越高
    fn confidence_from_diff(diff: f32, threshold: f32) -> f32 {
        (1.0 - diff / (threshold * 2.0)).clamp(0.0, 1.0)
    }

    /// 2x2 平均降采样（RGBA），返回 (像素, 宽, 高)
    fn downsample_avg(pixels: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
        let nw = w / 2;
        let nh = h / 2;
        if nw == 0 || nh == 0 {
            return (pixels.to_vec(), w, h);
        }
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut sums = [0u64; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let idx = (((y * 2 + dy) * w + (x * 2 + dx)) * 4) as usize;
                        for c in 0..4 {
                            sums[c] += pixels[idx + c] as u64;
                        }
                    }
                }
                let oidx = ((y * nw + x) * 4) as usize;
                for c in 0..4 {
                    out[oidx + c] = (sums[c] / 4) as u8;
                }
            }
        }
        (out, nw, nh)
    }

    /// 滑动窗口差异计算（平均绝对差，带提前终止）
    ///
    /// `early_stop_at` 为当前最优平均差；累计差一旦超过
    /// `early_stop_at * 像素数 * 3` 立即终止，跳过不可能胜出的窗口。
    #[allow(clippy::too_many_arguments)]
    fn window_diff(
        frame_pixels: &[u8],
        fw: u32,
        tpl_pixels: &[u8],
        tw: u32,
        th: u32,
        x: u32,
        y: u32,
        early_stop_at: f32,
    ) -> f32 {
        let mut total_diff: u64 = 0;
        let pixel_count = (tw * th) as u64;
        let stop_total = if early_stop_at < f32::MAX {
            (early_stop_at * pixel_count as f32 * 3.0) as u64
        } else {
            u64::MAX
        };

        'outer: for ty in 0..th {
            for tx in 0..tw {
                let f_idx = (((y + ty) * fw + (x + tx)) * 4) as usize;
                let t_idx = ((ty * tw + tx) * 4) as usize;

                let dr = frame_pixels[f_idx].abs_diff(tpl_pixels[t_idx]) as u64;
                let dg = frame_pixels[f_idx + 1].abs_diff(tpl_pixels[t_idx + 1]) as u64;
                let db = frame_pixels[f_idx + 2].abs_diff(tpl_pixels[t_idx + 2]) as u64;

                total_diff += dr + dg + db;
                if total_diff >= stop_total {
                    break 'outer;
                }
            }
        }

        (total_diff as f32) / (pixel_count as f32 * 3.0)
    }

    /// 偏色单点找色 - 按指定方向扫描，返回第一个匹配点
    ///
    /// 使用 DeltaColor 的 RGB 三通道独立容差判断，比欧几里得距离更精确。
    fn find_color_with_delta(
        &self,
        frame: &Frame,
        target: Color,
        delta: &DeltaColor,
        direction: ScanDirection,
    ) -> Result<FindResult> {
        let xs: Vec<u32> = match direction {
            ScanDirection::LeftTop | ScanDirection::LeftBottom => (0..frame.width).collect(),
            ScanDirection::RightTop | ScanDirection::RightBottom => {
                (0..frame.width).rev().collect()
            }
        };
        let ys: Vec<u32> = match direction {
            ScanDirection::LeftTop | ScanDirection::RightTop => (0..frame.height).collect(),
            ScanDirection::LeftBottom | ScanDirection::RightBottom => {
                (0..frame.height).rev().collect()
            }
        };

        for &y in &ys {
            for &x in &xs {
                if let Some(color) = frame.get_pixel(x, y) {
                    if delta.matches(&color, &target) {
                        return Ok(FindResult {
                            found: true,
                            rect: Some(Rect {
                                x: x as i32,
                                y: y as i32,
                                w: 1,
                                h: 1,
                            }),
                            confidence: 1.0,
                            method: FindMethod::ColorScan,
                        });
                    }
                }
            }
        }

        Ok(FindResult {
            found: false,
            rect: None,
            confidence: 0.0,
            method: FindMethod::ColorScan,
        })
    }

    /// 找色 - 兼容旧接口（使用单容差值）
    fn find_color(&self, frame: &Frame, target: Color, tolerance: u8) -> Result<FindResult> {
        let delta = DeltaColor::from_tolerance(tolerance);
        self.find_color_with_delta(frame, target, &delta, ScanDirection::default())
    }

    /// 提取主要颜色
    pub fn extract_colors(&self, _frame: &Frame) -> Result<Vec<crate::vision::ColorSpot>> {
        // TODO: K-Means 聚类
        Ok(vec![])
    }
}
