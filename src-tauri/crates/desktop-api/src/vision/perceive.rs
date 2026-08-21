//! UI 感知合并引擎 — OCR + YOLO IoU 去重（desktop-api 共享层）。
//!
//! 逻辑移植自主 crate `src/desktop/ui_perception.rs`：PaddleOCR 看到文字但看
//! 不到图标/空输入框，YOLO 看到 UI 元素但不知道上面写了什么。本模块将两者合并，
//! 输出统一的 UiElement 视图，并提供一次加载图片同时跑 OCR + YOLO 的入口。

use crate::vision::models::{resolve_models_dir, validate_ocr_models, yolo_model_available};
use crate::vision::paddle_ocr::{OcrBlock, PaddleOcr};
use crate::vision::yolo::YoloDetector;
use crate::vision::{Element, ElementKind};
use crate::{Frame, FrameSource, Rect, Scope};

// ─── 合并输出类型 ───────────────────────────────────────────────

/// 合并后的 UI 元素
#[derive(Debug, Clone)]
pub struct UiElement {
    pub id: u32,
    pub kind: ElementKind,
    pub text: Option<String>,
    pub rect: Rect,
    pub confidence: f32,
    pub source: ElementSource,
}

/// 元素来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementSource {
    /// 仅 OCR 检测到
    Ocr,
    /// 仅 YOLO 检测到
    Yolo,
    /// 两者检测到（IoU > 阈值）
    Both,
}

// ─── 合并参数 ───────────────────────────────────────────────────

/// 合并 IoU 阈值
const IOU_MERGE_THRESHOLD: f32 = 0.3;

// ─── 合并入口 ───────────────────────────────────────────────────

/// 合并 OCR 文字块和 YOLO 元素检测结果。
///
/// - 每个 OCR 块尝试匹配最佳 YOLO 框（IoU > 0.3）
///   - 匹配成功 → 合并为 Both，kind 启发式推断
///   - 匹配失败 → 保留为 Ocr
/// - 未匹配的 YOLO 框 → 保留为 Yolo（纯图标）
pub fn merge(ocr_blocks: &[OcrBlock], yolo_elements: &[Element]) -> Vec<UiElement> {
    let mut result: Vec<UiElement> = Vec::new();
    let mut yolo_used = vec![false; yolo_elements.len()];

    // ── 第一轮：OCR → 匹配 YOLO ──
    for block in ocr_blocks {
        let ocr_rect = Rect {
            x: block.x,
            y: block.y,
            w: block.w.max(0) as u32,
            h: block.h.max(0) as u32,
        };

        // 找最佳匹配的 YOLO 框
        let mut best_iou: f32 = 0.0;
        let mut best_idx: Option<usize> = None;

        for (j, yolo_elem) in yolo_elements.iter().enumerate() {
            if yolo_used[j] {
                continue;
            }
            let iou = rect_iou(&ocr_rect, &yolo_elem.rect);
            if iou > IOU_MERGE_THRESHOLD && iou > best_iou {
                best_iou = iou;
                best_idx = Some(j);
            }
        }

        if let Some(idx) = best_idx {
            // 匹配成功 → 合并
            yolo_used[idx] = true;
            let yolo_elem = &yolo_elements[idx];
            let combined_kind = infer_kind(&block.text, &yolo_elem.rect);

            result.push(UiElement {
                id: 0, // 最后统一编号
                kind: combined_kind,
                text: Some(block.text.clone()),
                rect: yolo_elem.rect, // YOLO 框更准
                confidence: (block_confidence(&block.text) + yolo_elem.confidence) / 2.0,
                source: ElementSource::Both,
            });
        } else {
            // 无匹配 → 独立 OCR 元素
            result.push(UiElement {
                id: 0,
                kind: ElementKind::Text,
                text: Some(block.text.clone()),
                rect: ocr_rect,
                confidence: block_confidence(&block.text),
                source: ElementSource::Ocr,
            });
        }
    }

    // ── 第二轮：未匹配的 YOLO → 独立图标 ──
    for (j, yolo_elem) in yolo_elements.iter().enumerate() {
        if yolo_used[j] {
            continue;
        }
        result.push(UiElement {
            id: 0,
            kind: ElementKind::Icon,
            text: None,
            rect: yolo_elem.rect,
            confidence: yolo_elem.confidence,
            source: ElementSource::Yolo,
        });
    }

    // ── 统一编号 ──
    for (i, elem) in result.iter_mut().enumerate() {
        elem.id = i as u32;
    }

    result
}

