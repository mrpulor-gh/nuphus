//! Tauri command surface + nuphus-lib render bridge for the PDF render service.
//!
//! Round-trip (Tauri v2 `window.eval` has no return value):
//!   1. Rust: read PDF bytes → base64 → eval the frontend entry:
//!      `window.__nuphusRenderPdf(requestId, b64, maxPages, pageList?)`（渲染）
//!      `window.__nuphusRenderPdfText(requestId, b64, maxPages)`（文本提取）
//!   2. JS (frontend/src/core/pdf-render.ts): pdf.js renders each page to an
//!      offscreen canvas → PNG base64, or extracts the text layer via
//!      getTextContent → invoke `pdf_render_done` / `pdf_render_error`
//!   3. Rust: the command settles the pending channel; the render bridge
//!      decodes base64 → raw PNG bytes (text results are returned as-is).
//!      OCR 全程内存，不落临时文件
//!
//! Pattern mirrors video/commands.rs (fn-pointer injection, registered at app
//! setup so the nuphus lib can reach this shell-side service).

use base64::Engine;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// AppHandle for the bridge path (fn pointer has no Tauri State access).
static APP: OnceLock<AppHandle> = OnceLock::new();

/// In-flight render requests: request_id → result sender.
/// `Vec<String>` is one PNG base64 string per page (render) or one text
/// string per page (text extraction), as produced by the webview.
type RenderResult = Result<Vec<String>, String>;
static PENDING: OnceLock<Mutex<HashMap<String, mpsc::Sender<RenderResult>>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, mpsc::Sender<RenderResult>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 前端渲染 + 回传的整体等待上限
const RENDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Called once from main.rs setup: stores the AppHandle and registers the
/// bridge implementation into the nuphus lib.
pub fn init_bridge(app: &AppHandle) {
    let _ = APP.set(app.clone());
    nuphus::render_bridge::register_render_pdf_impl(bridge_render_pdf);
    nuphus::render_bridge::register_extract_pdf_text_impl(bridge_extract_pdf_text);
    tracing::info!("[render] tool bridge registered");
}

/// Bridge entry: drive the main window's pdf.js service and return one PNG
/// byte buffer per rendered page. `pages` = Some 时仅渲染指定 1-based 页码
/// （混合 PDF 只对无文本层页做 OCR），None 时维持 1..=max_pages 旧行为。
fn bridge_render_pdf(
    path: &str,
    max_pages: u32,
    pages: Option<&[u32]>,
) -> Result<Vec<Vec<u8>>, String> {
    // 页码为 u32 数字，可安全嵌入 JS 数组字面量
    let pages_arg = match pages {
        Some(ps) => format!(
            "[{}]",
            ps.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        ),
        None => "null".to_string(),
    };
    let pages_b64 = run_frontend_job(path, |request_id, pdf_b64| {
        format!(
            "window.__nuphusRenderPdf && window.__nuphusRenderPdf('{request_id}', '{pdf_b64}', {max_pages}, {pages_arg})"
        )
    })
    .map_err(|e| format!("PDF 渲染失败: {e}"))?;
    pages_b64
        .iter()
        .enumerate()
        .map(|(i, p)| {
            base64::engine::general_purpose::STANDARD
                .decode(p)
                .map_err(|e| format!("解码第 {} 页渲染结果失败: {e}", i + 1))
        })
        .collect()
}

/// Bridge entry: drive the main window's pdf.js text extraction and return
/// per-page text (empty string = no text layer on that page). 文本无需
/// base64 解码，直接回传。
fn bridge_extract_pdf_text(path: &str, max_pages: u32) -> Result<Vec<String>, String> {
    run_frontend_job(path, |request_id, pdf_b64| {
        format!(
            "window.__nuphusRenderPdfText && window.__nuphusRenderPdfText('{request_id}', '{pdf_b64}', {max_pages})"
        )
    })
    .map_err(|e| format!("PDF 文本提取失败: {e}"))
}

/// 渲染/提取共用的前后端往返：读文件 → base64 → eval 前端入口 → 等待
/// `pdf_render_done` / `pdf_render_error` 回传（60s 超时），清理挂起项。
fn run_frontend_job(
    path: &str,
    script: impl FnOnce(&str, &str) -> String,
) -> Result<Vec<String>, String> {
    let app = APP
        .get()
        .ok_or_else(|| "PDF 渲染服务未初始化（桌面壳未注册桥接）".to_string())?;
    let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 失败 {path}: {e}"))?;
    let pdf_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在，无法渲染 PDF".to_string())?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<RenderResult>();
    pending()
        .lock()
        .map_err(|_| "渲染桥内部锁污染".to_string())?
        .insert(request_id.clone(), tx);

    // base64 字母表（A-Za-z0-9+/=）不含引号与反斜杠，可安全嵌入 JS 字符串字面量。
    // 前端服务未注册时此调用静默无效，依赖下方超时返回结构化错误。
    let outcome = match window.eval(script(&request_id, &pdf_b64)) {
        Err(e) => Err(format!("触发前端渲染失败: {e}")),
        Ok(()) => match rx.recv_timeout(RENDER_TIMEOUT) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err("前端渲染超时（60秒）: 渲染服务未注册或渲染过久".to_string())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("渲染结果通道异常断开".to_string()),
        },
    };
    // 所有路径都必须清掉挂起项，避免 HashMap 泄漏
    pending()
        .lock()
        .map_err(|_| "渲染桥内部锁污染".to_string())?
        .remove(&request_id);

    outcome
}

/// Settle one in-flight request. Unknown/expired ids are rejected honestly
/// (duplicate or late frontend callbacks surface as command errors).
fn settle(request_id: String, result: RenderResult) -> Result<(), String> {
    let tx = pending()
        .lock()
        .map_err(|_| "渲染桥内部锁污染".to_string())?
        .remove(&request_id);
    match tx {
        Some(tx) => {
            // 接收端可能已因超时离开 — 结果丢弃即可
            let _ = tx.send(result);
            Ok(())
        }
        None => Err(format!("未知或过期的渲染请求: {request_id}")),
    }
}

#[tauri::command]
pub fn pdf_render_done(request_id: String, pages: Vec<String>) -> Result<(), String> {
    settle(request_id, Ok(pages))
}

#[tauri::command]
pub fn pdf_render_error(request_id: String, error: String) -> Result<(), String> {
    settle(request_id, Err(error))
}
