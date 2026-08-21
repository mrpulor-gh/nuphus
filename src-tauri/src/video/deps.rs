//! External tool provisioning for the video pipeline (yt-dlp / ffmpeg / ffprobe).
//!
//! Probe chain per tool (first hit wins), mirroring the NUPHUS_*_DIR convention:
//!   1. `NUPHUS_TOOLS_DIR` env override
//!   2. `PATH` (spawn `<exe> --version`)
//!   3. `<data_dir>/Nuphus/tools` (auto-download target)
//!
//! Missing tools are auto-downloaded (Windows only), mirroring
//! speech/download.rs conventions: source fallback, min_size anti-poisoning,
//! download to temp file then rename, progress callback throttled ~1 MiB.
//! Non-Windows / download failure → actionable error with manual-placement
//! instructions (never a hardcoded path assumption).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// External binaries the pipeline depends on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinTool {
    YtDlp,
    Ffmpeg,
    Ffprobe,
}

impl BinTool {
    pub fn exe_name(self) -> &'static str {
        if cfg!(windows) {
            match self {
                BinTool::YtDlp => "yt-dlp.exe",
                BinTool::Ffmpeg => "ffmpeg.exe",
                BinTool::Ffprobe => "ffprobe.exe",
            }
        } else {
            match self {
                BinTool::YtDlp => "yt-dlp",
                BinTool::Ffmpeg => "ffmpeg",
                BinTool::Ffprobe => "ffprobe",
            }
        }
    }

    /// Version-probe argument: ffmpeg/ffprobe take single-dash `-version`
    /// (double-dash prints the banner then exits AVERROR_UNKNOWN), yt-dlp
    /// takes GNU-style `--version`.
    fn version_arg(self) -> &'static str {
        match self {
            BinTool::YtDlp => "--version",
            BinTool::Ffmpeg | BinTool::Ffprobe => "-version",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BinTool::YtDlp => "yt-dlp",
            BinTool::Ffmpeg => "ffmpeg",
            BinTool::Ffprobe => "ffprobe",
        }
    }
}

/// `<data_dir>/Nuphus/tools` — auto-download target & third probe candidate.
pub fn tools_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir().ok_or_else(|| "cannot resolve user data dir".to_string())?;
    Ok(data_dir.join("Nuphus").join("tools"))
}

fn on_path(tool: BinTool) -> bool {
    quiet_command(tool.exe_name())
        .arg(tool.version_arg())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Probe only; returns resolved full path, or the bare exe name when found on
/// PATH (Command resolves it via PATH at spawn time).
pub fn locate(tool: BinTool) -> Option<PathBuf> {
    let exe = tool.exe_name();
    if let Ok(dir) = std::env::var("NUPHUS_TOOLS_DIR") {
        let p = PathBuf::from(dir).join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    if on_path(tool) {
        return Some(PathBuf::from(exe));
    }
    if let Ok(dir) = tools_dir() {
        let p = dir.join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Probe → auto-download if missing. `on_progress(label, downloaded, total)`
/// — total is 0 when the server sends no Content-Length.
pub fn ensure(tool: BinTool, on_progress: &dyn Fn(&str, u64, u64)) -> Result<PathBuf, String> {
    if let Some(p) = locate(tool) {
        return Ok(p);
    }
    download(tool, on_progress).map_err(|e| manual_hint(tool, &e))?;
    locate(tool).ok_or_else(|| format!("{} 下载完成但未能定位", tool.label()))
}

/// ffmpeg + ffprobe ship in the same archive — ensure both with one download.
pub fn ensure_ffmpeg_suite(
    on_progress: &dyn Fn(&str, u64, u64),
) -> Result<(PathBuf, PathBuf), String> {
    let ffmpeg = ensure(BinTool::Ffmpeg, on_progress)?;
    let ffprobe = ensure(BinTool::Ffprobe, on_progress)?;
    Ok((ffmpeg, ffprobe))
}

fn manual_hint(tool: BinTool, err: &str) -> String {
    let dir = tools_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<data_dir>/Nuphus/tools".to_string());
    format!(
        "未找到 {label}，自动下载失败：{err}。可手动下载 {exe} 后通过以下任一方式提供：\
        ① 放入 PATH；② 放置到 {dir}；③ 设置环境变量 NUPHUS_TOOLS_DIR 指向其所在目录",
        label = tool.label(),
        exe = tool.exe_name(),
        dir = dir,
    )
}

// ── Download ────────────────────────────────────────────────────────────

/// yt-dlp single-file binary (GitHub releases; CN mirror first, mirrors
/// speech/download.rs 与 embed.rs 的 CN 优先回退惯例).
const YTDLP_URLS: &[&str] = &[
    "https://ghfast.top/https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
];
/// ffmpeg release archives (must contain bin/ffmpeg.exe + bin/ffprobe.exe).
const FFMPEG_ZIP_URLS: &[&str] = &[
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
    "https://ghfast.top/https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
];

/// yt-dlp.exe is ~10 MB; ffmpeg essentials zip is ~80 MB.
const YTDLP_MIN_SIZE: u64 = 5 * 1024 * 1024;
const FFMPEG_ZIP_MIN_SIZE: u64 = 30 * 1024 * 1024;

fn download(tool: BinTool, on_progress: &dyn Fn(&str, u64, u64)) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("当前平台不支持自动下载".to_string());
    }
    let dir = tools_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建工具目录失败: {e}"))?;
    match tool {
        BinTool::YtDlp => {
            let dest = dir.join(BinTool::YtDlp.exe_name());
            download_first_ok(YTDLP_URLS, &dest, YTDLP_MIN_SIZE, tool.label(), on_progress)
        }
        BinTool::Ffmpeg | BinTool::Ffprobe => {
            let zip_path = dir.join("ffmpeg-download.zip.tmp");
            let result = download_first_ok(
                FFMPEG_ZIP_URLS,
                &zip_path,
                FFMPEG_ZIP_MIN_SIZE,
                "ffmpeg",
                on_progress,
            )
            .and_then(|_| extract_ffmpeg_bins(&zip_path, &dir));
            let _ = std::fs::remove_file(&zip_path);
            result
        }
    }
}

/// Try each URL (3 attempts per source, 1s/2s backoff); first success wins.
fn download_first_ok(
    urls: &[&str],
    dest: &PathBuf,
    min_size: u64,
    label: &str,
    on_progress: &dyn Fn(&str, u64, u64),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP client 初始化失败: {e}"))?;
    let mut last_err = String::new();
    for url in urls {
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_secs(attempt as u64));
            }
            match download_once(&client, url, dest, label, on_progress) {
                Ok(size) if size >= min_size => return Ok(()),
                Ok(size) => {
                    last_err = format!("{url} 返回内容过小（{size} B），疑似错误页");
                    let _ = std::fs::remove_file(dest);
                }
                Err(e) => {
                    last_err = format!("{url}: {e}");
                    let _ = std::fs::remove_file(dest);
                }
            }
        }
    }
    Err(last_err)
}

