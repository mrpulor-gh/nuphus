//! 拖拽/选择落盘：把浏览器 `File` 的字节内容写入应用缓存目录，换取绝对路径。
//!
//! 背景（单通道重构）：前端统一用 HTML5 `drop` / `<input type=file>` 作为**唯一**
//! 附件通道后，浏览器出于安全屏蔽了 `File.path`——非图片附件拿不到绝对路径，
//! 而引用栏与后端读文件都需要真实路径。此命令负责把字节落盘并回传路径，
//! 使「拖拽的文档」与「历史原生通道拿到的文档」在下游完全等价。
//!
//! 分块协议（大文件支持）：
//! - `begin_dropped_file` 建空文件并返回最终路径（同名不覆盖）；
//! - `append_dropped_file` 按 offset 追加，前端把文件切成 8MB 块流式发送，
//!   内存峰值只与单块有关，不再受单次 IPC 体积约束；
//! - `abort_dropped_file` 中途失败时清掉半截文件，避免缓存目录堆积残件。
//!
//! 设计要点：
//! - 落盘到应用缓存目录 `Nuphus/dropped/`，不污染用户目录、不覆盖既有文件；
//! - 文件名做安全净化（仅取末段、剔控制字符、剥前导点），杜绝路径穿越；
//! - 总大小 2GB、单块 16MB 双上限，防止异常调用打爆磁盘/内存；
//! - 逾期附件按缓存语义回收（30 天未改动），防止大文件长期堆积。

use base64::Engine;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 单个附件总大小上限（2 GiB）。附件只做「路径引用」，由 Agent 用工具按需读取。
const MAX_DROPPED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// 单块原始字节上限（16 MiB）：前端按 8 MiB 切块，这里留一倍余量。
const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
/// 落盘附件保留天数：超过即按缓存回收，避免大文件永久占盘。
const RETENTION_DAYS: u64 = 30;
/// 每次进程只清扫一次（拖拽是高频入口，不做重复扫描）。
static CLEANED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 落盘目录：`NUPHUS_DROP_DIR` > `NUPHUS_DATA_DIR/dropped` > `<data_dir>/Nuphus/dropped`
fn dropped_dir() -> PathBuf {
    std::env::var("NUPHUS_DROP_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var("NUPHUS_DATA_DIR")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::data_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join("Nuphus")
                });
            base.join("dropped")
        })
}

/// 净化文件名：剔除路径分隔符与控制字符，仅保留安全字符；为空则回退 `file`。
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

/// `sanitize_name` 是幂等的，所以「回传名字必须等于净化结果」可用于拒绝注入串。
fn safe_leaf_name(name: &str) -> Result<String, String> {
    let safe = sanitize_name(name);
    if safe == name {
        Ok(safe)
    } else {
        Err("非法文件名".to_string())
    }
}

/// 把（前端回传的）文件名解析成落盘路径；名字已是叶子名，join 不会逃出目录。
fn resolve_drop_target(name: &str) -> Result<PathBuf, String> {
    let safe = safe_leaf_name(name)?;
    Ok(dropped_dir().join(safe))
}

