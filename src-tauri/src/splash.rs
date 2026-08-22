//! Splash 进度推送 — 把启动阶段（模型下载/引擎加载）进度广播给 splash 窗口。
//!
//! 事件名 `splash:progress`，载荷 `{ pct?: u8, text: string }`：
//! - `pct` 为 0..=100 的百分比；`None` 表示不确定进度（只有文案）。
//! - 前端 splash.js 监听并更新进度条 + 状态文案。
//!
//! 取代旧的 `splash.eval("setStatus(...)")`：setStatus 定义在 splash.html 的
//! 内联 `<script>` 里，被 CSP `script-src 'self'`（无 unsafe-inline）拦截 →
//! setStatus 未定义，eval 静默失败，splash 状态文案实际上从未生效。事件推送
//! 走 Tauri IPC，不受页面 CSP 影响。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SplashProgress {
    pub pct: Option<u8>,
    pub text: String,
}

/// 广播一条 splash 进度。失败静默（splash 窗口可能尚未就绪/已关闭）。
pub fn emit_splash_progress(app: &AppHandle, pct: Option<u8>, text: &str) {
    let _ = app.emit(
        "splash:progress",
        SplashProgress {
            pct,
            text: text.to_string(),
        },
    );
}
