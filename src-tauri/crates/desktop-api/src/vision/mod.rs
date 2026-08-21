//! 视觉感知模块 - 截图、OCR、找图、找字、找色

use crate::core::*;
use crate::utils::cleanup::CleanupQueue;
use std::sync::Arc;
use uuid::Uuid;

pub mod capture;
pub mod locate;
pub mod models;
pub mod ocr;
pub mod paddle_ocr;
pub mod perceive;
pub mod yolo;

pub use capture::*;
pub use locate::*;
pub use models::*;
pub use ocr::*;
pub use paddle_ocr::*;
pub use perceive::*;
pub use yolo::*;

/// 视觉引擎 - 感知层核心
pub struct VisionEngine {
    #[allow(dead_code)]
    cleanup: Arc<CleanupQueue>,
    ocr: OcrEngine,
    locate: Locator,
}

impl VisionEngine {
    pub fn new(cleanup: Arc<CleanupQueue>) -> Self {
        Self {
            cleanup,
            ocr: OcrEngine::new(),
            locate: Locator::new(),
        }
    }

    /// 感知 - 根据范围截图并分析
    pub async fn see(
        &self,
        target: &Target,
        scope: Scope,
        what: PerceiveWhat,
    ) -> Result<Perception> {
        // 1. 截图
        let frame = self.capture(target, scope).await?;

        // 2. 分析
        let mut perception = Perception {
            frame_id: frame.id,
            timestamp: frame.timestamp,
            scope,
            texts: vec![],
            elements: vec![],
            colors: vec![],
        };

        match what {
            PerceiveWhat::Text => {
                perception.texts = self.ocr.recognize(&frame).await?;
            }
            PerceiveWhat::Elements => {
                perception.elements = vec![];
            }
            PerceiveWhat::Colors => {
                perception.colors = self.locate.extract_colors(&frame)?;
            }
            PerceiveWhat::All => {
                perception.texts = self.ocr.recognize(&frame).await?;
                perception.elements = vec![];
                perception.colors = self.locate.extract_colors(&frame)?;
            }
        }

        Ok(perception)
    }

    /// 查找 - 找图/找字/找色
    pub async fn find(&self, frame: &Frame, query: &Query) -> Result<FindResult> {
        self.locate.find_with_fallback(frame, query).await
    }

    pub async fn capture(&self, target: &Target, scope: Scope) -> Result<Frame> {
        capture::capture(target, scope).await
    }
}

/// 感知内容类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerceiveWhat {
    Text,
    Elements,
    Colors,
    All,
}

/// 感知结果
#[derive(Debug, Clone)]
pub struct Perception {
    pub frame_id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub scope: Scope,
    pub texts: Vec<TextBlock>,
    pub elements: Vec<Element>,
    pub colors: Vec<ColorSpot>,
}

/// 文字块
#[derive(Debug, Clone)]
pub struct TextBlock {
    pub text: String,
    pub confidence: f32,
    pub rect: Rect,
}

/// 界面元素
#[derive(Debug, Clone)]
pub struct Element {
    pub kind: ElementKind,
    pub rect: Rect,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Button,
    Input,
    Text,
    Image,
    Icon,
    Unknown,
}

/// 颜色点
#[derive(Debug, Clone)]
pub struct ColorSpot {
    pub color: Color,
    pub point: Point,
    pub area: u32,
}

// ───────────────────────────── 偏色系统 ─────────────────────────────

/// RGB 三通道独立偏色（大漠风格 "203040" = dr=32, dg=48, db=64）
#[derive(Debug, Clone, Copy)]
pub struct DeltaColor {
    pub dr: u8, // R 通道最大偏差
    pub dg: u8, // G 通道最大偏差
    pub db: u8, // B 通道最大偏差
}

impl DeltaColor {
    /// 从单 tolerance 值构造（三个通道相同值）
    pub fn from_tolerance(tolerance: u8) -> Self {
        Self {
            dr: tolerance,
            dg: tolerance,
            db: tolerance,
        }
    }

    /// 从 RGB 字符串构造，格式如 "203040" → dr=32, dg=48, db=64
    /// 或 "10a0b0" → dr=0x10, dg=0xa0, db=0xb0
    pub fn from_hex_str(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.len() != 6 {
            return None;
        }
        let dr = u8::from_str_radix(&s[0..2], 16).ok()?;
        let dg = u8::from_str_radix(&s[2..4], 16).ok()?;
        let db = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { dr, dg, db })
    }

    /// 检查颜色是否在偏色范围内
    pub fn matches(&self, actual: &Color, target: &Color) -> bool {
        actual.r.abs_diff(target.r) <= self.dr
            && actual.g.abs_diff(target.g) <= self.dg
            && actual.b.abs_diff(target.b) <= self.db
    }
}

/// 扫描方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanDirection {
    /// 左上 → 右下（默认）
    #[default]
    LeftTop,
    /// 右上 → 左下
    RightTop,
    /// 左下 → 右上
    LeftBottom,
    /// 右下 → 左上
    RightBottom,
}

/// 查找查询
#[derive(Debug, Clone)]
pub enum Query {
    /// 找图 - 模板匹配
    Image(Vec<u8>), // PNG bytes
    /// 找字 - OCR 关键词
    Text(String),
    /// 找色 - 颜色匹配
    Color { target: Color, tolerance: u8 },
    /// 组合: 先找字，再验证颜色
    TextThenColor {
        text: String,
        color: Color,
        tolerance: u8,
    },
    /// 组合: 先找图，再验证文字
    ImageThenText { image: Vec<u8>, text: String },
}

/// 查找结果
#[derive(Debug, Clone)]
pub struct FindResult {
    pub found: bool,
    pub rect: Option<Rect>,
    pub confidence: f32,
    pub method: FindMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindMethod {
    ImageMatch,
    TextOcr,
    ColorScan,
    Combined,
    Fallback,
}