fn download_once(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &PathBuf,
    label: &str,
    on_progress: &dyn Fn(&str, u64, u64),
) -> Result<u64, String> {
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_report: u64 = 0;
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("读取数据失败: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入文件失败: {e}"))?;
        downloaded += n as u64;
        // Throttle progress to ~1 MiB deltas (mirrors speech/download.rs).
        if downloaded - last_report >= 1024 * 1024 || (total > 0 && downloaded == total) {
            last_report = downloaded;
            on_progress(label, downloaded, total);
        }
    }
    file.flush().map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(downloaded)
}

/// Extract bin/ffmpeg.exe + bin/ffprobe.exe from a release archive.
fn extract_ffmpeg_bins(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;
    let mut extracted = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let target = if name.ends_with("bin/ffmpeg.exe") {
            Some(dest_dir.join(BinTool::Ffmpeg.exe_name()))
        } else if name.ends_with("bin/ffprobe.exe") {
            Some(dest_dir.join(BinTool::Ffprobe.exe_name()))
        } else {
            None
        };
        if let Some(out_path) = target {
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建 {} 失败: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("解压 {} 失败: {e}", name))?;
            extracted += 1;
        }
    }
    if extracted < 2 {
        return Err("压缩包中未找到 bin/ffmpeg.exe 与 bin/ffprobe.exe".to_string());
    }
    Ok(())
}

// ── Command helpers ─────────────────────────────────────────────────────

/// Command with a hidden console window on Windows (GUI app must not flash
/// console windows when spawning CLI tools).
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run a command, capturing output; on non-zero exit return an error with the
/// stderr tail (actionable for the LLM/user).
pub fn run_capture(cmd: &mut Command, what: &str) -> Result<std::process::Output, String> {
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("{what} 启动失败: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "{what} 失败（{:?}）：\n{}",
            out.status.code(),
            tail
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_suite_is_locatable_on_dev_machine_or_error_is_actionable() {
        match locate(BinTool::Ffmpeg) {
            Some(p) => eprintln!("[deps] ffmpeg at {}", p.display()),
            None => {
                let msg = manual_hint(BinTool::Ffmpeg, "simulated failure");
                assert!(msg.contains("NUPHUS_TOOLS_DIR"));
                eprintln!("[deps] ffmpeg missing, hint OK:\n{msg}");
            }
        }
    }
}
