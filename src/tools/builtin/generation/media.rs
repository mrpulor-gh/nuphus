//! media — 生成输入的媒体引用归一化（协议无关）
//!
//! 生成端（任何厂商）只认识「可被服务端取到的图片」：公网 URL 或 data URI。
//! 本地文件路径在协议层没有任何意义，因此「本地文件 → data URI」必须在语义层
//! 统一完成——协议实现不得各自读取文件，否则每家协议都要复制一份文件 IO，
//! 厂商知识就漏到了通用层。
//!
//! 本模块只认文件系统与 MIME，不认识任何服务商。

use super::GenError;
use std::path::{Path, PathBuf};

/// 本地图片输入上限：data URI 会按 4/3 膨胀，读入前先按此上限拦截。
///
/// 这是内存保护阈值，不是厂商约束；厂商自己的更严格限制由协议层按其错误返回。
pub(super) const LOCAL_IMAGE_INPUT_CAP_BYTES: u64 = 32 * 1024 * 1024;

/// 把语义层的图片引用归一化为协议可直接发送的形态：
/// - `http(s)://` / `data:` → 原样返回；
/// - 本地路径（含 `file://` 前缀）→ 读文件转 `data:<mime>;base64,...`；
/// - 其它（既非 URL 也不是可读文件）→ 明确报错，不猜、不静默丢弃。
pub(super) fn image_ref_to_data_uri(source: &str) -> Result<String, GenError> {
    image_ref_to_data_uri_capped(source, LOCAL_IMAGE_INPUT_CAP_BYTES)
}

fn image_ref_to_data_uri_capped(source: &str, cap_bytes: u64) -> Result<String, GenError> {
    let mut value = source.trim();
    if value.is_empty() {
        return Err(GenError::protocol(
            "图片输入为空：请填本地图片路径、公网 URL 或 data URI",
        ));
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") {
        return Ok(value.to_string());
    }
    if lower.starts_with("file://") {
        value = file_url_to_path(value);
    }

    let path = PathBuf::from(value);
    let meta = std::fs::metadata(&path).map_err(|error| {
        GenError::protocol(format!(
            "图片引用既不是 http(s)/data URI，也不是可读的本地文件（{}）：{error}",
            path.display()
        ))
    })?;
    if !meta.is_file() {
        return Err(GenError::protocol(format!(
            "图片引用不是文件（{}）",
            path.display()
        )));
    }
    if meta.len() > cap_bytes {
        return Err(GenError::protocol(format!(
            "本地图片过大（{}，上限 {} 字节）：请先压缩，或改传公网 URL",
            path.display(),
            cap_bytes
        )));
    }
    let mime = image_mime(&path).ok_or_else(|| {
        GenError::protocol(format!(
            "无法从扩展名推断图片格式（{}）：支持 png/jpg/jpeg/webp/gif/bmp",
            path.display()
        ))
    })?;
    let bytes = std::fs::read(&path).map_err(|error| {
        GenError::protocol(format!("读取本地图片失败（{}）：{error}", path.display()))
    })?;
    if bytes.is_empty() {
        return Err(GenError::protocol(format!(
            "本地图片为空（{}）",
            path.display()
        )));
    }
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// `file:///C:/a.png` → `C:/a.png`；`file:///tmp/a.png` → `/tmp/a.png`。
/// 其它 `file://` 形态原样返回（由后续元数据读取给出明确错误）。
fn file_url_to_path(value: &str) -> &str {
    let rest = value.get("file://".len()..).unwrap_or(value);
    let bytes = rest.as_bytes();
    // Windows 盘符形态：/C:/... → C:/...
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        &rest[1..]
    } else {
        rest
    }
}

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuphus_media_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    #[test]
    fn passthrough_remote_and_data_uris() {
        for source in [
            "https://cdn.example.com/a.png",
            "http://cdn.example.com/a.png?x=1",
            "data:image/png;base64,AAAA",
        ] {
            assert_eq!(
                image_ref_to_data_uri(source).expect("passthrough"),
                source,
                "远程/内联引用必须原样透传"
            );
        }
    }

    #[test]
    fn local_image_becomes_data_uri() {
        let path = temp_file("frame.png", b"PNGDATA");
        let uri = image_ref_to_data_uri(&path.display().to_string()).expect("encode local image");
        assert!(uri.starts_with("data:image/png;base64,"), "{uri}");
        use base64::Engine as _;
        let encoded = uri.trim_start_matches("data:image/png;base64,");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("decode"),
            b"PNGDATA"
        );
    }

    #[test]
    fn file_url_prefix_is_accepted() {
        let path = temp_file("frame.jpg", b"JPG");
        let source = format!("file://{}", path.display());
        let uri = image_ref_to_data_uri(&source).expect("file:// local image");
        assert!(uri.starts_with("data:image/jpeg;base64,"), "{uri}");
    }

    #[test]
    fn empty_source_is_rejected() {
        let error = image_ref_to_data_uri("   ").expect_err("empty must fail");
        assert!(error.to_string().contains("图片输入为空"), "{error}");
    }

    #[test]
    fn missing_file_is_reported_with_actionable_hint() {
        let missing = std::env::temp_dir().join("nuphus_media_missing_frame.png");
        let error = image_ref_to_data_uri(&missing.display().to_string()).expect_err("missing");
        let text = error.to_string();
        assert!(text.contains("可读的本地文件"), "{text}");
        assert!(text.contains("http(s)"), "{text}");
    }

    #[test]
    fn unknown_extension_is_rejected() {
        let path = temp_file("frame.txt", b"not an image");
        let error = image_ref_to_data_uri(&path.display().to_string()).expect_err("unknown ext");
        assert!(error.to_string().contains("无法从扩展名推断"), "{error}");
    }

    #[test]
    fn oversized_local_image_is_rejected_before_reading() {
        let path = temp_file("big.png", b"1234567890");
        let error =
            image_ref_to_data_uri_capped(&path.display().to_string(), 4).expect_err("over cap");
        assert!(error.to_string().contains("本地图片过大"), "{error}");
    }
}
