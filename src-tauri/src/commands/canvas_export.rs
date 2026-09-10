// canvas_export.rs — 画布导出落盘命令
// save_prototype_png：把 UI 原型画布的 PNG data URL 解码后写入「保存目录」下的
// 「Nuphus 原型图」子目录（候选顺序与可写性探测见 base_dir_candidates / resolve_base_dir），
// 返回写入文件的绝对路径。前端据此明确告知用户文件落在哪里，替代 WebView 的
// 静默 <a download>（点了不知道存到哪儿）。

use std::path::{Path, PathBuf};

/// data URL 前缀：前端 toPng 固定输出 PNG，只接受这一种
const PNG_DATA_URL_PREFIX: &str = "data:image/png;base64,";
/// 下载目录下的子目录名（不存在时创建）
const EXPORT_DIR_NAME: &str = "Nuphus 原型图";
/// Windows 文件名保留字符：统一剔除，避免写出非法文件名
const ILLEGAL_NAME_CHARS: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
/// 文件主名兜底（屏幕未命名时）
const FALLBACK_STEM: &str = "screen";
/// 同名序号上限；超过则退化为时间戳文件名（仍不覆盖）
const DEDUP_LIMIT: u32 = 1000;

/// 保存 UI 原型画布 PNG（data URL），返回写入文件的绝对路径。
/// 目录为下载目录下的「Nuphus 原型图」子目录；重名自动追加 -1/-2… 序号，绝不覆盖。
#[tauri::command]
pub fn save_prototype_png(data_url: String, file_name: String) -> Result<String, String> {
    let bytes = decode_png_data_url(&data_url)?;

    let dir = export_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建保存目录失败：{}", e))?;
    let path = unique_path(&dir, &sanitize_stem(&file_name));
    std::fs::write(&path, &bytes).map_err(|e| format!("写入图片失败：{}", e))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 解析 PNG data URL 并解码；前缀不符、base64 非法或内容为空时报错。
fn decode_png_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let encoded = data_url
        .strip_prefix(PNG_DATA_URL_PREFIX)
        .ok_or_else(|| "图片数据格式不正确：缺少 PNG data URL 前缀".to_string())?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("图片数据解码失败：{}", e))?;
    if bytes.is_empty() {
        return Err("图片数据为空".to_string());
    }
    Ok(bytes)
}

/// 保存目录：候选基目录（见 base_dir_candidates）中第一个可写者，下的「Nuphus 原型图」子目录。
fn export_dir() -> PathBuf {
    resolve_base_dir(base_dir_candidates()).join(EXPORT_DIR_NAME)
}

/// 保存目录候选，按优先级逐个探测「可创建且可写」，第一个通过者胜出：
///   1. `dirs::download_dir()`（系统下载目录）
///   2. 注册表 User Shell Folders 的 Downloads 值（Windows：下载目录被重定向时的权威来源）
///   3. `%USERPROFILE%\Downloads`
///   4. `dirs::picture_dir()`
///   5. `%USERPROFILE%\Pictures`
///   6. `%USERPROFILE%\Desktop`
///   7. 系统临时目录（最后兜底）
///
/// 必须逐个探测而不能直接采纳 `dirs::download_dir()`：下载目录被重定向到不存在的盘符时
/// 它仍可能返回该路径，直接写入只会失败或把文件静默丢到 Temp（用户找不到）。
fn base_dir_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = dirs::download_dir() {
        out.push(p);
    }
    #[cfg(target_os = "windows")]
    if let Some(p) = windows_shell_folder_downloads() {
        out.push(p);
    }
    if let Some(p) = home_join("Downloads") {
        out.push(p);
    }
    if let Some(p) = dirs::picture_dir() {
        out.push(p);
    }
    if let Some(p) = home_join("Pictures") {
        out.push(p);
    }
    if let Some(p) = home_join("Desktop") {
        out.push(p);
    }
    out.push(std::env::temp_dir());
    out
}

