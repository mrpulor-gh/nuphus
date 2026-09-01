// tools/video.rs — 视频内部机制命令（工具页手动调用，非 agent 工具）
//
// 移植自 nuphus-tools-rs engines/video.rs，修复已知缺陷：
// - 硬编码 "ffmpeg" → crate::video::deps::ensure_ffmpeg_suite 定位（探测链 +
//   自动下载 + NUPHUS_TOOLS_DIR 可操作错误提示），绝不硬编码路径
// - to_str().unwrap() panic → to_string_lossy + 错误传播
// - stderr 尾部拼接 → 复用 deps::run_capture 的 stderr tail 提取
// - video_info 从行解析改 ffprobe JSON（结构化、抗格式变化）
//
// 所有命令 async + spawn_blocking：ffmpeg 耗时，不在 UI 线程执行。

use std::path::{Path, PathBuf};

use crate::video::deps::{ensure_ffmpeg_suite, quiet_command, run_capture};

/// 输入文件大小上限（10GB：视频压缩/抽帧是大文件场景，不设过低）
const VIDEO_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;

fn ensure_input_video(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_file() {
        return Err(format!("视频文件不存在：{}", path));
    }
    let meta = std::fs::metadata(&p).map_err(|e| format!("读取视频信息失败：{}", e))?;
    if meta.len() > VIDEO_MAX_BYTES {
        return Err("文件超过 10GB 上限".to_string());
    }
    Ok(p)
}

fn ensure_output_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{}", e))?;
        }
    }
    Ok(())
}

/// 压缩视频（libx264 重编码）。quality: low / medium / high
#[tauri::command]
pub async fn video_compress(
    input_path: String,
    output_path: String,
    quality: Option<String>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let output = PathBuf::from(&output_path);
    ensure_output_dir(&output)?;

    let (video_bitrate, audio_bitrate) = match quality.as_deref().unwrap_or("medium") {
        "low" => ("1M", "64k"),
        "high" => ("4M", "128k"),
        _ => ("2M", "96k"),
    };

    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let output_arg = output.to_string_lossy().into_owned();
    let size_before = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);

    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffmpeg);
        cmd.args([
            "-hide_banner",
            "-i",
            &input_arg,
            "-c:v",
            "libx264",
            "-b:v",
            video_bitrate,
            "-c:a",
            "aac",
            "-b:a",
            audio_bitrate,
            "-y",
            &output_arg,
        ]);
        run_capture(&mut cmd, "视频压缩")
    })
    .await
    .map_err(|e| format!("video_compress task failed: {e}"))??;
    let size_after = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "quality": quality.unwrap_or_else(|| "medium".to_string()),
        "size_before": size_before,
        "size_after": size_after,
        "saved_bytes": size_before.saturating_sub(size_after),
    }))
}

/// 提取音频。format: mp3（默认）/ wav
#[tauri::command]
pub async fn video_extract_audio(
    input_path: String,
    output_path: String,
    format: Option<String>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let output = PathBuf::from(&output_path);
    ensure_output_dir(&output)?;

    let fmt = format.unwrap_or_else(|| "mp3".to_string());
    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let output_arg = output.to_string_lossy().into_owned();
    let fmt_inner = fmt.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffmpeg);
        cmd.arg("-hide_banner").arg("-i").arg(&input_arg);
        match fmt_inner.as_str() {
            "wav" => {
                cmd.args(["-vn", "-acodec", "pcm_s16le", "-ar", "44100", "-ac", "2"]);
            }
            _ => {
                cmd.args(["-vn", "-acodec", "libmp3lame", "-ab", "192k"]);
            }
        }
        cmd.arg("-y").arg(&output_arg);
        run_capture(&mut cmd, "音频提取")
    })
    .await
    .map_err(|e| format!("video_extract_audio task failed: {e}"))??;

    let size_after = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "format": fmt,
        "size_after": size_after,
    }))
}

