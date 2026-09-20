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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SplashProgress {
    pub pct: Option<u8>,
    pub text: String,
}

/// 最近一次 splash 动静的时刻（UNIX 毫秒，0 = 从未有过）。
///
/// 启动看门狗（`crate::startup_guard`）用它判断「启动是否还在推进」：看动静
/// 而不是看耗时——首次运行的模型下载可以合法跑几分钟，只要进度在推就不算卡死。
static LAST_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 标记一次「启动还在动」。启动看门狗的起算点也走这里。
pub fn mark_activity() {
    LAST_ACTIVITY_MS.store(now_ms(), Ordering::SeqCst);
}

/// 距最近一次动静过去的毫秒数；从未有过动静时返回 None（由调用方决定怎么算）。
pub fn idle_millis() -> Option<u64> {
    let last = LAST_ACTIVITY_MS.load(Ordering::SeqCst);
    if last == 0 {
        None
    } else {
        Some(now_ms().saturating_sub(last))
    }
}

/// 广播一条 splash 进度。失败静默（splash 窗口可能尚未就绪/已关闭）。
pub fn emit_splash_progress(app: &AppHandle, pct: Option<u8>, text: &str) {
    // 有动静即续命看门狗（含 Rust 启动阶段、前端 splash_status_update、下载进度）
    mark_activity();
    let _ = app.emit(
        "splash:progress",
        SplashProgress {
            pct,
            text: text.to_string(),
        },
    );
}
