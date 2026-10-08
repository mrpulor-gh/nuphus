//! cdp_window.rs — 唤起 Agent 浏览器的窗口（CDP 驱动的那个 Chrome）。
//!
//! 与 `external.rs` 的分工：那边把外链交给**系统浏览器**（人在自己浏览器里看，
//! 带自己的登录态与扩展）；这边唤起的是 **Nuphus 自己的浏览器实例**——
//! 独立 profile，Agent 的自动化都发生在这个窗口里。
//!
//! 二者不是同一件事，故并存：人不必为了看链接切到自动化浏览器，但需要让
//! Agent 操作的那个窗口可见时（想看它进行到哪一步），走本命令把它提到前台。

use nuphus::browser::{get_or_launch, runtime, BrowserError};

/// 把诊断行追加到日志文件。
///
/// 本应用没有安装 tracing subscriber——既有代码里的 `tracing::warn!` 全部是
/// 空操作，失败信息无处可查（这正是「点了没反应、查不出原因」的直接原因）。
/// 这里自己落一份可读的文件，排查问题时看 `%APPDATA%\Nuphus\cdp_window.log`。
fn diag(line: &str) {
    use std::io::Write;
    let path = std::env::var("APPDATA")
        .map(|d| std::path::PathBuf::from(d).join("Nuphus").join("cdp_window.log"))
        .unwrap_or_else(|_| std::path::PathBuf::from("cdp_window.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{now}] {line}");
    }
}

/// 唤起浏览器窗口：有窗口则提到前台，没有则启动一个并置前。
///
/// 返回窗口当前所在页面 URL，便于前端回显实际落点。
///
/// ## 为什么先 spawn_blocking 再 block_on
///
/// 不能在 async 上下文里直接 `nuphus_browser::runtime().block_on(...)`：Tauri 把
/// async command 调度到它自己的 runtime，而那是另一个独立 runtime。在 async
/// 上下文里 block_on 会占住 Tauri 的 worker 线程，chromiumoxide 的 CDP handler
/// 又需要 browser runtime 被驱动——两个 runtime 互相等，命令永远不返回
/// （现场表现：按钮点了完全没反应，日志只停在入口那一行）。
///
/// 做法：spawn_blocking 把整段工作挪到阻塞线程，在那里 block_on 是合法的
/// （没有外层 async 在等它），主流程只 await 这个 JoinHandle。
#[tauri::command]
pub async fn browser_show_window() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let url = runtime()
            .block_on(async {
                diag("step1: calling get_or_launch");
                let mut guard = match get_or_launch(/* headless */ false).await {
                    Ok(g) => g,
                    Err(e) => {
                        diag(&format!("get_or_launch FAILED: {e}"));
                        return Err(BrowserError::Launch(e));
                    }
                };
                let client = guard
                    .as_mut()
                    .ok_or(BrowserError::NotStarted)?;
                diag(&format!(
                    "step2: bring_to_front (connection alive = {})",
                    client.is_connection_alive().await
                ));
                if let Err(e) = client.bring_to_front().await {
                    diag(&format!("bring_to_front FAILED: {e}"));
                    return Err(e);
                }
                let url = client.current_url().await?;
                diag(&format!("window front, url = {url}"));
                Ok(url)
            })
            .map_err(|e| format!("打开浏览器窗口失败：{e}"))?;
        Ok(url)
    })
    .await
    .map_err(|e| format!("打开浏览器窗口失败：{e}"))?
}