/// 同名不覆盖：冲突时加时间戳，仍冲突再加序号。
fn unique_target(dir: &Path, safe_name: &str) -> PathBuf {
    let base = dir.join(safe_name);
    if !base.exists() {
        return base;
    }
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    for seq in 0..1000u32 {
        let candidate = if seq == 0 {
            dir.join(format!("{stem}-{stamp}{ext}"))
        } else {
            dir.join(format!("{stem}-{stamp}-{seq}{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{stamp}-{}{ext}", std::process::id()))
}

/// 缓存回收：删除 `RETENTION_DAYS` 天未改动的落盘附件。
/// 只扫前 512 个条目，大目录也不会拖慢入口；进程内只跑一次。
fn cleanup_stale_drops(dir: &Path) {
    if CLEANED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(RETENTION_DAYS * 24 * 3600);
    for entry in entries.flatten().take(512) {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        if let Ok(modified) = meta.modified() {
            if modified < cutoff {
                std::fs::remove_file(entry.path()).ok();
            }
        }
    }
}

/// 建空文件，返回最终 `{ path, name }`；后续分块按 `name` 追加。
#[tauri::command]
pub async fn begin_dropped_file(file_name: String) -> Result<Value, String> {
    let dir = dropped_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建落盘目录失败: {e}"))?;
    cleanup_stale_drops(&dir);

    let safe_name = sanitize_name(&file_name);
    // `create_new` 保证并发同名也不会互相截断：冲突就重新取候选名
    let mut created: Option<PathBuf> = None;
    for _ in 0..8 {
        let candidate = unique_target(&dir, &safe_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => {
                created = Some(candidate);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("创建文件失败: {e}")),
        }
    }
    let target = created.ok_or_else(|| "创建文件失败：同名冲突过多".to_string())?;

    let final_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&safe_name)
        .to_string();
    Ok(json!({
        "path": target.to_string_lossy(),
        "name": final_name,
    }))
}

/// 按 offset 追加一块 base64；offset 必须等于当前文件长度（拒绝乱序/重放）。
#[tauri::command]
pub async fn append_dropped_file(
    file_name: String,
    chunk_base64: String,
    offset: u64,
) -> Result<Value, String> {
    // base64 体积 ≈ 原始字节 * 4/3，先按长度粗筛，避免解码超大串
    if chunk_base64.len() > MAX_CHUNK_BYTES * 4 / 3 + 16 {
        return Err("分块超过 16MB 上限".to_string());
    }
    let target = resolve_drop_target(&file_name)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(chunk_base64.trim())
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    if bytes.is_empty() {
        return Err("分块内容为空".to_string());
    }
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err("分块超过 16MB 上限".to_string());
    }

    let current = std::fs::metadata(&target)
        .map_err(|e| format!("附件不存在或已被清理: {e}"))?
        .len();
    if current != offset {
        return Err(format!(
            "分块顺序错乱（当前已写入 {current} 字节，收到 offset {offset}）"
        ));
    }
    if current + bytes.len() as u64 > MAX_DROPPED_BYTES {
        return Err(format!(
            "文件超过 {}GB 上限",
            MAX_DROPPED_BYTES / 1024 / 1024 / 1024
        ));
    }

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&target)
        .map_err(|e| format!("打开文件失败: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(json!({ "written": current + bytes.len() as u64 }))
}

/// 中途失败时清掉半截文件；文件已不存在视为成功（幂等）。
#[tauri::command]
pub async fn abort_dropped_file(file_name: String) -> Result<(), String> {
    let target = resolve_drop_target(&file_name)?;
    match std::fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("清理半截文件失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 环境变量是进程级状态，测试必须串行，否则互相踩 `NUPHUS_DROP_DIR`。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn use_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuphus_drop_{}_{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::env::set_var("NUPHUS_DROP_DIR", &dir);
        dir
    }

    fn cleanup(dir: &Path) {
        std::fs::remove_dir_all(dir).ok();
        std::env::remove_var("NUPHUS_DROP_DIR");
    }

    /// 文件名净化：只取末段，剔除控制字符与前导点，防路径穿越。
    #[test]
    fn sanitize_name_strips_paths_and_control_chars() {
        assert_eq!(sanitize_name("a.txt"), "a.txt");
        // Windows 路径与 POSIX 路径都只取末段
        assert_eq!(sanitize_name(r"C:\Users\me\secret.txt"), "secret.txt");
        assert_eq!(sanitize_name("/home/me/secret.txt"), "secret.txt");
        // 经典穿越串：取末段后已无分隔符
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        // 控制字符剔除
        assert_eq!(sanitize_name("a\u{0}b.txt"), "ab.txt");
        // 前导点（隐藏文件语义）被剥掉，避免落盘成 `.` / `..`
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name(".."), "file");
        // 空名回退
        assert_eq!(sanitize_name(""), "file");
    }

    /// 净化必须幂等：`append` 用「回传名 == 净化结果」判定注入。
    #[test]
    fn sanitize_name_is_idempotent() {
        for raw in [
            "a.txt",
            "../x.txt",
            ".hidden",
            "..",
            "",
            "a\u{0}b",
            " 空格.txt ",
        ] {
            let once = sanitize_name(raw);
            assert_eq!(sanitize_name(&once), once, "raw={raw:?}");
        }
    }

    /// 分块往返：两块拼起来与原始字节一致，路径真实可读。
    #[tokio::test]
    async fn begin_plus_append_round_trip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("roundtrip");

        let started = begin_dropped_file("note.txt".into())
            .await
            .expect("应建文件成功");
        let name = started["name"].as_str().expect("应返回 name").to_string();
        let path = started["path"].as_str().expect("应返回 path").to_string();

        let payload = b"hello dropped file";
        let (first, second) = payload.split_at(8);
        let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);

        append_dropped_file(name.clone(), enc(first), 0)
            .await
            .expect("首块应写入");
        let out = append_dropped_file(name.clone(), enc(second), first.len() as u64)
            .await
            .expect("次块应写入");
        assert_eq!(out["written"].as_u64(), Some(payload.len() as u64));
        assert_eq!(std::fs::read(&path).expect("路径应可读"), payload);

        cleanup(&dir);
    }

    /// 空文件：只 begin 不 append 也应得到可读的 0 字节文件。
    #[tokio::test]
    async fn begin_only_yields_empty_file() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("empty");

        let started = begin_dropped_file("empty.txt".into())
            .await
            .expect("应建文件成功");
        let path = started["path"].as_str().unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);

        cleanup(&dir);
    }

    /// 乱序 offset 必须拒绝，避免覆盖或错位拼接。
    #[tokio::test]
    async fn rejects_misordered_offset() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("offset");

        let started = begin_dropped_file("a.bin".into()).await.unwrap();
        let name = started["name"].as_str().unwrap().to_string();
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"abcd");
        append_dropped_file(name.clone(), b64.clone(), 0)
            .await
            .unwrap();
        let err = append_dropped_file(name, b64, 0)
            .await
            .expect_err("重复 offset 应报错");
        assert!(err.contains("顺序错乱"), "错误应指向 offset: {err}");

        cleanup(&dir);
    }

    /// 非法文件名（含路径）必须拒绝，防止越权写盘。
    #[tokio::test]
    async fn rejects_path_like_name() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("traversal");

        let err = append_dropped_file(
            r"..\..\evil.txt".into(),
            base64::engine::general_purpose::STANDARD.encode(b"x"),
            0,
        )
        .await
        .expect_err("路径名应被拒绝");
        assert!(err.contains("非法文件名"), "错误应指向文件名: {err}");

        cleanup(&dir);
    }

    /// 非法 base64 必须报错而非写出半个文件。
    #[tokio::test]
    async fn rejects_invalid_base64() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("badb64");

        let started = begin_dropped_file("x.txt".into()).await.unwrap();
        let name = started["name"].as_str().unwrap().to_string();
        let err = append_dropped_file(name, "!!!not base64!!!".into(), 0)
            .await
            .expect_err("非法 base64 应报错");
        assert!(err.contains("base64"), "错误信息应指向解码失败: {err}");

        cleanup(&dir);
    }

    /// abort 删除半截文件，且对不存在的文件幂等。
    #[tokio::test]
    async fn abort_removes_partial_file() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("abort");

        let started = begin_dropped_file("partial.txt".into()).await.unwrap();
        let name = started["name"].as_str().unwrap().to_string();
        let path = started["path"].as_str().unwrap().to_string();
        abort_dropped_file(name.clone()).await.expect("应删除成功");
        assert!(!std::path::Path::new(&path).exists());
        // 再删一次不报错
        abort_dropped_file(name).await.expect("重复 abort 应幂等");

        cleanup(&dir);
    }

    /// 同名文件不覆盖：第二次 begin 得到不同路径。
    #[tokio::test]
    async fn begin_does_not_overwrite() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = use_temp_dir("unique");

        let first = begin_dropped_file("same.txt".into()).await.unwrap();
        let second = begin_dropped_file("same.txt".into()).await.unwrap();
        assert_ne!(
            first["path"].as_str().unwrap(),
            second["path"].as_str().unwrap()
        );

        cleanup(&dir);
    }
}