/// 一次加载图片同时执行 PaddleOCR + YOLO 并合并。
///
/// - OCR 模型缺失 → Err（明确错误，附缺失文件，供下载指引）。
/// - YOLO 模型缺失 → 仅 OCR 结果，`yolo_available = false`（诚实标注，不假装可用）。
/// - 返回 `PerceiveOutput`，含合并元素、OCR/YOLO 数量与 YOLO 可用性。
pub fn perceive_image(path: &str) -> Result<PerceiveOutput, String> {
    let models_dir = resolve_models_dir().ok_or_else(|| {
        "未找到模型目录，请设置 NUPHUS_MODELS_DIR 环境变量或确保模型文件位于 data_dir/Nuphus/models".to_string()
    })?;
    validate_ocr_models(&models_dir).map_err(|e| {
        format!(
            "{e}\n下载指引: 首次启动应用时会自动下载 PaddleOCR 模型（可在 设置 → 模型 → 本地视觉模型 查看进度）；\
             若自动下载失败，请重试或检查网络，也可手动从 \
             https://hf-mirror.com/SWHL/RapidOCR 与 https://gitee.com/paddlepaddle/PaddleOCR \
             下载后放入 {}",
            models_dir.display()
        )
    })?;

    // 一次解码，OCR 用 RGB、YOLO 用 RGBA Frame
    let img = image::open(path)
        .map_err(|e| format!("打开图像失败 {path}: {e}"))?
        .to_rgb8();
    let (w, h) = (img.width(), img.height());

    let mut ocr = PaddleOcr::new()?;
    let ocr_blocks = ocr.ocr_image_with_boxes(&img)?;

    let yolo_available = yolo_model_available(&models_dir);
    let yolo_elements: Vec<Element> = if yolo_available {
        let rgba = image::DynamicImage::ImageRgb8(img).to_rgba8();
        let frame = Frame {
            id: uuid::Uuid::new_v4(),
            pixels: rgba.into_raw(),
            width: w,
            height: h,
            scope: Scope::Fullscreen,
            timestamp: chrono::Utc::now(),
            source: FrameSource::Screenshot,
        };
        YoloDetector::new()
            .detect(&frame)
            .map_err(|e| format!("YOLO 检测失败: {e}"))?
    } else {
        vec![]
    };

    let elements = merge(&ocr_blocks, &yolo_elements);

    Ok(PerceiveOutput {
        elements,
        ocr_count: ocr_blocks.len(),
        yolo_count: yolo_elements.len(),
        yolo_available,
    })
}

/// perceive_image 的输出
#[derive(Debug, Clone)]
pub struct PerceiveOutput {
    pub elements: Vec<UiElement>,
    pub ocr_count: usize,
    pub yolo_count: usize,
    pub yolo_available: bool,
}

// ─── Kind 启发式推断 ────────────────────────────────────────────

/// 对 OCR + YOLO 合并的元素，根据文字内容和几何特征推断 ElementKind
fn infer_kind(text: &str, rect: &Rect) -> ElementKind {
    let trimmed = text.trim();

    // 规则 1：宽高比 > 3 → Input（输入框特征）
    if rect.h > 0 && (rect.w as f32 / rect.h as f32) > 3.0 {
        return ElementKind::Input;
    }

    // 规则 2：含特定关键字 → Input
    let lower = trimmed.to_lowercase();
    let input_keywords = [
        "输入",
        "搜索",
        "请输入",
        "password",
        "email",
        "用户名",
        "密码",
    ];
    for kw in &input_keywords {
        if lower.contains(kw) {
            return ElementKind::Input;
        }
    }

    // 规则 3：1-4 字符，不含标点 → Button
    let char_count = trimmed.chars().count();
    if (1..=4).contains(&char_count) && !contains_punctuation(trimmed) {
        return ElementKind::Button;
    }

    // 默认 → Text
    ElementKind::Text
}

/// 检查字符串是否含有标点符号
fn contains_punctuation(s: &str) -> bool {
    s.chars().any(|c| {
        c.is_ascii_punctuation()
            || c == '。'
            || c == '，'
            || c == '、'
            || c == '；'
            || c == '：'
            || c == '？'
            || c == '！'
            || c == '…'
            || c == '（'
            || c == '）'
            || c == '《'
            || c == '》'
            || c == '“'
            || c == '”'
            || c == '‘'
            || c == '’'
    })
}

