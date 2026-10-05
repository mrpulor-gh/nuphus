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

use sha2::{Digest, Sha256};
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

    // 内容寻址去重：目录内已有逐字节相同的文件则直接复用其路径，不再落新副本。
    // unique_path 只按「同名追加 -1/-2」规避覆盖，于是同一张图在多张主题卡上各设
    // 一次背景（或上传重试）就生成 N 个同字节副本 —— 实测本机 images/ 68 个文件
    // 301.9MB 中仅 20 个唯一内容，202.3MB（67%）是逐字节重复。调用方（主题皮肤 /
    // 头像）只消费返回的路径，复用路径对其完全透明。
    if let Some(existing) = find_identical(&dir, src, size) {
        return Ok(existing.to_string_lossy().into_owned());
    }

    let dest = unique_path(&dir, &sanitize_stem(src), &ext);
    std::fs::copy(src, &dest).map_err(|e| format!("复制图片失败：{}", e))?;

    Ok(dest.to_string_lossy().into_owned())
}

/// 流式计算文件 SHA-256。
///
/// 刻意不整文件读入：入库源有 `MAX_IMAGE_BYTES` 兜底，但目录里的存量文件不受该
/// 上限约束（历史大图 / 后续调整上限都可能超），故一律分块读。
fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

/// 在 `dir` 内寻找与 `src` 逐字节相同的既有文件，命中则返回其路径。
///
/// 先按体积过滤再算哈希：`metadata` 比读文件便宜两个量级，体积不符的候选在读
/// 内容前就被排除，不会为「必然不匹配」的文件付出 IO。极端情况下（目录内大量
/// 同体积文件）会退化为逐个哈希，仍有 `MAX_IMAGE_BYTES` 单文件上限兜底。
fn find_identical(dir: &Path, src: &Path, src_size: u64) -> Option<PathBuf> {
    let src_hash = sha256_file(src).ok()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() != src_size {
            continue;
        }
        if sha256_file(&path).ok() == Some(src_hash) {
            return Some(path);
        }
    }
    None
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
                !p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .map(|e| IMAGE_EXTS.contains(&e.as_str()))
                    .unwrap_or(false),
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
        // 用当前平台的路径分隔符构造，避免把 Windows 风格字面量带到 Linux CI 上
        // （`\` 在非 Windows 不是分隔符，file_stem() 会返回整串）。
        let seps = std::path::MAIN_SEPARATOR;
        assert_eq!(
            sanitize_stem(Path::new(&format!("x{seps}a:b*c?d.jpg"))),
            "abcd"
        );
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
    fn sha256_file_matches_known_digest() {
        // sha256("abc") = ba7816bf...f20015ad（标准测试向量，防哈希实现被改坏）
        let dir = scratch_dir("sha");
        std::fs::write(dir.join("abc.bin"), b"abc").unwrap();

        let expected: [u8; 32] = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(sha256_file(&dir.join("abc.bin")).unwrap(), expected);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_bytes_reuse_the_existing_file() {
        // 同一份字节换个文件名再次入库，必须命中既有文件而不是再落一个副本
        let dir = scratch_dir("dedup-hit");
        let src = scratch_dir("dedup-hit-src");
        std::fs::write(dir.join("wall.png"), b"identical-bytes").unwrap();
        std::fs::write(src.join("another-name.png"), b"identical-bytes").unwrap();

        let found = find_identical(
            &dir,
            &src.join("another-name.png"),
            b"identical-bytes".len() as u64,
        );
        assert_eq!(
            found.as_ref().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("wall.png")),
            "同字节必须复用既有路径"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn same_size_different_bytes_is_not_deduped() {
        // 体积相同但内容不同：体积预过滤放行后，哈希必须能区分（防只比大小的退化）
        let dir = scratch_dir("dedup-samesize");
        let src = scratch_dir("dedup-samesize-src");
        std::fs::write(dir.join("a.png"), b"AAA").unwrap();
        std::fs::write(src.join("b.png"), b"BBB").unwrap();

        assert!(
            find_identical(&dir, &src.join("b.png"), 3).is_none(),
            "同体积不同字节不得误判为重复"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn different_size_is_rejected_before_hashing() {
        // 体积不同即无重复可能；这条守住预过滤不被绕过的回归
        let dir = scratch_dir("dedup-size");
        let src = scratch_dir("dedup-size-src");
        std::fs::write(dir.join("a.png"), b"longer-bytes").unwrap();
        std::fs::write(src.join("b.png"), b"short").unwrap();

        assert!(find_identical(&dir, &src.join("b.png"), 5).is_none());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&src);
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
