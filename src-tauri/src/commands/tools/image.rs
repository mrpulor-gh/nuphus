// tools/image.rs — 图片内部机制命令（工具页手动调用，非 agent 工具）
//
// 移植自 nuphus-tools-rs engines/image.rs，修复已知缺陷：
// - resize_exact 变形 → 改为保纵横比（contain，不放大）
// - PNG「压缩」名不副实 → PngEncoder::new_with_quality(Best + Adaptive) 真压缩
// - 无资源上限 → 解码前 image::image_dimensions 像素上限检查，防 OOM
// - to_str().unwrap() panic → to_string_lossy + 错误传播
// - 输出格式随扩展名推断（png/jpg/bmp/gif），未知扩展名报错而非静默

use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{image_dimensions, DynamicImage, GenericImageView, ImageFormat};

/// 像素上限（5000 万）：解码前检查，超出直接拒绝，防超大图 OOM
const MAX_PIXELS: u64 = 50_000_000;
/// 输入文件大小上限（300MB）
const IMAGE_MAX_BYTES: u64 = 300 * 1024 * 1024;
/// 图片文件扩展名白名单（image::open 依赖这些格式的 decode feature）
const ALLOWED_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];

fn ensure_input_image(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_file() {
        return Err(format!("图片文件不存在：{}", path));
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !ALLOWED_EXTS.contains(&ext.as_str()) {
        return Err(format!("不支持的图片格式：{}", ext));
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("读取图片信息失败：{}", e))?;
    if meta.len() > IMAGE_MAX_BYTES {
        return Err(format!(
            "图片超过 {}MB 上限",
            IMAGE_MAX_BYTES / 1024 / 1024
        ));
    }
    // 解码前检查像素总量（OOM 防线：解码一幅 2 亿像素图可吃掉数 GB 内存）
    let (w, h) = image_dimensions(&p).map_err(|e| format!("读取图片尺寸失败：{}", e))?;
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(format!(
            "图片分辨率 {}×{} 超过 {} 万像素上限，请先缩小后再处理",
            w,
            h,
            MAX_PIXELS / 10_000
        ));
    }
    Ok(p)
}

/// 输出扩展名 → ImageFormat；未知返回 None（调用方报错）
fn output_format(path: &Path) -> Option<ImageFormat> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "bmp" => Some(ImageFormat::Bmp),
        "gif" => Some(ImageFormat::Gif),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn ensure_output_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }
    Ok(())
}

/// 保纵横比缩放（contain：目标框内完整放下，不放大）
fn contain_dimensions(width: u32, height: u32, max_w: Option<u32>, max_h: Option<u32>) -> (u32, u32) {
    let target_w = max_w.unwrap_or(width).max(1);
    let target_h = max_h.unwrap_or(height).max(1);
    if width <= target_w && height <= target_h {
        return (width, height);
    }
    let scale = f64::min(target_w as f64 / width as f64, target_h as f64 / height as f64);
    let scale = f64::min(scale, 1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

/// 按输出扩展名写文件：JPEG 用 quality，PNG 用真压缩（Best + Adaptive），
/// 其余格式直接保存。
fn save_image(
    img: &DynamicImage,
    output: &Path,
    quality: u8,
) -> Result<u64, String> {
    let format = output_format(output)
        .ok_or_else(|| "输出扩展名必须是 png / jpg / jpeg / bmp / gif / webp 之一".to_string())?;
    let file = std::fs::File::create(output).map_err(|e| format!("创建输出文件失败：{}", e))?;
    let mut writer = std::io::BufWriter::new(file);
    match format {
        ImageFormat::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(&mut writer, quality.min(100));
            img.write_with_encoder(encoder)
                .map_err(|e| format!("JPEG 编码失败：{}", e))?;
        }
        ImageFormat::Png => {
            let encoder = PngEncoder::new_with_quality(
                &mut writer,
                CompressionType::Best,
                FilterType::Adaptive,
            );
            img.write_with_encoder(encoder)
                .map_err(|e| format!("PNG 编码失败：{}", e))?;
        }
        _ => {
            img.write_to(&mut writer, format)
                .map_err(|e| format!("编码失败：{}", e))?;
        }
    }
    drop(writer);
    std::fs::metadata(output)
        .map(|m| m.len())
        .map_err(|e| format!("读取输出大小失败：{}", e))
}

/// 压缩图片：可指定最大宽/高（保纵横比）+ 质量；输出格式随扩展名推断。
/// JPEG 质量参数生效；PNG 走 Best+Adaptive 真压缩。
#[tauri::command]
pub fn image_compress(
    input_path: String,
    output_path: String,
    max_width: Option<u32>,
    max_height: Option<u32>,
    quality: Option<u8>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_image(&input_path)?;
    let output = PathBuf::from(&output_path);
    if output_format(&output).is_none() {
        return Err("输出扩展名必须是 png / jpg / jpeg / bmp / gif / webp 之一".to_string());
    }
    ensure_output_dir(&output)?;

    let img = image::open(&input).map_err(|e| format!("打开图片失败：{}", e))?;
    let (w, h) = img.dimensions();
    let (new_w, new_h) = contain_dimensions(w, h, max_width, max_height);
    let resized = if (new_w, new_h) != (w, h) {
        img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let size_before = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
    let size_after = save_image(&resized, &output, quality.unwrap_or(82))?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "width": new_w,
        "height": new_h,
        "size_before": size_before,
        "size_after": size_after,
        "saved_bytes": size_before.saturating_sub(size_after),
    }))
}

