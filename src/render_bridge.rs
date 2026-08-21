//! render_bridge — Injection point for the desktop shell's document render
//! service (pdf.js in the main webview).
//!
//! The nuphus lib is Tauri-agnostic, and builtin tool executors are bare
//! `fn` pointers that cannot capture state. The PDF page renderer lives in
//! the desktop shell (src-tauri/src/render/, driving pdf.js inside the main
//! window's webview), so the shell registers a plain fn here at startup
//! (same process, direct call — no IPC). Pattern mirrors `video_bridge`.
//!
//! Contracts:
//! - render: (pdf path, max pages, optional 1-based page list) → one PNG
//!   byte buffer per rendered page (base64 decoding happens inside the
//!   bridge — the webview round-trip speaks base64, callers see only raw
//!   bytes). `None` page list renders `1..=max_pages` (legacy behavior);
//!   `Some` renders exactly those pages (mixed PDFs: OCR only the pages
//!   without a text layer).
//! - text extraction: (pdf path, max pages) → per-page text via pdf.js
//!   `getTextContent` (handles CID-keyed fonts that lopdf cannot). An empty
//!   string marks a page without a text layer (OCR candidate).
//!
//! Used by `utils::office::read_pdf`: text layer first, rendering + OCR
//! only for pages with no extractable text.

use std::sync::OnceLock;

/// Bridge implementation signature: (pdf path, max pages, optional 1-based
/// page list) → per-page PNG bytes.
pub type RenderPdfImpl = fn(&str, u32, Option<&[u32]>) -> Result<Vec<Vec<u8>>, String>;

/// Text-extraction bridge signature: (pdf path, max pages) → per-page text
/// (empty string = page has no text layer).
pub type ExtractPdfTextImpl = fn(&str, u32) -> Result<Vec<String>, String>;

static IMPL: OnceLock<RenderPdfImpl> = OnceLock::new();
static TEXT_IMPL: OnceLock<ExtractPdfTextImpl> = OnceLock::new();

/// Called once by the desktop shell at startup. Idempotent (first wins).
pub fn register_render_pdf_impl(f: RenderPdfImpl) {
    let _ = IMPL.set(f);
}

/// Called once by the desktop shell at startup. Idempotent (first wins).
pub fn register_extract_pdf_text_impl(f: ExtractPdfTextImpl) {
    let _ = TEXT_IMPL.set(f);
}

/// True when the desktop shell has registered the render service.
pub fn is_available() -> bool {
    IMPL.get().is_some()
}

/// True when the desktop shell has registered the text-extraction service.
pub fn is_text_available() -> bool {
    TEXT_IMPL.get().is_some()
}

/// Render a PDF to per-page PNG bytes (pages `1..=max_pages`). Err when the
/// shell never registered (e.g. headless builds / tests) — callers surface
/// this as an honest failure, never fabricated output.
pub fn render_pdf(path: &str, max_pages: u32) -> Result<Vec<Vec<u8>>, String> {
    render(path, max_pages, None)
}

/// Render only the given 1-based pages. Used by the mixed-PDF path to OCR
/// just the pages without a text layer instead of the whole document.
pub fn render_pdf_pages(path: &str, pages: &[u32]) -> Result<Vec<Vec<u8>>, String> {
    render(path, pages.len() as u32, Some(pages))
}

fn render(path: &str, max_pages: u32, pages: Option<&[u32]>) -> Result<Vec<Vec<u8>>, String> {
    match IMPL.get() {
        Some(f) => f(path, max_pages, pages),
        None => Err("PDF 渲染服务不可用（桌面壳未注册 render bridge）".to_string()),
    }
}

/// Extract per-page text via the shell's pdf.js service. Err when the shell
/// never registered the text-extraction bridge.
pub fn extract_pdf_text(path: &str, max_pages: u32) -> Result<Vec<String>, String> {
    match TEXT_IMPL.get() {
        Some(f) => f(path, max_pages),
        None => Err("PDF 文本提取服务不可用（桌面壳未注册 extract bridge）".to_string()),
    }
}