/// 抽帧：每 interval_secs 秒抽取一帧，输出到目录 frame_0001.jpg ...
#[tauri::command]
pub async fn video_extract_frames(
    input_path: String,
    output_dir: String,
    interval_secs: Option<f64>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let dir = PathBuf::from(&output_dir);
    if dir.is_file() {
        return Err("输出路径是文件，应为目录".to_string());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建输出目录失败：{}", e))?;

    let interval = interval_secs.unwrap_or(1.0);
    if !(interval > 0.0 && interval <= 3600.0) {
        return Err("抽帧间隔需在 0.001~3600 秒之间".to_string());
    }

    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let dir_arg = dir.to_string_lossy().into_owned();
    let fps_filter = format!("fps=1/{}", interval);

    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffmpeg);
        cmd.args([
            "-hide_banner",
            "-i",
            &input_arg,
            "-vf",
            &fps_filter,
            "-q:v",
            "2",
            "-y",
        ]);
        let pattern = format!("{}/frame_%04d.jpg", dir_arg);
        cmd.arg(pattern);
        run_capture(&mut cmd, "视频抽帧")
    })
    .await
    .map_err(|e| format!("video_extract_frames task failed: {e}"))??;

    let files = std::fs::read_dir(&dir)
        .map_err(|e| format!("读取输出目录失败：{}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .count();
    Ok(serde_json::json!({
        "output_dir": dir.to_string_lossy(),
        "frames": files,
        "interval_secs": interval,
    }))
}

/// 获取视频信息（ffprobe JSON 解析：时长 / 编码 / 分辨率 / 音频编码）
#[tauri::command]
pub async fn video_info(path: String) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&path)?;
    let (_ffmpeg, ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();

    let output = tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffprobe);
        cmd.args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            &input_arg,
        ]);
        run_capture(&mut cmd, "视频信息探测")
    })
    .await
    .map_err(|e| format!("video_info task failed: {e}"))??;

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("解析 ffprobe 输出失败：{}", e))?;

    let mut video_codec = serde_json::Value::Null;
    let mut audio_codec = serde_json::Value::Null;
    let mut width = serde_json::Value::Null;
    let mut height = serde_json::Value::Null;
    if let Some(streams) = json.get("streams").and_then(|s| s.as_array()) {
        for s in streams {
            let codec_type = s.get("codec_type").and_then(|v| v.as_str()).unwrap_or("");
            match codec_type {
                "video" => {
                    video_codec = s
                        .get("codec_name")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    width = s.get("width").cloned().unwrap_or(serde_json::Value::Null);
                    height = s.get("height").cloned().unwrap_or(serde_json::Value::Null);
                }
                "audio" => {
                    audio_codec = s
                        .get("codec_name")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                }
                _ => {}
            }
        }
    }
    let duration = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let size = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);

    Ok(serde_json::json!({
        "path": input.to_string_lossy(),
        "duration_seconds": duration,
        "video_codec": video_codec,
        "audio_codec": audio_codec,
        "width": width,
        "height": height,
        "size_bytes": size,
    }))
}
/// 视频转 GIF：ffmpeg palettegen + paletteuse 两段式（色彩保留，避免灰蒙蒙）。
/// fps 默认 10（1-30）；scale 为输出宽度（默认 480，16-2000），高度等比。
#[tauri::command]
pub async fn video_to_gif(
    input_path: String,
    output_path: String,
    fps: Option<f64>,
    scale: Option<u32>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let output = PathBuf::from(&output_path);
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "gif" {
        return Err("输出扩展名必须是 .gif".to_string());
    }
    ensure_output_dir(&output)?;

    let fps = fps.unwrap_or(10.0).clamp(1.0, 30.0);
    let scale_w = scale.unwrap_or(480).clamp(16, 2000);
    let vf = format!("fps={},scale={}:-1:flags=lanczos", fps, scale_w);

    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let output_arg = output.to_string_lossy().into_owned();
    // 调色板临时文件：输出同目录同名 .palette.png（完成后删除）
    let palette = output.with_extension("palette.png");
    let palette_arg = palette.to_string_lossy().into_owned();
    let size_before = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);

    tauri::async_runtime::spawn_blocking(move || {
        // ① 生成调色板
        let mut cmd1 = quiet_command(ffmpeg.clone());
        cmd1.args([
            "-hide_banner",
            "-i",
            &input_arg,
            "-vf",
            &format!("{},palettegen", vf),
            "-y",
            &palette_arg,
        ]);
        run_capture(&mut cmd1, "视频转GIF（调色板）")?;
        // ② 应用调色板输出 GIF
        let mut cmd2 = quiet_command(ffmpeg);
        cmd2.args([
            "-hide_banner",
            "-i",
            &input_arg,
            "-i",
            &palette_arg,
            "-lavfi",
            &format!("{}[x];[x][1:v]paletteuse", vf),
            "-y",
            &output_arg,
        ]);
        run_capture(&mut cmd2, "视频转GIF")
    })
    .await
    .map_err(|e| format!("video_to_gif task failed: {e}"))??;

    let _ = std::fs::remove_file(&palette);
    let size_after = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "size_before": size_before,
        "size_after": size_after,
        "fps": fps,
        "scale": scale_w,
    }))
}

