// user_assets.rs — 用户图片入库
//
// # 为什么需要这个命令（架构原则：本地应用的图片就应该落在本地磁盘上）
//
// 皮肤背景与头像原先走 `<input type=file>` → FileReader → **dataURL** →
// localStorage。那条路有三个结构性毛病：
//   1. localStorage 单 origin 配额约 5MB，一张全屏截图的 base64 就占 2–4MB，
//      存满后**后续任何写入**（含 `nuphus_theme`）都抛 QuotaExceededError，
//      而调用方一律 `catch {}` 静默吞掉 —— 用户看到的是「主题和背景一起丢了」；
//   2. 每次刷新都要把数 MB 的 base64 读进内存常驻；
//   3. dataURL 到底还是把本地文件的字节复制了一份，纯浪费。
//
// 本地应用没有这些理由：图片本来就在本地磁盘，正确做法是
// **把文件复制进应用数据目录，只存路径**，渲染侧用 `convertFileSrc(path)`
// 走 asset:// 读出来。本命令就是这条链的「入库」环节。
//
// # 落点选择（三条约束同时满足）
//
// `nuphus_data_dir()`（`src/utils/mod.rs`，运行时写入的权威根；Windows =
// `%APPDATA%\.nuphus`）下的 `images/` 子目录。这样：
//   - `NUPHUS_DATA_DIR` 环境变量覆盖对它生效，不分裂数据归属；
//   - 天然落在 `tauri.conf.json` 的 assetProtocol scope `$APPDATA/**` 内
//     —— 前端 `convertFileSrc()` 无需任何 CSP/scope 改动即可渲染；
//   - 与 `team.rs` 的 `app_icons_dir()`（外部 Agent 图标本地化）同一思路，
//     根治「用户移动/删除原文件 → 引用失效」。
//
// ⚠️ 皮肤背景是 CSS `background-image` / `<img>`，受主应用 CSP 的 `img-src`
// 约束（tauri.conf.json 的 csp 段**不含** `preview:`）→ 只能走 asset://，
// 不能照抄 PreviewOverlay 的 `convertFileSrc(path, 'preview')`。

use std::path::{Path, PathBuf};

/// 允许入库的图片扩展名（比较前统一转小写）。
/// 与 `dict_ocr.rs` 的读图白名单保持一致，避免出现「能存不能读」的格式。
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// 应用数据目录下的子目录名。
const USER_IMAGE_DIR: &str = "images";

/// 同名序号上限；超过则退化为时间戳文件名（仍不覆盖）。
const DEDUP_LIMIT: u32 = 1000;

/// 文件主名兜底（源文件名为空或全是非法字符时）。
const FALLBACK_STEM: &str = "image";

/// 单张图片体积上限：32 MiB。
///
/// 超限直接拒绝而不是"复制进去再说"：asset:// 读一张超大图时 WebView 解码
/// 可能失败甚至 OOM，表现为背景静默不显示、用户无从判断原因。宁可在这里明确
/// 报错并告知体积，也别把必然失败的产物写进 images/。
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// 把用户选中的本地图片复制进应用数据目录的 `images/` 子目录，
/// 返回写入文件的**绝对路径**（前端只存这个路径，不再存 base64）。
///
/// 同目录内绝不覆盖已有文件：重名时主名追加 `-1` / `-2` … 序号，
/// 与 `canvas_export.rs::unique_path` 的策略一致（用户连续换两次同名背景图，
/// 前一张不该被悄悄替换掉）。
#[tauri::command]
pub fn save_user_image(source_path: String) -> Result<String, String> {
    let src = Path::new(&source_path);
    if !src.is_file() {
        return Err(format!("找不到文件：{}", source_path));
    }

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .ok_or_else(|| "文件没有扩展名，无法识别为图片".to_string())?;
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "不支持的图片格式：.{}（仅支持 {}）",
            ext,
            IMAGE_EXTS.join(" / .")
        ));
    }

    let size = std::fs::metadata(src)
        .map_err(|e| format!("读取文件信息失败：{}", e))?
        .len();
    if size > MAX_IMAGE_BYTES {
        return Err(format!(
            "图片过大：{:.1} MB，上限 {:.0} MB。请先用图片工具缩小再选。",
            size as f64 / 1024.0 / 1024.0,
            MAX_IMAGE_BYTES as f64 / 1024.0 / 1024.0
        ));
    }

    let dir = user_image_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建图片目录失败：{}", e))?;

    let dest = unique_path(&dir, &sanitize_stem(src), &ext);
    std::fs::copy(src, &dest).map_err(|e| format!("复制图片失败：{}", e))?;

    Ok(dest.to_string_lossy().into_owned())
}

