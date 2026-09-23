// user_image.rs — 用户图片（皮肤背景 / 头像）落盘命令
//
// 背景：这些图片原先以 dataURL 形式存 localStorage，受浏览器每源约 5MB 配额限制。
// 稍大的图在 `setItem` 时抛 `QuotaExceededError`，而调用侧没有捕获，表现为
// 「选择图片后界面毫无变化」（异常中断了后续的 CSS 变量注入），且同源配额是共享的，
// 一次大图写入失败会连带影响语言、主题等 key 的后续写入。
//
// 本地应用的图片没有理由受浏览器配额约束：这里改为写入数据目录
// `{data_dir}/Nuphus/user-images/{kind}/{uuid}.{ext}`，localStorage 只存
// `{kind}/{uuid}.{ext}` 形式的文件名；展示时由 `read_user_image` 返回磁盘路径，
// 前端 convertFileSrc 转 asset:// URL 交由 WebView 直接读文件渲染（不经 base64）。
// （仅在内存/CSS 中使用，不经过任何配额层）。保存即替换：写新文件、删旧文件。

use std::path::{Path, PathBuf};

/// dataURL 允许的 mime → 扩展名白名单。非图片载荷直接拒绝，防呆。
const ALLOWED_EXTENSIONS: [(&str, &str); 5] = [
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
    ("image/bmp", "bmp"),
];

/// 合法的 kind：目录名与 localStorage key 后缀共用。新用途必须在此登记，
/// 避免任意字符串拼出未预期的目录。
const ALLOWED_KINDS: [&str; 3] = ["skin", "avatar", "nuphus-avatar"];

/// 保存结果：绝对路径（前端经 convertFileSrc 直接加载，无需 base64 中转）+ 文件名（写回 localStorage）。
#[derive(serde::Serialize)]
pub struct SavedUserImage {
    /// localStorage 只存这个短标识；避免绝对路径随用户目录变化而失效。
    pub name: String,
    /// 磁盘绝对路径：前端 convertFileSrc(path) → asset:// URL，由 WebView 原生读文件渲染。
    pub path: String,
}

/// 保存用户图片（dataURL）到 `{kind}/` 目录并替换旧图片。
///
/// - `kind`：`skin` / `avatar` / `nuphus-avatar`
/// - `old_name`：此前保存返回的文件名（旧值可能是历史遗留的 dataURL，调用方此时应传空串；
///   旧 dataURL 本就不在磁盘上，无从删除）
/// - 返回新的 `{kind}/{uuid}.{ext}` 与即时展示 dataURL
#[tauri::command]
pub fn save_user_image(
    kind: String,
    old_name: String,
    data_url: String,
) -> Result<SavedUserImage, String> {
    save_user_image_at(&images_root(), &kind, &old_name, &data_url)
}

#[tauri::command]
pub fn read_user_image(name: String) -> Result<String, String> {
    read_user_image_at(&images_root(), &name)
}

#[tauri::command]
pub fn delete_user_image(name: String) -> Result<(), String> {
    delete_user_image_at(&images_root(), &name)
}

/// 图片根目录：`{data_dir}/Nuphus/user-images`（与模型目录同级组织）。
fn images_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Nuphus")
        .join("user-images")
}

fn checked_kind(kind: &str) -> Result<&'static str, String> {
    ALLOWED_KINDS
        .iter()
        .find(|allowed| **allowed == kind)
        .copied()
        .ok_or_else(|| format!("未知的图片类型：{kind}"))
}

/// 从 dataURL 解析（扩展名, mime）；前缀不在白名单内即拒绝。
fn extension_for(data_url: &str) -> Result<(&'static str, &'static str), String> {
    let head = data_url
        .split(',')
        .next()
        .ok_or_else(|| "图片数据格式不正确".to_string())?;
    let mime = head
        .strip_prefix("data:")
        .and_then(|rest| rest.strip_suffix(";base64"))
        .ok_or_else(|| "图片数据格式不正确：缺少 dataURL 前缀".to_string())?;
    // 返回静态表中的 (&'static str, &'static str)，不用 data_url 的局部切片
    ALLOWED_EXTENSIONS
        .iter()
        .find(|(allowed, _)| *allowed == mime)
        .map(|(allowed_mime, extension)| (*extension, *allowed_mime))
        .ok_or_else(|| format!("不支持的图片格式：{mime}"))
}

/// 解码 dataURL 的 base64 载荷；非法或空内容报错。
fn decode_data_url(data_url: &str, mime: &str) -> Result<Vec<u8>, String> {
    let prefix = format!("data:{mime};base64,");
    let encoded = data_url
        .strip_prefix(&prefix)
        .ok_or_else(|| "图片数据格式不正确".to_string())?;
    let bytes = encode_base64_decode(encoded)?;
    if bytes.is_empty() {
        return Err("图片数据为空".to_string());
    }
    Ok(bytes)
}

fn encode_base64_decode(encoded: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("图片数据解码失败：{}", e))
}

/// 把 `{kind}/{file}` 形式的文件名解析到磁盘路径。
/// 只允许单个 `/` 分隔的两段，且两段都不得为 `.` / `..` / 空 / 含路径符——
/// 任何试图穿越出 `user-images` 根目录的输入都在此拒绝。
fn resolve_image_path(base: &Path, name: &str) -> Result<PathBuf, String> {
    let trimmed = name.trim();
    let mut segments = trimmed.split('/');
    let (Some(kind), Some(file), None) = (segments.next(), segments.next(), segments.next()) else {
        return Err("图片文件名格式不正确".into());
    };
    for segment in [kind, file] {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            return Err("图片文件名格式不正确".into());
        }
    }
    if !ALLOWED_KINDS.contains(&kind) {
        return Err(format!("未知的图片类型：{kind}"));
    }
    let path = base.join(kind).join(file);
    // 归一化后再校验一次前缀，双保险。
    if !path.starts_with(base) {
        return Err("图片文件名格式不正确".into());
    }
    Ok(path)
}

