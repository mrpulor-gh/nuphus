//! Document render service — pdf.js inside the main webview, driven from Rust.
//!
//! First consumer: scanned-PDF OCR fallback in `nuphus::utils::office::read_pdf`
//! (lopdf extracts zero text → render pages to PNG → PaddleOCR).
//!
//! Modules:
//! - `commands` — Tauri commands (pdf_render_done / pdf_render_error) +
//!   nuphus-lib render bridge injection (fn-pointer, single process, no IPC)

pub mod commands;