/// 图片入库目录：`nuphus_data_dir()/images`。
///
/// 刻意不自己拼 `%APPDATA%` —— 必须复用 `nuphus_data_dir()`，否则
/// `NUPHUS_DATA_DIR` 覆盖用户的图片会与其它功能的数据分裂到两处。
pub fn user_image_dir() -> PathBuf {
    nuphus::utils::nuphus_data_dir().join(USER_IMAGE_DIR)
}

/// 文件名主名净化：剔除 Windows 保留字符与首尾空白/点，空则兜底。
fn sanitize_stem(src: &Path) -> String {
    const ILLEGAL: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let raw = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(FALLBACK_STEM);
    let cleaned: String = raw.chars().filter(|c| !ILLEGAL.contains(c)).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        FALLBACK_STEM.to_string()
    } else {
        cleaned.to_string()
    }
}

/// 目标路径：同名已存在时主名追加 `-1`/`-2`…，保留原扩展名（皮肤图可能是 jpg，
/// 不能像 canvas_export 那样无条件写成 .png）。绝不复用已存在的文件名。
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{}.{}", stem, ext));
    if !first.exists() {
        return first;
    }
    for n in 1..DEDUP_LIMIT {
        let candidate = dir.join(format!("{}-{}.{}", stem, n, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("{}-{}.{}", stem, stamp, ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时目录（带纳秒后缀，测试之间不互相撞；用完即删）。
    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("nuphus-user-img-test-{}-{}", tag, nanos));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    #[test]
    fn rejects_non_image_extension() {
        // 白名单是入库前的唯一防线：格式错了宁可拒绝，也别把任意文件搬进数据目录
        for name in ["a.txt", "a.rs", "a.exe", "a.pdf", "noext"] {
            let p = Path::new(name);
            assert!(
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .map(|e| IMAGE_EXTS.contains(&e.as_str()))
                    .unwrap_or(false)
                    == false,
                "{} 不应被识别为允许的图片",
                name
            );
        }
    }

    #[test]
    fn accepts_declared_image_extensions_case_insensitively() {
        for name in ["a.PNG", "a.jpg", "a.JPEG", "a.gif", "a.WebP", "a.bmp"] {
            let p = Path::new(name);
            let ok = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .map(|e| IMAGE_EXTS.contains(&e.as_str()))
                .unwrap_or(false);
            assert!(ok, "{} 应被接受", name);
        }
    }

    #[test]
    fn sanitize_stem_drops_reserved_chars_and_dots() {
        assert_eq!(sanitize_stem(Path::new(r"C:\x\a:b*c?d.jpg")), "abcd");
        assert_eq!(sanitize_stem(Path::new("  home.. .png")), "home");
        // 主名全是非法字符时不能写出隐藏文件
        assert_eq!(sanitize_stem(Path::new("... .png")), FALLBACK_STEM);
    }

    #[test]
    fn unique_path_never_reuses_an_existing_name() {
        let dir = scratch_dir("unique");
        // 造出首个与 -1，下一个必须落到 -2（而不是覆盖前两者）
        std::fs::write(dir.join("skin.png"), b"1").unwrap();
        std::fs::write(dir.join("skin-1.png"), b"2").unwrap();

        let p = unique_path(&dir, "skin", "png");
        assert_eq!(p.file_name().unwrap(), "skin-2.png");
        assert!(!p.exists(), "返回的路径必须是尚不存在的文件");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_preserves_original_extension() {
        let dir = scratch_dir("ext");
        std::fs::write(dir.join("wall.jpg"), b"1").unwrap();

        let p = unique_path(&dir, "wall", "jpg");
        assert_eq!(p.file_name().unwrap(), "wall-1.jpg");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_image_dir_lives_under_the_runtime_data_root() {
        // 这是 asset scope 成立的前提：落点必须挂在 nuphus_data_dir() 之下，
        // 否则前端 convertFileSrc() 读不到（scope 只放行 $APPDATA/** 等四处）
        let dir = user_image_dir();
        assert_eq!(dir.file_name().unwrap(), USER_IMAGE_DIR);
        assert!(
            dir.starts_with(nuphus::utils::nuphus_data_dir()),
            "图片目录必须位于 nuphus_data_dir() 之下，实际 {:?}",
            dir
        );
    }
}
