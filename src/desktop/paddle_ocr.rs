//! PaddleOCR ONNX Runtime 集成
//!
//! 使用 PP-OCRv4 ONNX 模型进行文本检测和识别。
//! 需要 onnxruntime.dll 在 PATH 或 exe 同目录中。

use std::path::PathBuf;
use tracing;

/// OCR 文本块（带位置）
#[derive(Debug, Clone)]
pub struct OcrBlock {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

// ─── Debug 探针：保存中间产物（仅在 debug 编译时激活）─────────
#[cfg(debug_assertions)]
mod debug_probes {
    use std::sync::atomic::{AtomicU32, Ordering};

    static DEBUG_CROP_ID: AtomicU32 = AtomicU32::new(0);
    static DEBUG_REC_ID: AtomicU32 = AtomicU32::new(0);

    pub fn reset_counters() {
        DEBUG_CROP_ID.store(0, Ordering::SeqCst);
        DEBUG_REC_ID.store(0, Ordering::SeqCst);
    }

    pub fn save_crop(crop: &image::RgbImage, box_coords: (u32, u32, u32, u32)) {
        let id = DEBUG_CROP_ID.fetch_add(1, Ordering::SeqCst);
        let dir = std::path::Path::new("debug_ocr");
        std::fs::create_dir_all(dir).ok();
        let (x1, y1, x2, y2) = box_coords;
        let path = dir.join(format!(
            "crop_{:04}_x{}y{}w{}h{}.bmp",
            id,
            x1,
            y1,
            x2.saturating_sub(x1),
            y2.saturating_sub(y1)
        ));
        if let Err(e) = crop.save(&path) {
            tracing::debug!("[OCR DEBUG] Failed to save crop: {e}");
        } else {
            tracing::debug!("[OCR DEBUG] Saved crop: {}", path.display());
        }
    }

    pub fn save_rec_result(text: &str, raw_indices: &[usize]) {
        let id = DEBUG_REC_ID.fetch_add(1, Ordering::SeqCst);
        let dir = std::path::Path::new("debug_ocr");
        std::fs::create_dir_all(dir).ok();
        let path = dir.join("rec_results.txt");
        let line = format!(
            "crop_{:04} | {} | {:?}\n",
            id,
            text,
            &raw_indices[..raw_indices.len().min(40)]
        );
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            f.write_all(line.as_bytes()).ok();
        }
    }

    pub fn save_det_visualization(img: &image::RgbImage, boxes: &[(u32, u32, u32, u32)]) {
        let dir = std::path::Path::new("debug_ocr");
        std::fs::create_dir_all(dir).ok();
        let path = dir.join("det_boxes.bmp");
        let mut vis = img.clone();
        let red = image::Rgb([255u8, 0, 0]);
        for &(x1, y1, x2, y2) in boxes {
            let x2 = x2.min(vis.width());
            let y2 = y2.min(vis.height());
            for x in x1..x2 {
                vis.put_pixel(x, y1, red);
                if y2 > 0 {
                    vis.put_pixel(x, y2 - 1, red);
                }
            }
            for y in y1..y2 {
                vis.put_pixel(x1, y, red);
                if x2 > 0 {
                    vis.put_pixel(x2 - 1, y, red);
                }
            }
        }
        if let Err(e) = vis.save(&path) {
            tracing::debug!("[OCR DEBUG] Failed to save det vis: {e}");
        } else {
            tracing::debug!(
                "[OCR DEBUG] Saved det visualization: {} ({} boxes)",
                path.display(),
                boxes.len()
            );
        }
    }
}

/// PaddleOCR 引擎（封装检测 + 识别两阶段 ONNX 模型）
pub struct PaddleOcr {
    det_session: ort::session::Session,
    rec_session: ort::session::Session,
    char_dict: Vec<String>,
}

