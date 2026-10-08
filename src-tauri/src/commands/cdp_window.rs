//! cdp_window.rs — 唤起 Agent 浏览器的窗口（CDP 驱动的那个 Chrome）。
//!
//! 与 `external.rs` 的分工：那边把外链交给**系统浏览器**（人在自己浏览器里看，
//! 带自己的登录态与扩展）；这边唤起的是 **Nuphus 自己的浏览器实例**——
//! 独立 profile，Agent 的自动化与人的示教、标注都在这个窗口里发生。
//!
//! 二者不是同一件事，故并存：人不必为了看链接切到自动化浏览器，但需要让
//! Agent 操作的那个窗口可见时（例如想看它进行到哪一步、或亲手示范一步），
//! 走本命令把它提到前台。

use nuphus::browser::{get_or_launch, runtime, BrowserError};

/// 唤起浏览器窗口：有窗口则提到前台，没有则启动一个并置前。
///
/// 返回窗口当前所在页面 URL，便于前端在按钮 title / 提示里回显实际落点
/// （例如用户之前停在某个站点上）。
#[tauri::command]
pub async fn browser_show_window() -> Result<String, String> {
    // 走browser crate 的进程级常驻 runtime：新建临时 runtime 会连带杀掉
    // chromiumoxide 的 CDP handler，留下连不上的僵尸连接（见 nuphus-browser
    // 模块文档的 runtime 说明）。
    let url = runtime()
        .block_on(async {
            // get_or_launch 的错误是面向人的可读字符串（它本来服务于 Agent 工具
            // 的错误回传），这里保留原文，不强行塞进 BrowserError 变体里。
            let mut guard = get_or_launch(/* headless */ false)
                .await
                .map_err(|e| BrowserError::Launch(e))?;
            let client = guard
                .as_mut()
                .ok_or_else(|| BrowserError::NotStarted)?;
            client.bring_to_front().await?;
            client.current_url().await
        })
        .map_err(|e| format!("打开浏览器窗口失败：{e}"))?;
    Ok(url)
}