/// 从候选里取第一个「可创建且可写」的目录；全部不可用时回落系统临时目录。
/// 每次降级都留日志（禁止静默降级），便于排查「文件到底写到哪儿了」。
fn resolve_base_dir<I: IntoIterator<Item = PathBuf>>(candidates: I) -> PathBuf {
    for dir in candidates {
        if dir_is_usable(&dir) {
            return dir;
        }
        tracing::warn!(
            "[canvas_export] 候选保存目录不可用，降级到下一候选：{}",
            dir.display()
        );
    }
    let fallback = std::env::temp_dir();
    tracing::warn!(
        "[canvas_export] 候选保存目录全部不可用，回落系统临时目录：{}",
        fallback.display()
    );
    fallback
}

/// 目录可用性探测：能创建（已存在则复用）且真正写得出文件。
/// 只 create_dir_all 不够——目录可能只读或受权限约束，只有写入才能暴露真相。
fn dir_is_usable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".nuphus-write-probe-{}", std::process::id()));
    let writable = std::fs::write(&probe, b"").is_ok();
    if writable {
        let _ = std::fs::remove_file(&probe);
    }
    writable
}

/// 用户主目录下的子目录（Windows 读 USERPROFILE，其它平台读 HOME）。
fn home_join(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME");
    home.filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(name))
}

/// 注册表 `User Shell Folders` 里的 Downloads 值（值名为已知文件夹 GUID，Windows 固定）。
/// `RegGetValueW` 会自动展开 REG_EXPAND_SZ 中的 `%USERPROFILE%` 等变量；读不到即 None，
/// 交由下一个候选继续，不影响主流程。
#[cfg(target_os = "windows")]
fn windows_shell_folder_downloads() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    /// User Shell Folders 子键（下载目录被重定向时写在这里）
    const SUBKEY: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
    /// Downloads 的已知文件夹 GUID（值名，系统固定）
    const VALUE: &str = "{374DE290-123F-4565-9164-39C4925E467B}";
    /// 路径缓冲上限：注册表里的路径远短于此，留足余量避免截断
    const BUF_BYTES: u32 = 8 * 1024;

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let subkey = wide(SUBKEY);
    let value = wide(VALUE);
    let mut buf = vec![0u8; BUF_BYTES as usize];
    let mut len = BUF_BYTES;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast::<std::ffi::c_void>(),
            &mut len,
        )
    };
    // status 非 0：键值不存在或不可读；len < 2：只有 NUL，视为空
    if status != 0 || len < 2 {
        return None;
    }
    // 写回的是 UTF-16 字符串（含结尾 NUL），按字节长度取有效区段
    let units: Vec<u16> = buf[..len as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let path = String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// 文件名净化：剥离 .png 后缀、剔除保留字符与首尾空白/点，
/// 结果为空时兜底 FALLBACK_STEM（避免写出 ".png" 这类隐藏文件）。
fn sanitize_stem(file_name: &str) -> String {
    let raw = file_name.trim();
    let stem = raw
        .strip_suffix(".png")
        .or_else(|| raw.strip_suffix(".PNG"))
        .unwrap_or(raw);
    let cleaned: String = stem
        .chars()
        .filter(|c| !ILLEGAL_NAME_CHARS.contains(c))
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        FALLBACK_STEM.to_string()
    } else {
        cleaned.to_string()
    }
}