impl PaddleOcr {
    /// 初始化引擎：加载检测模型、识别模型、字符字典
    pub fn new() -> Result<Self, String> {
        let models_dir = Self::models_dir()?;

        let det_path = models_dir.join("ch_PP-OCRv4_det.onnx");
        let rec_path = models_dir.join("ch_PP-OCRv4_rec.onnx");
        let dict_path = models_dir.join("ch_PP-OCR_keys_v1.txt");

        if !det_path.exists() {
            return Err(format!(
                "检测模型未找到: {}\n重新构建时会自动下载（src-tauri/build.rs）",
                det_path.display()
            ));
        }
        if !rec_path.exists() {
            return Err(format!(
                "识别模型未找到: {}\n重新构建时会自动下载（src-tauri/build.rs）",
                rec_path.display()
            ));
        }

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

    /// OCR 识别图片，返回纯文本
    pub fn ocr(&mut self, image_path: &str) -> Result<String, String> {
        let img = image::open(image_path)
            .map_err(|e| format!("打开图像失败 {image_path}: {e}"))?
            .to_rgb8();
        self.ocr_image(&img)
    }

    /// OCR 识别内存中的 RGB 图像，返回纯文本
    ///
    /// 与 `ocr` 同一管线，只是输入不落盘——供 PDF 渲染兜底等内存链路使用。
    pub fn ocr_image(&mut self, img: &image::RgbImage) -> Result<String, String> {
        // 重置 debug 计数器（仅在 debug 编译时）
        #[cfg(debug_assertions)]
        {
            debug_probes::reset_counters();
            // 清空上次的 rec_results.txt
            _ = std::fs::remove_file("debug_ocr/rec_results.txt");
        }

        // 阶段 1: 检测文本框
        let boxes = self.detect(img)?;

        if boxes.is_empty() {
            return Ok(String::new());
        }

        // 阶段 2: 对每个文本框识别文字
        let (img_w, img_h) = (img.width(), img.height());
        let mut results: Vec<(u32, String)> = Vec::new();
        for &box_coords in &boxes {
            let (cx1, cy1, cx2, cy2) = Self::expand_box(box_coords, img_w, img_h);
            let crop = image::imageops::crop_imm(
                img,
                cx1,
                cy1,
                cx2.saturating_sub(cx1),
                cy2.saturating_sub(cy1),
            );
            #[cfg(debug_assertions)]
            debug_probes::save_crop(&crop.to_image(), (cx1, cy1, cx2, cy2));
            let text = self.recognize(crop.to_image())?;
            if !text.is_empty() {
                results.push((box_coords.1, text));
            }
        }

        // 按 y 坐标排序（从上到下）
        results.sort_by_key(|(y, _)| *y);

        let full_text: String = results
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(full_text)
    }

    /// OCR 识别图片，返回带位置的文本块
    pub fn ocr_with_boxes(&mut self, image_path: &str) -> Result<Vec<OcrBlock>, String> {
        let img = image::open(image_path)
            .map_err(|e| format!("打开图像失败 {image_path}: {e}"))?
            .to_rgb8();

        // 重置 debug 计数器（仅在 debug 编译时）
        #[cfg(debug_assertions)]
        {
            debug_probes::reset_counters();
            // 清空上次的 rec_results.txt
            _ = std::fs::remove_file("debug_ocr/rec_results.txt");
        }

        let boxes = self.detect(&img)?;

        let (img_w, img_h) = (img.width(), img.height());
        let mut results = Vec::new();
        for &(x1, y1, x2, y2) in &boxes {
            let (cx1, cy1, cx2, cy2) = Self::expand_box((x1, y1, x2, y2), img_w, img_h);
            let crop = image::imageops::crop_imm(
                &img,
                cx1,
                cy1,
                cx2.saturating_sub(cx1),
                cy2.saturating_sub(cy1),
            );
            #[cfg(debug_assertions)]
            debug_probes::save_crop(&crop.to_image(), (cx1, cy1, cx2, cy2));
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

    // ─── 检测阶段 ───────────────────────────────────────────

    /// 对检测框添加边距，避免裁剪时切掉文字边缘像素
    /// 参考 PP-OCR 标准做法，对检测框添加边距
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
        let boxes = Self::postprocess_det(&outputs, img_w, img_h, scale, pad_x, pad_y)?;
        #[cfg(debug_assertions)]
        debug_probes::save_det_visualization(img, &boxes);
        Ok(boxes)
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

    // ─── 识别阶段 ───────────────────────────────────────────

    /// 文本识别：对裁剪区域进行识别
    fn recognize(&mut self, crop: image::RgbImage) -> Result<String, String> {
        let (cw, ch) = (crop.width(), crop.height());
        if cw == 0 || ch == 0 {
            return Ok(String::new());
        }

        let input = Self::preprocess_rec(&crop);

        let input_value = ort::value::TensorRef::from_array_view(input.view())
            .map_err(|e| format!("创建识别输入失败: {e}"))?;

        let outputs = self
            .rec_session
            .run(ort::inputs!["x" => input_value])
            .map_err(|e| format!("识别推理失败: {e}"))?;

        let text = Self::ctc_decode(&self.char_dict, &outputs)?;
        Ok(text)
    }

    /// 识别预处理：resize → normalize → NCHW
    fn preprocess_rec(crop: &image::RgbImage) -> ndarray::Array4<f32> {
        let (w, h) = (crop.width(), crop.height());
        let target_h = 48u32;
        let target_w = 320u32;

        let ratio = target_h as f32 / h as f32;
        let new_w = (w as f32 * ratio) as u32;
        let resized = image::imageops::resize(
            crop,
            new_w.min(target_w),
            target_h,
            image::imageops::FilterType::Triangle,
        );

        let mean = [0.5f32, 0.5, 0.5];
        let std = [0.5f32, 0.5, 0.5];

        let mut array = ndarray::Array4::<f32>::zeros((1, 3, target_h as usize, target_w as usize));

        for y in 0..target_h as usize {
            for x in 0..resized.width() as usize {
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
        tracing::debug!(
            "[OCR DEBUG] seq_len={seq_len}, num_classes={num_classes}, char_dict.len={}",
            char_dict.len()
        );
        // PaddleOCR 训练约定：
        // - class 0  = CTC blank（skip）
        // - class 1..=char_dict.len() = 字典字符（dict[0..]）
        // - class num_classes-1 = space 字符（保留，不 skip！）
        let blank_id = 0usize;
        let space_id = num_classes.saturating_sub(1);

        let mut prev = blank_id;
        let mut chars = Vec::new();
        #[allow(unused_variables, unused_mut)]
        let mut debug_indices = Vec::new(); // debug: raw argmax

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
            if cfg!(debug_assertions) {
                debug_indices.push(max_idx); // debug
            }

            if max_idx != blank_id && max_idx != prev {
                if max_idx == space_id {
                    chars.push(" ".to_string());
                } else if max_idx >= 1 && max_idx <= char_dict.len() {
                    // 索引 1..=char_dict.len() 映射到 dict[0..]
                    chars.push(char_dict[max_idx - 1].clone());
                }
            }
            prev = max_idx;
        }

        let result = chars.join("");
        #[cfg(debug_assertions)]
        {
            tracing::debug!(
                "[OCR DEBUG] raw indices (first 40): {:?}",
                &debug_indices[..debug_indices.len().min(40)]
            );
            tracing::debug!("[OCR DEBUG] result: {result}");
            debug_probes::save_rec_result(&result, &debug_indices);
        }
        Ok(result)
    }

    // ─── 工具函数 ───────────────────────────────────────────

    /// 获取模型目录 — 委托给公共 resolve_models_dir()
    fn models_dir() -> Result<PathBuf, String> {
        crate::desktop::resolve_models_dir().ok_or_else(|| {
            "未找到模型目录，请设置 NUPHUS_MODELS_DIR 环境变量或确保模型文件在正确位置".to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_exist() {
        let dir = PaddleOcr::models_dir().expect("models_dir");
        assert!(
            dir.join("ch_PP-OCRv4_det.onnx").exists(),
            "det model not found at {}",
            dir.display()
        );
        assert!(
            dir.join("ch_PP-OCRv4_rec.onnx").exists(),
            "rec model not found at {}",
            dir.display()
        );
        assert!(
            dir.join("ch_PP-OCR_keys_v1.txt").exists(),
            "dict not found at {}",
            dir.display()
        );
    }

    #[test]
    fn test_dict_content() {
        let dir = PaddleOcr::models_dir().expect("models_dir");
        let dict = std::fs::read_to_string(dir.join("ch_PP-OCR_keys_v1.txt")).expect("read dict");
        let lines: Vec<&str> = dict.lines().collect();
        assert!(lines.len() > 100, "dict too short: {} lines", lines.len());
        assert!(lines.contains(&"的"), "dict missing common char '的'");
    }
}
