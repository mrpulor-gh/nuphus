//! PaddleOCR ONNX Runtime 集成（desktop-api 共享层）。
//!
//! 逻辑移植自主 crate `src/desktop/paddle_ocr.rs`（PP-OCRv4 ONNX 本地 OCR），
//! 保持同一套预处理/推理/后处理管线；仅剥离主 crate 的 debug 探针与依赖模型
//! 文件的单元测试。运行需要 `onnxruntime.dll` 可加载（ort load-dynamic）且
//! 模型三件套位于 resolve_models_dir() 命中的目录。

use std::path::PathBuf;

use crate::vision::models::{
    resolve_models_dir, validate_ocr_models, PADDLE_DET_MODEL, PADDLE_DICT, PADDLE_REC_MODEL,
};

/// OCR 文本块（带位置）
#[derive(Debug, Clone)]
pub struct OcrBlock {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// PaddleOCR 引擎：文本检测（det）→ 逐框识别（rec）→ CTC 解码。
pub struct PaddleOcr {
    det_session: ort::session::Session,
    rec_session: ort::session::Session,
    char_dict: Vec<String>,
}

impl PaddleOcr {
    /// 初始化引擎：加载检测模型、识别模型、字符字典。
    /// 模型缺失时返回明确错误（含缺失路径，供下载指引）。
    pub fn new() -> Result<Self, String> {
        let models_dir = Self::models_dir()?;
        validate_ocr_models(&models_dir)?;

        let det_path = models_dir.join(PADDLE_DET_MODEL);
        let rec_path = models_dir.join(PADDLE_REC_MODEL);
        let dict_path = models_dir.join(PADDLE_DICT);

        // 加载 ONNX 会话
        let det_session = ort::session::Session::builder()
            .map_err(|e| format!("创建检测会话构建器失败: {e}"))?
            .commit_from_file(det_path)
            .map_err(|e| format!("加载检测模型失败: {e}"))?;

        let rec_session = ort::session::Session::builder()
            .map_err(|e| format!("创建识别会话构建器失败: {e}"))?
            .commit_from_file(rec_path)
            .map_err(|e| format!("加载识别模型失败: {e}"))?;

        // 加载字符字典
        let dict_content =
            std::fs::read_to_string(&dict_path).map_err(|e| format!("读取字典失败: {e}"))?;
        let char_dict: Vec<String> = dict_content.lines().map(|s| s.to_owned()).collect();

        if char_dict.len() < 100 {
            return Err(format!(
                "字典内容异常: 仅 {} 个字符 (预期 ~6623)",
                char_dict.len()
            ));
        }

        Ok(Self {
            det_session,
            rec_session,
            char_dict,
        })
    }

    /// OCR 识别图片文件，返回带位置的文本块。
    pub fn ocr_with_boxes(&mut self, image_path: &str) -> Result<Vec<OcrBlock>, String> {
        let img = image::open(image_path)
            .map_err(|e| format!("打开图像失败 {image_path}: {e}"))?
            .to_rgb8();
        self.ocr_image_with_boxes(&img)
    }

    /// OCR 识别内存 RGB 图像，返回带位置的文本块（不落盘）。
    pub fn ocr_image_with_boxes(&mut self, img: &image::RgbImage) -> Result<Vec<OcrBlock>, String> {
        // 阶段 1: 检测文本框
        let boxes = self.detect(img)?;

        if boxes.is_empty() {
            return Ok(vec![]);
        }

        // 阶段 2: 对每个文本框识别文字
        let (img_w, img_h) = (img.width(), img.height());
        let mut results: Vec<OcrBlock> = Vec::new();
        for &(x1, y1, x2, y2) in &boxes {
            let (cx1, cy1, cx2, cy2) = Self::expand_box((x1, y1, x2, y2), img_w, img_h);
            let crop = image::imageops::crop_imm(
                img,
                cx1,
                cy1,
                cx2.saturating_sub(cx1),
                cy2.saturating_sub(cy1),
            );
            let text = self.recognize(crop.to_image())?;
            if !text.is_empty() {
                results.push(OcrBlock {
                    text,
                    x: x1 as i32,
                    y: y1 as i32,
                    w: (x2.saturating_sub(x1)) as i32,
                    h: (y2.saturating_sub(y1)) as i32,
                });
            }
        }

        results.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
        Ok(results)
    }