// ─── 辅助函数 ───────────────────────────────────────────────────

/// 估计 OCR 文字块的置信度（基于文字长度）
fn block_confidence(text: &str) -> f32 {
    let len = text.trim().len();
    if len == 0 {
        0.0
    } else if len <= 2 {
        0.95
    } else {
        0.85
    }
}

/// 计算两个矩形框的 IoU（Intersection over Union）
fn rect_iou(a: &Rect, b: &Rect) -> f32 {
    let ax1 = a.x;
    let ay1 = a.y;
    let ax2 = a.x + a.w as i32;
    let ay2 = a.y + a.h as i32;

    let bx1 = b.x;
    let by1 = b.y;
    let bx2 = b.x + b.w as i32;
    let by2 = b.y + b.h as i32;

    let inter_x1 = ax1.max(bx1);
    let inter_y1 = ay1.max(by1);
    let inter_x2 = ax2.min(bx2);
    let inter_y2 = ay2.min(by2);

    if inter_x2 <= inter_x1 || inter_y2 <= inter_y1 {
        return 0.0;
    }

    let inter_area = (inter_x2 - inter_x1) as f32 * (inter_y2 - inter_y1) as f32;
    let area_a = (a.w * a.h) as f32;
    let area_b = (b.w * b.h) as f32;
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

// ─── 测试 ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str, x: i32, y: i32, w: i32, h: i32) -> OcrBlock {
        OcrBlock {
            text: text.into(),
            x,
            y,
            w,
            h,
        }
    }

    fn elem(x: i32, y: i32, w: u32, h: u32) -> Element {
        Element {
            kind: ElementKind::Icon,
            rect: Rect { x, y, w, h },
            confidence: 0.8,
        }
    }

    #[test]
    fn test_ocr_only() {
        let blocks = vec![block("hello", 10, 10, 100, 30)];
        let result = merge(&blocks, &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, ElementSource::Ocr);
        assert_eq!(result[0].kind, ElementKind::Text);
        assert_eq!(result[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn test_yolo_only() {
        let elements = vec![elem(50, 50, 30, 30)];
        let result = merge(&[], &elements);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, ElementSource::Yolo);
        assert_eq!(result[0].kind, ElementKind::Icon);
        assert_eq!(result[0].text, None);
    }

    #[test]
    fn test_merge_ocr_yolo() {
        let blocks = vec![block("确认", 45, 45, 40, 25)];
        let elements = vec![elem(40, 40, 50, 30)];
        let result = merge(&blocks, &elements);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, ElementSource::Both);
        assert_eq!(result[0].kind, ElementKind::Button); // 2 chars, no punct
        assert_eq!(result[0].text.as_deref(), Some("确认"));
    }

    #[test]
    fn test_merge_input_keyword() {
        let blocks = vec![block("请输入内容", 45, 45, 50, 30)];
        let elements = vec![elem(40, 40, 60, 35)];
        let result = merge(&blocks, &elements);
        assert_eq!(result[0].kind, ElementKind::Input);
    }

    #[test]
    fn test_merge_input_wide_aspect() {
        let blocks = vec![block("test", 50, 5, 300, 25)];
        let elements = vec![elem(0, 0, 400, 30)];
        let result = merge(&blocks, &elements);
        assert_eq!(result[0].kind, ElementKind::Input);
    }

    #[test]
    fn test_merge_no_overlap_keeps_both() {
        let blocks = vec![block("a", 0, 0, 10, 10), block("b", 100, 100, 10, 10)];
        let elements = vec![elem(300, 300, 20, 20)];
        let result = merge(&blocks, &elements);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].id, 0);
        assert_eq!(result[1].id, 1);
        assert_eq!(result[2].id, 2);
    }

    #[test]
    fn test_rect_iou_non_overlapping_is_zero() {
        assert_eq!(
            rect_iou(
                &Rect {
                    x: 0,
                    y: 0,
                    w: 10,
                    h: 10
                },
                &Rect {
                    x: 100,
                    y: 100,
                    w: 10,
                    h: 10
                }
            ),
            0.0
        );
    }
}
