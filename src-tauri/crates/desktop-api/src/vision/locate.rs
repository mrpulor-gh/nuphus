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

    /// 找图 - 像素级滑动窗口匹配（降采样搜索 + 提前终止）
    async fn find_image(&self, frame: &Frame, template: &[u8]) -> Result<FindResult> {
        const THRESHOLD: f32 = 30.0;
        const COARSE_STEP: u32 = 4;

        // 1. 解码模板 PNG
        let template_img = match image::load_from_memory(template) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                tracing::warn!("[find_image] 模板解码失败: {}", e);
                return Ok(FindResult {
                    found: false,
                    rect: None,
                    confidence: 0.0,
                    method: FindMethod::ImageMatch,
                });
            }
        };

        let (tw, th) = (template_img.width(), template_img.height());
        let fw = frame.width;
        let fh = frame.height;

        if tw > fw || th > fh {
            return Ok(FindResult {
                found: false,
                rect: None,
                confidence: 0.0,
                method: FindMethod::ImageMatch,
            });
        }

        let tpl_pixels = template_img.as_raw();
        let frame_pixels = &frame.pixels;

        // 2. 粗扫（步长=4）
        let mut best_x = 0u32;
        let mut best_y = 0u32;
        let mut best_diff = f32::MAX;

        let coarse_y_max = fh - th + 1;
        let coarse_x_max = fw - tw + 1;

        let start = std::time::Instant::now();

        for cy in (0..coarse_y_max).step_by(COARSE_STEP as usize) {
            for cx in (0..coarse_x_max).step_by(COARSE_STEP as usize) {
                let diff = Self::window_diff(frame_pixels, fw, tpl_pixels, tw, th, cx, cy);
                if diff < best_diff {
                    best_diff = diff;
                    best_x = cx;
                    best_y = cy;
                }
            }
        }

        // 3. 精扫（在粗扫最佳位置的邻域内逐像素搜索）
        let fine_start_x = best_x.saturating_sub(COARSE_STEP);
        let fine_start_y = best_y.saturating_sub(COARSE_STEP);
        let fine_end_x = (best_x + COARSE_STEP).min(coarse_x_max.saturating_sub(1));
        let fine_end_y = (best_y + COARSE_STEP).min(coarse_y_max.saturating_sub(1));

        let mut fine_best_x = best_x;
        let mut fine_best_y = best_y;
        let mut fine_best_diff = best_diff;

        for fy in fine_start_y..=fine_end_y {
            for fx in fine_start_x..=fine_end_x {
                let diff = Self::window_diff(frame_pixels, fw, tpl_pixels, tw, th, fx, fy);
                if diff < fine_best_diff {
                    fine_best_diff = diff;
                    fine_best_x = fx;
                    fine_best_y = fy;
                }
            }
        }

        let elapsed = start.elapsed().as_millis();
        if elapsed > 500 {
            tracing::warn!(
                "[find_image] 搜索耗时 {}ms ({}x{} 帧, {}x{} 模板)",
                elapsed,
                fw,
                fh,
                tw,
                th
            );
        }

        // 4. 判断是否匹配
        if fine_best_diff < THRESHOLD {
            Ok(FindResult {
                found: true,
                rect: Some(Rect {
                    x: fine_best_x as i32,
                    y: fine_best_y as i32,
                    w: tw,
                    h: th,
                }),
                confidence: 1.0 - (fine_best_diff / (THRESHOLD * 2.0)),
                method: FindMethod::ImageMatch,
            })
        } else {
            Ok(FindResult {
                found: false,
                rect: None,
                confidence: 0.0,
                method: FindMethod::ImageMatch,
            })
        }
    }

    /// 滑动窗口差异计算
    #[allow(clippy::too_many_arguments)]
    fn window_diff(
        frame_pixels: &[u8],
        fw: u32,
        tpl_pixels: &[u8],
        tw: u32,
        th: u32,
        x: u32,
        y: u32,
    ) -> f32 {
        let mut total_diff: u64 = 0;
        let pixel_count = (tw * th) as u64;

        for ty in 0..th {
            for tx in 0..tw {
                let f_idx = (((y + ty) * fw + (x + tx)) * 4) as usize;
                let t_idx = ((ty * tw + tx) * 4) as usize;

                let dr = frame_pixels[f_idx].abs_diff(tpl_pixels[t_idx]) as u64;
                let dg = frame_pixels[f_idx + 1].abs_diff(tpl_pixels[t_idx + 1]) as u64;
                let db = frame_pixels[f_idx + 2].abs_diff(tpl_pixels[t_idx + 2]) as u64;

                total_diff += dr + dg + db;
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
