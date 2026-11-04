//! artifacts — 生成产物落盘与下载（协议无关的文件层）
//!
//! 只做「把协议层交出的载体写到 `~/.nuphus/generated/`」：远程 URL 下载、
//! base64 解码、命名带时间戳、带回字节数。这里不认识任何服务商——端点、
//! 请求键名、响应解析全部活在协议实现里。

use super::{Artifact, GenError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// 单个文件下载上限（防异常响应打爆磁盘）
pub(super) const DOWNLOAD_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// 待落盘的产物载体：远程 URL 或内联 base64（协议无关的两种载荷形态）。
pub(super) enum Payload {
    Url(String),
    Base64(String),
}

/// 生成工具专用 blocking client（复用 web.rs 的 AGENT_TTL 缓存模式）
static GEN_AGENT_CACHE: Mutex<Option<(reqwest::blocking::Client, i64)>> = Mutex::new(None);

pub(super) fn get_agent() -> reqwest::blocking::Client {
    let now = crate::tools::builtin::web::unix_now();
    if let Ok(cache) = GEN_AGENT_CACHE.lock() {
        if let Some((ref agent, ts)) = *cache {
            if now - ts < crate::tools::builtin::web::AGENT_TTL {
                return agent.clone();
            }
        }
    }
    let agent = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to build reqwest blocking Client");
    if let Ok(mut cache) = GEN_AGENT_CACHE.lock() {
        *cache = Some((agent.clone(), now));
    }
    agent
}

/// reqwest 错误归一：超时 / 连接 / 其它（不出现服务商名）。
pub(super) fn classify_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("API request failed: timeout ({e})")
    } else if e.is_connect() {
        format!("API request failed: connection error ({e})")
    } else {
        format!("API request failed: {e}")
    }
}

/// 产出目录：~/.nuphus/generated/（自动创建）
pub(super) fn generated_dir() -> Result<PathBuf, GenError> {
    let home = dirs::home_dir().ok_or_else(|| GenError::transport("无法定位用户主目录"))?;
    let dir = home.join(".nuphus").join("generated");
    std::fs::create_dir_all(&dir)
        .map_err(|e| GenError::transport(format!("创建产出目录失败 {}: {e}", dir.display())))?;
    Ok(dir)
}

pub(super) fn file_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S_%3f").to_string()
}

/// 从 URL 推断图像扩展名（白名单内取原扩展，否则兜底 jpg）。
pub(super) fn image_ext_from_url(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "png",
        "webp" => "webp",
        "jpeg" => "jpeg",
        _ => "jpg",
    }
}

/// 下载 URL 到本地文件（带大小上限）。
fn download_to_file(
    agent: &reqwest::blocking::Client,
    url: &str,
    path: &Path,
) -> Result<u64, GenError> {
    let mut resp = agent.get(url).send().map_err(|e| {
        GenError::transport(format!("下载生成文件失败: {}", classify_reqwest_error(&e)))
    })?;
    if !resp.status().is_success() {
        return Err(GenError::transport(format!(
            "下载生成文件失败: HTTP {}",
            resp.status()
        )));
    }
    let mut file = std::fs::File::create(path)
        .map_err(|e| GenError::transport(format!("创建文件失败 {}: {e}", path.display())))?;
    let written = std::io::copy(&mut resp, &mut file)
        .map_err(|e| GenError::transport(format!("写入文件失败 {}: {e}", path.display())))?;
    if written == 0 {
        let _ = std::fs::remove_file(path);
        return Err(GenError::transport(format!("下载内容为空: {url}")));
    }
    if written > DOWNLOAD_CAP_BYTES {
        let _ = std::fs::remove_file(path);
        return Err(GenError::transport(format!(
            "下载文件超过大小上限 ({DOWNLOAD_CAP_BYTES} bytes): {url}"
        )));
    }
    Ok(written)
}

/// base64 载荷落盘。
fn save_base64_image(b64: &str, path: &Path) -> Result<u64, GenError> {
    use base64::Engine;
    // 兼容 data URI 前缀形态
    let payload = b64.split_once(',').map(|(_, p)| p).unwrap_or(b64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| GenError::protocol(format!("image_base64 解码失败: {e}")))?;
    if bytes.is_empty() {
        return Err(GenError::protocol("image_base64 解码结果为空"));
    }
    std::fs::write(path, &bytes)
        .map_err(|e| GenError::transport(format!("写入文件失败 {}: {e}", path.display())))?;
    Ok(bytes.len() as u64)
}

/// 图像载体落盘：URL 下载 / base64 解码，命名 `image_<ts>_<i>.<ext>`
/// （顺序即协议返回顺序，主产物 = 第一条）。
pub(super) fn save_image_payloads(
    agent: &reqwest::blocking::Client,
    items: &[Payload],
) -> Result<Vec<Artifact>, GenError> {
    let dir = generated_dir()?;
    let ts = file_timestamp();
    let mut saved = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let artifact = match item {
            Payload::Url(url) => {
                let path = dir.join(format!("image_{ts}_{i}.{}", image_ext_from_url(url)));
                let bytes = download_to_file(agent, url, &path)?;
                Artifact {
                    path,
                    bytes,
                    task_id: None,
                }
            }
            Payload::Base64(b64) => {
                let path = dir.join(format!("image_{ts}_{i}.jpg"));
                let bytes = save_base64_image(b64, &path)?;
                Artifact {
                    path,
                    bytes,
                    task_id: None,
                }
            }
        };
        saved.push(artifact);
    }
    Ok(saved)
}

/// 视频下载落盘：命名 `video_<ts>.mp4`，任务号随产物带回（工具文案要用）。
pub(super) fn save_video_download(
    agent: &reqwest::blocking::Client,
    url: &str,
    task_id: &str,
) -> Result<Artifact, GenError> {
    let dir = generated_dir()?;
    let path = dir.join(format!("video_{}.mp4", file_timestamp()));
    let bytes = download_to_file(agent, url, &path)?;
    Ok(Artifact {
        path,
        bytes,
        task_id: Some(task_id.to_string()),
    })
}