/// 格式转换：输出格式随扩展名推断（JPEG 默认 quality 90）
#[tauri::command]
pub fn image_convert(
    input_path: String,
    output_path: String,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_image(&input_path)?;
    let output = PathBuf::from(&output_path);
    let format = output_format(&output)
        .ok_or_else(|| "输出扩展名必须是 png / jpg / jpeg / bmp / gif 之一".to_string())?;
    ensure_output_dir(&output)?;

    let img = image::open(&input).map_err(|e| format!("打开图片失败：{}", e))?;
    let size_after = save_image(&img, &output, 90)?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "width": img.width(),
        "height": img.height(),
        "format": format!("{:?}", format).to_ascii_lowercase(),
        "size_after": size_after,
    }))
}

/// 缩放图片：目标宽高框内保纵横比缩放（contain，不放大）
#[tauri::command]
pub fn image_resize(
    input_path: String,
    output_path: String,
    width: u32,
    height: u32,
) -> Result<serde_json::Value, String> {
    if width == 0 || height == 0 {
        return Err("目标宽高必须大于 0".to_string());
    }
    let input = ensure_input_image(&input_path)?;
    let output = PathBuf::from(&output_path);
    if output_format(&output).is_none() {
        return Err("输出扩展名必须是 png / jpg / jpeg / bmp / gif 之一".to_string());
    }
    ensure_output_dir(&output)?;

    let img = image::open(&input).map_err(|e| format!("打开图片失败：{}", e))?;
    let (w, h) = img.dimensions();
    let (new_w, new_h) = contain_dimensions(w, h, Some(width), Some(height));
    let resized = if (new_w, new_h) != (w, h) {
        img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let size_after = save_image(&resized, &output, 90)?;

    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "source_width": w,
        "source_height": h,
        "width": new_w,
        "height": new_h,
        "size_after": size_after,
    }))
}