    /// OCR 识别图片文件，返回纯文本（与 ocr_with_boxes 同一管线）。
    pub fn ocr(&mut self, image_path: &str) -> Result<String, String> {
        let img = image::open(image_path)
            .map_err(|e| format!("打开图像失败 {image_path}: {e}"))?
            .to_rgb8();
        self.ocr_image(&img)
    }

    /// OCR 识别内存中的 RGB 图像，返回纯文本。
    pub fn ocr_image(&mut self, img: &image::RgbImage) -> Result<String, String> {
        let blocks = self.ocr_image_with_boxes(img)?;
        Ok(blocks
            .into_iter()
            .map(|b| b.text)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    // ─── 文本检测 ───────────────────────────────────────────

    /// 文本检测：返回 [(x1, y1, x2, y2), ...]
    fn detect(&mut self, img: &image::RgbImage) -> Result<Vec<(u32, u32, u32, u32)>, String> {
        let (img_w, img_h) = (img.width() as f32, img.height() as f32);

        // 预处理
        let (input, scale, pad_x, pad_y) = Self::preprocess_det(img);

        // 推理
        let input_value = ort::value::TensorRef::from_array_view(input.view())
            .map_err(|e| format!("创建检测输入失败: {e}"))?;

        let outputs = self
            .det_session
            .run(ort::inputs!["x" => input_value])
            .map_err(|e| format!("检测推理失败: {e}"))?;

        // 后处理
        Self::postprocess_det(&outputs, img_w, img_h, scale, pad_x, pad_y)
    }

    /// 检测预处理：resize + normalize → NCHW f32 tensor
    fn preprocess_det(img: &image::RgbImage) -> (ndarray::Array4<f32>, f32, u32, u32) {
        let (w, h) = (img.width(), img.height());

        let max_size = 960.0;
        let limit_side_len = if w.max(h) as f32 > max_size {
            max_size
        } else {
            w.max(h) as f32
        };
        let ratio = if w > h {
            limit_side_len / w as f32
        } else {
            limit_side_len / h as f32
        };

        let new_w = ((w as f32 * ratio) as u32).max(32);
        let new_h = ((h as f32 * ratio) as u32).max(32);

        let pad_w = (new_w.div_ceil(32) * 32) - new_w;
        let pad_h = (new_h.div_ceil(32) * 32) - new_h;

        let resized =
            image::imageops::resize(img, new_w, new_h, image::imageops::FilterType::Triangle);

        let out_h = (new_h + pad_h) as usize;
        let out_w = (new_w + pad_w) as usize;

        // 直接构建 CHW 格式的 Array4
        let mean = [0.485f32, 0.456, 0.406];
        let std = [0.229f32, 0.224, 0.225];

        let mut array = ndarray::Array4::<f32>::zeros((1, 3, out_h, out_w));

        for y in 0..new_h as usize {
            for x in 0..new_w as usize {
                let p = resized.get_pixel(x as u32, y as u32);
                // PaddleOCR 使用 RGB 通道顺序（PIL 默认 / cv2 BGR2RGB）
                // image crate 返回 RGB，直接使用无需重排
                let rgb = [0usize, 1, 2];
                for c in 0..3 {
                    let val = p[rgb[c]] as f32 / 255.0;
                    let normalized = (val - mean[c]) / std[c];
                    array[[0, c, y, x]] = normalized;
                }
            }
        }
        // pad 区域已是 0

        (array, ratio, pad_w, pad_h)
    }

    /// 检测后处理：从模型输出中提取文本边界框
    fn postprocess_det(
        outputs: &ort::session::SessionOutputs,
        img_w: f32,
        img_h: f32,
        scale: f32,
        pad_w: u32,
        pad_h: u32,
    ) -> Result<Vec<(u32, u32, u32, u32)>, String> {
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取检测输出失败: {e}"))?;

        let ndim = shape.len();
        let dims: Vec<usize> = (0..ndim).map(|i| shape[i] as usize).collect();

        // 情况 1: 输出是 (N, 6) 格式 — [cls, score, x1, y1, x2, y2]
        if ndim == 2 && dims[1] == 6 {
            let num_boxes = dims[0];
            let mut boxes = Vec::new();
            for i in 0..num_boxes {
                let offset = i * 6;
                let score = data[offset + 1];
                if score < 0.5 {
                    continue;
                }
                let (x1, y1, x2, y2) = (
                    data[offset + 2],
                    data[offset + 3],
                    data[offset + 4],
                    data[offset + 5],
                );
                let (x1, y1, x2, y2) = if x1 <= 1.0 && x2 <= 1.0 && y1 <= 1.0 && y2 <= 1.0 {
                    (
                        (x1 * img_w) as u32,
                        (y1 * img_h) as u32,
                        (x2 * img_w) as u32,
                        (y2 * img_h) as u32,
                    )
                } else {
                    (
                        (x1 / scale).max(0.0) as u32,
                        (y1 / scale).max(0.0) as u32,
                        (x2 / scale).max(0.0) as u32,
                        (y2 / scale).max(0.0) as u32,
                    )
                };
                boxes.push((x1, y1, x2, y2));
            }
            return Ok(boxes);
        }

        // 情况 2: 输出是概率图 (1, 1, H, W)
        if ndim == 4 && dims[1] == 1 {
            let h = dims[2] as u32;
            let w = dims[3] as u32;
            let threshold: f32 = 0.3;

            let mut bitmap = vec![false; (h * w) as usize];
            for y in 0..h {
                for x in 0..w {
                    let v = data[(y * w + x) as usize];
                    if v > threshold {
                        bitmap[(y * w + x) as usize] = true;
                    }
                }
            }

            let min_area = 100.0;
            let boxes = Self::find_boxes_from_bitmap(&bitmap, w, h, min_area, scale, pad_w, pad_h);
            return Ok(boxes);
        }

        Err(format!(
            "不支持的检测输出维度: {:?}，预期 (N,6) 或 (1,1,H,W)",
            dims
        ))
    }

    /// 从二值位图中提取文本框（简化版连通区域检测）
    fn find_boxes_from_bitmap(
        bitmap: &[bool],
        w: u32,
        h: u32,
        min_area: f32,
        scale: f32,
        _pad_w: u32,
        _pad_h: u32,
    ) -> Vec<(u32, u32, u32, u32)> {
        let mut visited = vec![false; bitmap.len()];
        let mut boxes = Vec::new();
        let step = 4u32;

        for y in (0..h).step_by(step as usize) {
            for x in (0..w).step_by(step as usize) {
                let idx = (y * w + x) as usize;
                if !bitmap[idx] || visited[idx] {
                    continue;
                }

                let (mut min_x, mut max_x) = (x, x);
                let (mut min_y, mut max_y) = (y, y);
                let mut stack = vec![(x, y)];
                visited[idx] = true;
                let mut area = 0u32;

                while let Some((cx, cy)) = stack.pop() {
                    area += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    for (nx, ny) in [
                        (cx.wrapping_sub(1), cy),
                        (cx + 1, cy),
                        (cx, cy.wrapping_sub(1)),
                        (cx, cy + 1),
                    ] {
                        if nx < w && ny < h {
                            let ni = (ny * w + nx) as usize;
                            if bitmap[ni] && !visited[ni] {
                                visited[ni] = true;
                                stack.push((nx, ny));
                            }
                        }
                    }
                }

                if area as f32 >= min_area {
                    let orig_x1 = (min_x as f32 / scale).max(0.0) as u32;
                    let orig_y1 = (min_y as f32 / scale).max(0.0) as u32;
                    let orig_x2 = (max_x as f32 / scale).max(0.0) as u32;
                    let orig_y2 = (max_y as f32 / scale).max(0.0) as u32;
                    boxes.push((orig_x1, orig_y1, orig_x2, orig_y2));
                }
            }
        }

        boxes
    }

    /// 文本框外扩（提升识别准确率）
    fn expand_box(
        (x1, y1, x2, y2): (u32, u32, u32, u32),
        img_w: u32,
        img_h: u32,
    ) -> (u32, u32, u32, u32) {
        let bw = (x2.saturating_sub(x1)) as f32;
        let bh = (y2.saturating_sub(y1)) as f32;
        if bw < 1.0 || bh < 1.0 {
            return (x1, y1, x2, y2);
        }
        let margin_x = (bw * 0.05) as u32;
        let margin_y = (bh * 0.10) as u32;
        (
            x1.saturating_sub(margin_x),
            y1.saturating_sub(margin_y),
            (x2 + margin_x).min(img_w),
            (y2 + margin_y).min(img_h),
        )
    }

    // ─── 文本识别 ───────────────────────────────────────────

    /// 识别单个文本区域：预处理 → 推理 → CTC 解码
    fn recognize(&mut self, crop: image::RgbImage) -> Result<String, String> {
        let input = Self::preprocess_rec(&crop);
        let input_value = ort::value::TensorRef::from_array_view(input.view())
            .map_err(|e| format!("创建识别输入失败: {e}"))?;

        let outputs = self
            .rec_session
            .run(ort::inputs!["x" => input_value])
            .map_err(|e| format!("识别推理失败: {e}"))?;

        Self::ctc_decode(&self.char_dict, &outputs)
    }

    /// 识别预处理：resize → normalize → NCHW（高 48、宽 320，右侧补 0）
    fn preprocess_rec(img: &image::RgbImage) -> ndarray::Array4<f32> {
        let (w, h) = (img.width(), img.height());
        let target_h = 48u32;
        let target_w = 320u32;

        let ratio = target_h as f32 / h as f32;
        let new_w = (w as f32 * ratio) as u32;
        let resized = image::imageops::resize(
            img,
            new_w.min(target_w),
            target_h,
            image::imageops::FilterType::Triangle,
        );

        // 右侧补 0 到 target_w（PaddleOCR 训练用 320 宽）
        let mean = [0.5f32, 0.5, 0.5];
        let std = [0.5f32, 0.5, 0.5];

        let mut array = ndarray::Array4::<f32>::zeros((1, 3, target_h as usize, target_w as usize));

        for y in 0..target_h as usize {
            for x in 0..resized.width() as usize {
                let p = resized.get_pixel(x as u32, y as u32);
                // PaddleOCR 使用 RGB 通道顺序
                let rgb = [0usize, 1, 2];
                for c in 0..3 {
                    let val = p[rgb[c]] as f32 / 255.0;
                    let normalized = (val - mean[c]) / std[c];
                    array[[0, c, y, x]] = normalized;
                }
            }
        }

        array
    }

    /// CTC 解码：从识别模型输出中解码文本
    fn ctc_decode(
        char_dict: &[String],
        outputs: &ort::session::SessionOutputs,
    ) -> Result<String, String> {
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("提取识别输出失败: {e}"))?;

        let ndim = shape.len();
        if ndim != 3 {
            return Err(format!(
                "不支持的识别输出维度: {:?}",
                (0..ndim).map(|i| shape[i]).collect::<Vec<_>>()
            ));
        }

        let (seq_len, num_classes) = (shape[1] as usize, shape[2] as usize);
        // PaddleOCR 训练约定：
        // - class 0  = CTC blank（skip）
        // - class 1..=char_dict.len() = 字典字符（dict[0..]）
        // - class num_classes-1 = space 字符（保留，不 skip！）
        let blank_id = 0usize;
        let space_id = num_classes.saturating_sub(1);

        let mut prev = blank_id;
        let mut chars = Vec::new();

        for t in 0..seq_len {
            let mut max_val = f32::NEG_INFINITY;
            let mut max_idx = blank_id;
            let offset = t * num_classes;
            for c in 0..num_classes {
                let val = data[offset + c];
                if val > max_val {
                    max_val = val;
                    max_idx = c;
                }
            }

            if max_idx != blank_id && max_idx != prev {
                if max_idx == space_id {
                    chars.push(" ".to_string());
                } else if max_idx >= 1 && max_idx <= char_dict.len() {
                    chars.push(char_dict[max_idx - 1].clone());
                }
            }
            prev = max_idx;
        }

        Ok(chars.join(""))
    }

    // ─── 工具函数 ───────────────────────────────────────────

    /// 获取模型目录 — 委托给公共 resolve_models_dir()
    fn models_dir() -> Result<PathBuf, String> {
        resolve_models_dir().ok_or_else(|| {
            "未找到模型目录，请设置 NUPHUS_MODELS_DIR 环境变量或确保模型文件在正确位置".to_string()
        })
    }
}
