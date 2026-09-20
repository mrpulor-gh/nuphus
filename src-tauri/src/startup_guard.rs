//! 启动看门狗 —— 前端初始化彻底卡死时的兜底出口。
//!
//! 背景（2026-09-20 实测）：dev 模式下主窗走 Vite:5174，而 Vite 的预打包目录
//! 被上一次非正常退出（蓝屏）打断写入 → 重启后从头 re-optimize，与主窗页面
//! 加载竞态 → 主窗页面死在半途、React 从未执行 → 没人调 `finish_startup` →
//! splash 永远停在「正在启动…」，用户零出口（关闭 splash 与显示主窗全是
//! `let _ =`，且没有任何超时兜底）。
//!
//! 判据是「有没有动静」而不是「启动花了多久」：Rust 每个启动阶段、前端每次
//! `splash_status_update`、模型下载的每一段进度都会推 `splash:progress`
//! （见 `splash::emit_splash_progress`）。只要还在推就不算卡死 —— 首次运行
//! 的模型下载耗时数分钟也照样安全；真正卡死（前端没起来 / 死在半途 / 下载
//! 停滞）才会静默满 N 秒。
//!
//! 超时动作：ERROR 日志（下次排障一眼看到）+ 重载主窗 webview（前端若只是
//! 加载失败，reload 通常即可自愈）+ 关 splash + 显示主窗 + HUD 提示。

use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::commands::toolbar::STARTUP_FINISHED;

/// 默认静默阈值（秒）。`NUPHUS_STARTUP_TIMEOUT_SECS` 可覆盖，0 = 禁用看门狗。
/// 45s 的取舍：正常启动（含首次模型下载）全程有进度事件，够不到这个数；
/// 而卡死时用户等待时间也可接受。
pub const DEFAULT_TIMEOUT_SECS: u64 = 45;
/// 轮询间隔：够密以在超时后及时兜底，又几乎无开销。
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// 解析阈值（抽成纯函数便于测试）。
fn parse_timeout(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// 启动看门狗：独立 OS 线程（不占 tokio worker，也不受前端事件循环影响）。
///
/// 必须在 setup 末尾调用：此前各阶段推的 `splash:progress` 都已落地，
/// 起算点 = setup 结束时刻。
pub fn spawn(app: AppHandle) {
    let timeout_secs = parse_timeout(std::env::var("NUPHUS_STARTUP_TIMEOUT_SECS").ok().as_deref());
    if timeout_secs == 0 {
        tracing::info!("[StartupGuard] 看门狗已禁用（NUPHUS_STARTUP_TIMEOUT_SECS=0）");
        return;
    }
    crate::splash::mark_activity();

    let spawn_result = std::thread::Builder::new()
        .name("startup-guard".to_string())
        .spawn(move || {
            let limit_ms = timeout_secs * 1000;
            loop {
                std::thread::sleep(POLL_INTERVAL);
                if STARTUP_FINISHED.load(Ordering::SeqCst) {
                    // 正常路径：前端调过 finish_startup（或用户点了「后台下载」）。
                    // 日志里同时出现本行 = 前端确实起来了；只有 ERROR 那行才是卡死。
                    tracing::info!("[StartupGuard] 前端已就绪，看门狗退出");
                    return;
                }
                // 从未有过动静（异常）→ 直接按超时处理
                let idle_ms = crate::splash::idle_millis().unwrap_or(limit_ms);
                if idle_ms >= limit_ms {
                    fire(&app, idle_ms, timeout_secs);
                    return;
                }
            }
        });
    if let Err(e) = spawn_result {
        // 线程创建失败仅降级为「没有兜底」，绝不拖累启动本身
        tracing::warn!("[StartupGuard] Failed to spawn watchdog thread (no startup fallback): {e}");
    }
}

/// 判定卡死后的兜底：重载并显示主窗、关掉 splash、HUD 告知用户。
fn fire(app: &AppHandle, idle_ms: u64, timeout_secs: u64) {
    tracing::error!(
        "[StartupGuard] 前端 {}s 无任何启动动静（未收到 finish_startup，splash 亦无进度事件）——判定卡死，\
         强行打开主界面。dev 模式常见原因：Vite 预打包重建 / 前端 dev server 不可达 / 主窗页面加载失败；\
         若主界面仍空白请重启应用。",
        idle_ms / 1000
    );

    match app.get_webview_window("main") {
        Some(main) => {
            // 重载：前端若只是「加载失败」这一种死法，reload 通常即可自愈
            let _ = main.reload();
            let _ = main.show();
            let _ = main.set_focus();
        }
        None => tracing::error!("[StartupGuard] main 窗口缺失，无法打开主界面"),
    }

    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }

    // HUD 是独立窗口 + 独立前端：主界面 React 已经死掉时它照样能显示
    crate::commands::hud::hud_update(
        app.clone(),
        format!("启动超时（{timeout_secs}s 无响应）：已强行打开主界面，若仍空白请重启应用"),
        "error".to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_defaults_and_overrides() {
        assert_eq!(parse_timeout(None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_timeout(Some("15")), 15);
        assert_eq!(parse_timeout(Some(" 7 ")), 7);
        // 0 = 显式禁用，不能被当成「解析失败退回默认」
        assert_eq!(parse_timeout(Some("0")), 0);
        // 非法值退回默认
        assert_eq!(parse_timeout(Some("abc")), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_timeout(Some("-1")), DEFAULT_TIMEOUT_SECS);
    }
}