fn remove_image_at(base: &Path, name: &str) {
    if let Ok(path) = resolve_image_path(base, name) {
        let _ = std::fs::remove_file(path);
    }
}

/// 保存逻辑主体：命令与测试共用（测试经此注入 tmp 根目录，不触碰真实数据目录）。
fn save_user_image_at(
    base: &Path,
    kind: &str,
    old_name: &str,
    data_url: &str,
) -> Result<SavedUserImage, String> {
    let kind = checked_kind(kind)?;
    let (extension, mime) = extension_for(data_url)?;
    let bytes = decode_data_url(data_url, mime)?;

    let dir = base.join(kind);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建图片目录失败：{}", e))?;

    let file_stem = uuid::Uuid::new_v4().simple().to_string();
    let file_name = format!("{file_stem}.{extension}");
    let path = dir.join(&file_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入图片失败：{}", e))?;

    // 新文件落盘成功后再删旧文件；旧文件删除失败不影响本次保存（下次保存仍会尝试）。
    if !old_name.trim().is_empty() {
        remove_image_at(base, old_name.trim());
    }

    Ok(SavedUserImage {
        name: format!("{kind}/{file_name}"),
        path: path.to_string_lossy().into_owned(),
    })
}

/// 读回逻辑主体：返回图片在磁盘上的绝对路径（前端 convertFileSrc 直接加载）。
fn read_user_image_at(base: &Path, name: &str) -> Result<String, String> {
    let path = resolve_image_path(base, name)?;
    if !path.is_file() {
        return Err("图片文件不存在".into());
    }
    Ok(path.to_string_lossy().into_owned())
}

/// 删除逻辑主体：文件不存在视为成功（幂等，重复清除不报错）。
fn delete_user_image_at(base: &Path, name: &str) -> Result<(), String> {
    let path = resolve_image_path(base, name)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除图片失败：{}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("nuphus-user-image-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// 1x1 透明 PNG 的 dataURL（最小合法载荷）。
    fn tiny_png_data_url() -> String {
        concat!(
            "data:image/png;base64,",
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
        )
        .to_string()
    }

    #[test]
    fn save_read_delete_round_trip_under_injected_root() {
        let root = test_root("roundtrip");
        let saved = save_user_image_at(&root, "skin", "", &tiny_png_data_url()).unwrap();
        assert!(saved.name.starts_with("skin/"));
        assert!(saved.name.ends_with(".png"));
        assert!(
            std::path::Path::new(&saved.path).is_file(),
            "返回的 path 必须是真实文件：{}",
            saved.path
        );

        // 读回返回同一磁盘路径（前端据此 convertFileSrc 直接加载，不经 base64）
        let read = read_user_image_at(&root, &saved.name).unwrap();
        assert_eq!(read, saved.path, "读回应返回同一文件路径");

        delete_user_image_at(&root, &saved.name).unwrap();
        assert!(
            !std::path::Path::new(&saved.path).exists(),
            "删除后文件必须不存在"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replacing_an_image_removes_the_previous_file() {
        let root = test_root("replace");
        let first = save_user_image_at(&root, "skin", "", &tiny_png_data_url()).unwrap();
        let second = save_user_image_at(&root, "skin", &first.name, &tiny_png_data_url()).unwrap();
        assert_ne!(first.name, second.name, "每次保存必须生成新文件名");
        assert!(
            !resolve_image_path(&root, &first.name).unwrap().exists(),
            "旧文件必须被替换删除"
        );
        assert!(resolve_image_path(&root, &second.name).unwrap().is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_image_payloads_are_rejected_before_touching_the_disk() {
        let root = test_root("reject");
        let result = save_user_image_at(&root, "skin", "", "data:text/html;base64,PGh0bWw+");
        assert!(result.is_err(), "非图片 mime 必须拒绝");
        assert!(matches!(result, Err(error) if error.contains("不支持的图片格式")));
        let no_prefix = save_user_image_at(&root, "skin", "", "https://example.com/a.png");
        assert!(no_prefix.is_err(), "普通 URL 必须拒绝");
        let empty = save_user_image_at(&root, "unknown-kind", "", &tiny_png_data_url());
        assert!(empty.is_err(), "未登记的 kind 必须拒绝");
        assert!(!root.exists(), "拒绝路径不得创建任何目录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn traversal_names_never_escape_the_images_root() {
        let root = test_root("traversal");
        for hostile in [
            "../skin/x.png",
            "skin/../../etc/passwd",
            "skin//x.png",
            "/etc/x.png",
            "skin/x.png/y.png",
            "..\\skin\\x.png",
            "skin/x.png ",
        ] {
            let result = resolve_image_path(&root, hostile);
            if let Ok(path) = &result {
                assert!(
                    path.starts_with(&root),
                    "解析结果必须留在根目录内：{hostile} -> {}",
                    path.display()
                );
                assert_eq!(
                    path.parent().unwrap().parent().unwrap(),
                    &root,
                    "最多下探两级：{hostile}"
                );
            }
        }
        // 合法形态必须放行
        assert!(resolve_image_path(&root, "skin/abc.png").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_missing_file_is_idempotent() {
        let root = test_root("idempotent");
        assert!(delete_user_image_at(&root, "skin/not-exist.png").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 测试直接调用 _at 主体，经注入的 tmp 根目录执行，不触碰真实数据目录。 ──
}
