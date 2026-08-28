//! preview:// 自定义协议 — 沙箱化本地文件运行底座
//!
//! 设计定位（大王定调）：功能底座，不绑死 HTML。通用能力 = 路径 → 文件字节
//! + mime 推断 + 独立安全头。任何前端组件（当前 PreviewOverlay，未来画廊 /
//! 音视频面板等）经 convertFileSrc(path, 'preview') 即可获得可运行文档。
//!
//! 安全模型（与主应用 CSP 隔离）：
//! - iframe `sandbox` 属性：无相同源泄漏面，预览内容碰不到主应用与系统
//! - 响应头 `Content-Security-Policy: sandbox ...`：协议层兜底沙箱——即使
//!   前端 iframe 属性被遗漏，文档仍被强制沙箱化（纵深防御）
//! - 独立宽松 CSP 只作用于 preview 响应本身：agent 产出的 HTML 游戏/交互
//!   demo 可执行内联脚本、引用 CDN 引擎与同目录资源，主应用 CSP 一字不动

use std::path::PathBuf;

use tauri::http::{header, Request, Response, StatusCode};

/// 单文件预览大小上限：覆盖大型游戏/音视频产物，防误读超大文件耗尽内存
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;

/// 注册 preview 协议到 Builder（main.rs Builder 链首调用）
pub fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol("preview", move |_ctx, request, responder| {
        // 文件 IO 移出协议回调线程，避免大文件阻塞事件循环
        tauri::async_runtime::spawn_blocking(move || {
            responder.respond(serve(request));
        });
    })
}

fn serve(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    // convertFileSrc 产物：Windows http://preview.localhost/<percent-encoded>
    //            Unix  preview://localhost/<percent-encoded>——path 段结构一致
    let raw = request.uri().path().trim_start_matches('/');
    let decoded = percent_decode(raw);
    let path = PathBuf::from(&decoded);

    if decoded.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "缺少文件路径");
    }
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &format!("无法读取文件: {e}")),
    };
    if !meta.is_file() {
        return error_response(StatusCode::BAD_REQUEST, "路径不是文件");
    }
    if meta.len() > MAX_PREVIEW_BYTES {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE, "文件超过预览大小上限（64 MB）");
    }

    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = guess_mime(&path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                // 文档级沙箱兜底 + 资源自由加载（内联脚本/CDN/同目录相对引用）
                .header(
                    "Content-Security-Policy",
                    "sandbox allow-scripts allow-same-origin allow-pointer-lock allow-modals \
                     allow-forms; default-src * 'unsafe-inline' 'unsafe-eval' data: blob:; \
                     img-src * data: blob:; media-src * data: blob:; font-src * data:; \
                     connect-src * data: blob:",
                )
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(bytes)
                .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "响应构建失败"))
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("读取失败: {e}")),
    }
}

fn error_response(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_else(|_| Response::builder().status(status).body(Vec::new()).unwrap())
}

/// 手写 percent-decode（避免为单一用途引入 url crate）
/// 注意：不做 `+` → 空格转换——那是 form 编码规则，路径中的 `+`（如 C++ 目录）必须原样保留
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 扩展名 → mime。覆盖网页游戏/交互 demo 常见资源类型，未知类型走
/// octet-stream（浏览器下载或忽略，不 crash）
fn guess_mime(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "txt" | "md" | "log" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_windows_path() {
        assert_eq!(percent_decode("C%3A%5CUsers%5Cgame.html"), "C:\\Users\\game.html");
    }

    #[test]
    fn percent_decode_unix_path() {
        assert_eq!(percent_decode("%2FUsers%2Fme%2Findex.html"), "/Users/me/index.html");
    }

    #[test]
    fn percent_decode_preserves_plus() {
        assert_eq!(percent_decode("C%3A%5C C%2B%2B%5Cindex.html"), "C:\\ C++\\index.html");
    }

    #[test]
    fn percent_decode_invalid_escape_kept() {
        assert_eq!(percent_decode("100%zz"), "100%zz");
    }
}
