//! OCR 实现 - 待 PaddleOCR 集成替换
//!
//! 当前为空壳实现，等待 ONNX Runtime + PaddleOCR 集成。

use crate::core::*;
use crate::vision::TextBlock;

/// 流式 OCR 结果
#[derive(Debug, Clone)]
pub enum OcrChunk {
    /// 快速结果 (低精度，先返回)
    Quick(Vec<TextBlock>),
    /// 详细结果 (高精度，后返回)
    Detailed(Vec<TextBlock>),
}

/// OCR 引擎（空壳，等待 PaddleOCR 集成）
pub struct OcrEngine;

impl Default for OcrEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl OcrEngine {
    pub fn new() -> Self {
        Self
    }

    /// 识别 - 全帧 OCR（当前返回空结果）
    pub async fn recognize(&self, _frame: &Frame) -> Result<Vec<TextBlock>> {
        Ok(vec![])
    }

    /// 快速识别 - 只找关键词区域
    pub async fn recognize_quick(
        &self,
        frame: &Frame,
        _keywords: &[String],
    ) -> Result<Vec<TextBlock>> {
        self.recognize(frame).await
    }

    /// 区域识别 - 只识别指定区域
    pub async fn recognize_region(&self, _frame: &Frame, _region: Rect) -> Result<Vec<TextBlock>> {
        Ok(vec![])
    }
}