/// 视频截取片段：ffmpeg -ss（快速 seek）+ -c copy（流复制，不重编码）。
/// start_sec 缺省 0；end_sec 缺省到末尾；超过时长 ffmpeg 自动截到末尾。
#[tauri::command]
pub async fn video_cut(
    input_path: String,
    output_path: String,
    start_sec: Option<f64>,
    end_sec: Option<f64>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let output = PathBuf::from(&output_path);
    ensure_output_dir(&output)?;

    let start = start_sec.unwrap_or(0.0);
    if !(start >= 0.0 && start <= 86400.0) {
        return Err("开始时间需在 0~86400 秒之间".to_string());
    }
    if let Some(end) = end_sec {
        if end <= start {
            return Err("结束时间需大于开始时间".to_string());
        }
    }

    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let output_arg = output.to_string_lossy().into_owned();
    let start_arg = format!("{}", start);
    let end_arg = end_sec.map(|e| format!("{}", e));

    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffmpeg);
        // -ss 置于 -i 前 = 快速 seek；-c copy = 流复制不重编码
        cmd.args([
            "-hide_banner",
            "-ss",
            &start_arg,
            "-i",
            &input_arg,
            "-c",
            "copy",
        ]);
        if let Some(e) = &end_arg {
            cmd.args(["-to", e]);
        }
        cmd.args(["-y", &output_arg]);
        run_capture(&mut cmd, "视频截取")
    })
    .await
    .map_err(|e| format!("video_cut task failed: {e}"))??;

    let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "start_sec": start,
        "end_sec": end_sec,
        "size_bytes": size,
    }))
}

/// 音频格式转换：输出扩展名决定编码（mp3=libmp3lame / wav=pcm_s16le / m4a=aac / flac=flac）。
/// 输入可为音频或视频（提取音轨）。bitrate 可选（如 "192k"，mp3/m4a 生效）。
#[tauri::command]
pub async fn audio_convert(
    input_path: String,
    output_path: String,
    bitrate: Option<String>,
) -> Result<serde_json::Value, String> {
    let input = ensure_input_video(&input_path)?;
    let output = PathBuf::from(&output_path);
    ensure_output_dir(&output)?;
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !["mp3", "wav", "m4a", "flac"].contains(&ext.as_str()) {
        return Err("输出扩展名必须是 mp3 / wav / m4a / flac 之一".to_string());
    }

    let (ffmpeg, _ffprobe) = ensure_ffmpeg_suite(&|_, _, _| {})?;
    let input_arg = input.to_string_lossy().into_owned();
    let output_arg = output.to_string_lossy().into_owned();
    let bitrate_arg = bitrate.filter(|b| !b.is_empty());
    let ext_inner = ext.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = quiet_command(ffmpeg);
        cmd.args(["-hide_banner", "-i", &input_arg]);
        match ext_inner.as_str() {
            "mp3" => {
                cmd.args(["-c:a", "libmp3lame"]);
                if let Some(b) = &bitrate_arg {
                    cmd.args(["-b:a", b]);
                }
            }
            "wav" => {
                cmd.args(["-c:a", "pcm_s16le", "-ar", "44100", "-ac", "2"]);
            }
            "m4a" => {
                cmd.args(["-c:a", "aac"]);
                if let Some(b) = &bitrate_arg {
                    cmd.args(["-b:a", b]);
                }
            }
            "flac" => {
                cmd.args(["-c:a", "flac"]);
            }
            _ => {}
        }
        cmd.args(["-y", &output_arg]);
        run_capture(&mut cmd, "音频转换")
    })
    .await
    .map_err(|e| format!("audio_convert task failed: {e}"))??;

    let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "format": ext,
        "size_bytes": size,
    }))
}