/// 获取图片信息（尺寸 / 文件大小 / 格式）
#[tauri::command]
pub fn image_info(path: String) -> Result<serde_json::Value, String> {
    let input = ensure_input_image(&path)?;
    let img = image::open(&input).map_err(|e| format!("打开图片失败：{}", e))?;
    let (w, h) = img.dimensions();
    let size = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
    let format = ImageFormat::from_path(&input)
        .map(|f| format!("{:?}", f).to_ascii_lowercase())
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(serde_json::json!({
        "path": input.display().to_string(),
        "width": w,
        "height": h,
        "size_bytes": size,
        "format": format,
    }))
}
/// 长图拼接：多张图片拼接为一张。direction: horizontal（横向，统一高度）/
/// vertical（纵向，统一宽度）。输出格式随扩展名推断。
#[tauri::command]
pub fn image_stitch(
    input_paths: Vec<String>,
    output_path: String,
    direction: Option<String>,
) -> Result<serde_json::Value, String> {
    if input_paths.is_empty() {
        return Err("至少需要选择一张图片".to_string());
    }
    let dir = direction.unwrap_or_else(|| "horizontal".to_string());
    if dir != "horizontal" && dir != "vertical" {
        return Err("拼接方向必须是 horizontal 或 vertical".to_string());
    }
    let output = PathBuf::from(&output_path);
    if output_format(&output).is_none() {
        return Err("输出扩展名必须是 png / jpg / jpeg / bmp / gif / webp 之一".to_string());
    }
    ensure_output_dir(&output)?;

    // 预检 + 解码全部输入（像素上限在 ensure_input_image 内）
    let mut imgs: Vec<DynamicImage> = Vec::with_capacity(input_paths.len());
    for p in &input_paths {
        let input = ensure_input_image(p)?;
        imgs.push(image::open(&input).map_err(|e| format!("打开图片失败（{}）：{}", p, e))?);
    }

    // 画布尺寸：横向 = 各宽之和 + 统一高度（取最大）；纵向 = 统一宽度（取最大）+ 各高之和
    let (canvas_w, canvas_h, unit, is_horizontal) = if dir == "horizontal" {
        let h = imgs.iter().map(|i| i.height()).max().unwrap_or(1);
        let w: u32 = imgs.iter().map(|i| i.width()).sum();
        (w, h, h, true)
    } else {
        let w = imgs.iter().map(|i| i.width()).max().unwrap_or(1);
        let h: u32 = imgs.iter().map(|i| i.height()).sum();
        (w, h, w, false)
    };

    let mut canvas = DynamicImage::new_rgb8(canvas_w, canvas_h);
    let mut offset: u32 = 0;
    for img in imgs {
        // 统一缩放：横向统一高度，纵向统一宽度（保纵横比）
        let scaled = if is_horizontal {
            if img.height() != unit {
                img.resize(0, unit, image::imageops::FilterType::Lanczos3)
            } else {
                img
            }
        } else if img.width() != unit {
            img.resize(unit, 0, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let (x, y) = if is_horizontal { (offset, 0) } else { (0, offset) };
        image::imageops::overlay(&mut canvas, &scaled, x.into(), y.into());
        offset += if is_horizontal { scaled.width() } else { scaled.height() };
    }

    let size_after = save_image(&canvas, &output, 90)?;
    Ok(serde_json::json!({
        "output": output.display().to_string(),
        "width": canvas_w,
        "height": canvas_h,
        "direction": dir,
        "sources": input_paths.len(),
        "size_after": size_after,
    }))
}
/// 批量压缩图片：多张输入 → 输出目录，每张按输入扩展名保存为 `原名-out.扩展名`。
/// 复用单文件压缩逻辑（保纵横比 + JPEG quality / PNG Best 压缩）。
#[tauri::command]
pub fn image_compress_batch(
    input_paths: Vec<String>,
    output_dir: String,
    max_width: Option<u32>,
    max_height: Option<u32>,
    quality: Option<u8>,
) -> Result<serde_json::Value, String> {
    if input_paths.is_empty() {
        return Err("至少需要选择一张图片".to_string());
    }
    let dir = PathBuf::from(&output_dir);
    if dir.is_file() {
        return Err("输出路径是文件，应为目录".to_string());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建输出目录失败：{}", e))?;

    let mut results = Vec::with_capacity(input_paths.len());
    for p in &input_paths {
        let input = ensure_input_image(p)?;
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string();
        let ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let output = dir.join(format!("{}-out.{}", stem, ext));

        let img = image::open(&input).map_err(|e| format!("打开图片失败（{}）：{}", p, e))?;
        let (w, h) = img.dimensions();
        let (new_w, new_h) = contain_dimensions(w, h, max_width, max_height);
        let resized = if (new_w, new_h) != (w, h) {
            img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let size_before = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
        let size_after = save_image(&resized, &output, quality.unwrap_or(82))?;
        results.push(serde_json::json!({
            "input": p,
            "output": output.display().to_string(),
            "saved_bytes": size_before.saturating_sub(size_after),
            "size_after": size_after,
        }));
    }

    Ok(serde_json::json!({
        "output_dir": dir.display().to_string(),
        "count": results.len(),
        "results": results,
    }))
}

/// 批量格式转换：多张输入 → 输出目录，统一转为 format（png/jpg/bmp/gif/webp）。
#[tauri::command]
pub fn image_convert_batch(
    input_paths: Vec<String>,
    output_dir: String,
    format: String,
) -> Result<serde_json::Value, String> {
    if input_paths.is_empty() {
        return Err("至少需要选择一张图片".to_string());
    }
    let fmt = format.to_ascii_lowercase();
    if !["png", "jpg", "jpeg", "bmp", "gif", "webp"].contains(&fmt.as_str()) {
        return Err("目标格式必须是 png / jpg / jpeg / bmp / gif / webp 之一".to_string());
    }
    let dir = PathBuf::from(&output_dir);
    if dir.is_file() {
        return Err("输出路径是文件，应为目录".to_string());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建输出目录失败：{}", e))?;

    let out_ext = if fmt == "jpeg" { "jpg" } else { fmt.as_str() };
    let mut results = Vec::with_capacity(input_paths.len());
    for p in &input_paths {
        let input = ensure_input_image(p)?;
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string();
        let output = dir.join(format!("{}.{}", stem, out_ext));

        let img = image::open(&input).map_err(|e| format!("打开图片失败（{}）：{}", p, e))?;
        let (w, h) = img.dimensions();
        let size_after = save_image(&img, &output, 90)?;
        results.push(serde_json::json!({
            "input": p,
            "output": output.display().to_string(),
            "width": w,
            "height": h,
            "size_after": size_after,
        }));
    }

    Ok(serde_json::json!({
        "output_dir": dir.display().to_string(),
        "format": fmt,
        "count": results.len(),
        "results": results,
    }))
}