/// 目标路径：同名文件已存在时追加 -1/-2… 序号（同目录内绝不复用文件名）。
fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{}.png", stem));
    if !first.exists() {
        return first;
    }
    for n in 1..DEDUP_LIMIT {
        let candidate = dir.join(format!("{}-{}.png", stem, n));
        if !candidate.exists() {
            return candidate;
        }
    }
    // 极端情况（同名文件超过上限）退化为时间戳文件名，仍不覆盖
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("{}-{}.png", stem, stamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4e, 0x47];

    #[test]
    fn decodes_png_data_url() {
        use base64::Engine as _;
        let url = format!(
            "{}{}",
            PNG_DATA_URL_PREFIX,
            base64::engine::general_purpose::STANDARD.encode(PNG_MAGIC)
        );
        assert_eq!(decode_png_data_url(&url).unwrap(), PNG_MAGIC.to_vec());
    }

    #[test]
    fn rejects_non_png_or_broken_payload() {
        assert!(decode_png_data_url("data:image/jpeg;base64,AAAA").is_err());
        assert!(decode_png_data_url("data:image/png;base64,@@@").is_err());
        assert!(decode_png_data_url("data:image/png;base64,").is_err());
    }

    #[test]
    fn sanitize_stem_drops_reserved_chars_and_suffix() {
        assert_eq!(sanitize_stem("a\\b/c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(sanitize_stem("screen 1.png"), "screen 1");
        assert_eq!(sanitize_stem("  home.. "), "home");
        assert_eq!(sanitize_stem("   "), FALLBACK_STEM);
        assert_eq!(sanitize_stem("../.."), FALLBACK_STEM);
    }

    #[test]
    fn unique_path_appends_suffix_instead_of_overwriting() {
        let dir = std::env::temp_dir().join(format!("nuphus-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "shot"), dir.join("shot.png"));
        std::fs::write(dir.join("shot.png"), PNG_MAGIC).unwrap();
        assert_eq!(unique_path(&dir, "shot"), dir.join("shot-1.png"));
        std::fs::write(dir.join("shot-1.png"), PNG_MAGIC).unwrap();
        assert_eq!(unique_path(&dir, "shot"), dir.join("shot-2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 测试用工作目录：每个用例独立命名，结束即清理
    fn test_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("nuphus-export-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 造一个「当目录用必然失败」的路径：拿一个已存在的文件冒充候选目录，
    /// 对应下载目录被重定向到失联位置（创建失败）的真实场景。
    fn blocked_candidate(root: &Path, name: &str) -> PathBuf {
        let file = root.join(name);
        std::fs::write(&file, PNG_MAGIC).unwrap();
        file
    }

    #[test]
    fn resolve_base_dir_skips_unusable_candidate_and_takes_the_next() {
        let root = test_root("skip");
        let blocked = blocked_candidate(&root, "blocked");
        let usable = root.join("ok");
        assert_eq!(resolve_base_dir(vec![blocked, usable.clone()]), usable);
        assert!(usable.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_base_dir_falls_back_to_temp_when_all_candidates_fail() {
        let root = test_root("none");
        let blocked = blocked_candidate(&root, "blocked");
        // 子目录挂在文件下面：创建同样失败，确保整条候选链都不可用
        let nested = blocked.join("sub");
        assert_eq!(
            resolve_base_dir(vec![blocked, nested]),
            std::env::temp_dir()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_base_dir_prefers_the_first_writable_candidate() {
        let root = test_root("first");
        let first = root.join("first");
        let second = root.join("second");
        assert_eq!(resolve_base_dir(vec![first.clone(), second]), first);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 候选链覆盖需求里的降级路径：download_dir 缺失时仍能落到 USERPROFILE 下的
    /// 可见目录（本机即 `<user>\Downloads`），而不是直接掉到 Temp。
    #[test]
    fn candidates_keep_a_visible_home_fallback_before_temp() {
        let candidates = base_dir_candidates();
        assert_eq!(candidates.last(), Some(&std::env::temp_dir()));
        #[cfg(target_os = "windows")]
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let downloads = PathBuf::from(home).join("Downloads");
            let temp = std::env::temp_dir();
            let downloads_at = candidates.iter().position(|c| *c == downloads);
            let temp_at = candidates.iter().position(|c| *c == temp);
            assert!(
                downloads_at.is_some(),
                "候选链应包含 %USERPROFILE%\\Downloads"
            );
            assert!(downloads_at < temp_at, "Downloads 必须排在 Temp 之前");
        }
    }

    /// 落盘目录保持既有命名：候选基目录 + 「Nuphus 原型图」子目录
    #[test]
    fn export_dir_keeps_the_prototype_subfolder_name() {
        let dir = export_dir();
        assert_eq!(dir.file_name(), Some(std::ffi::OsStr::new(EXPORT_DIR_NAME)));
    }
}
