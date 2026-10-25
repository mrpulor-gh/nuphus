//! cdp_window.rs — 唤起浏览器窗口（用户配了指纹浏览器时是用户那个，否则是
//! Nuphus 自管 CDP Chrome）。
//!
//! 与 `external.rs` 的分工：那边把外链交给**系统浏览器**（人在自己浏览器里看，
//! 带自己的登录态与扩展）；这边唤起的是 **Agent 自动化真正干活的那个浏览器
//! 实例**——用户配置了指纹浏览器（browser_cdp_url + browser_identity）时即
//! 用户那个（attach_external，失败硬报错、绝不自启自管 Chrome），未配时才是
//! Nuphus 自管实例（独立 profile）。
//!
//! 二者不是同一件事，故并存：人不必为了看链接切到自动化浏览器，但需要让
//! Agent 操作的那个窗口可见时（想看它进行到哪一步），走本命令把它提到前台。

use nuphus::browser::{get_or_launch, runtime, BrowserError};

/// 默认页导航裁决（browser_show_window 用）。
///
/// 仅「自管实例 + 当前空白页 + 传了默认页」三者同时成立才导。external
/// （用户指纹浏览器）一律不导：用户的标签页是现场——`about:blank` 也可能
/// 是他正要用的新标签；默认页只属于 Nuphus 自管实例的冷启动体验。
fn should_navigate_default(
    is_external: bool,
    current_url: &str,
    default_url: Option<&str>,
) -> Option<String> {
    if is_external || current_url != "about:blank" {
        return None;
    }
    default_url.map(str::to_owned)
}

/// 把诊断行追加到日志文件。
///
/// 本应用没有安装 tracing subscriber——既有代码里的 `tracing::warn!` 全部是
/// 空操作，失败信息无处可查（这正是「点了没反应、查不出原因」的直接原因）。
/// 这里自己落一份可读的文件，排查问题时看 `%APPDATA%\Nuphus\cdp_window.log`。
fn diag(line: &str) {
    use std::io::Write;
    let path = std::env::var("APPDATA")
        .map(|d| {
            std::path::PathBuf::from(d)
                .join("Nuphus")
                .join("cdp_window.log")
        })
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
pub async fn browser_show_window(
    // 空闲且当前页是空白页时导航到的默认页（Agent 执行中一律忽略，绝不打扰工作页）。
    // 由前端传执行态：running 时传 None，命令层再做一次服务端裁决。
    default_url: Option<String>,
) -> Result<String, String> {
    // 服务端二次校验：默认页必须是合法 http(s)（复用外链白名单，拒控制字符/伪协议）
    let default_url = default_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| crate::commands::external::is_allowed_external_url(s))
        .map(str::to_owned);
    tauri::async_runtime::spawn_blocking(move || {
        // 每行都带累计毫秒：入口慢在哪一段（预检 / 冷启 / 建页 / 置前）应当一眼可读，
        // 不必再靠外部探针复现（此前定位「每次等几秒」就是这么绕出来的）。
        let t0 = std::time::Instant::now();
        let ms = || t0.elapsed().as_millis();
        let url = runtime()
            .block_on(async {
                diag("step1: calling get_or_launch");
                let mut guard = match get_or_launch(/* headless */ false).await {
                    Ok(g) => g,
                    Err(e) => {
                        diag(&format!("get_or_launch FAILED after {}ms: {e}", ms()));
                        return Err(BrowserError::Launch(e));
                    }
                };
                let client = guard.as_mut().ok_or(BrowserError::NotStarted)?;
                let alive = client.is_connection_alive().await;
                diag(&format!(
                    "step2: bring_to_front ({}ms, connection alive = {alive})",
                    ms()
                ));
                if let Err(e) = client.bring_to_front().await {
                    diag(&format!("bring_to_front FAILED after {}ms: {e}", ms()));
                    return Err(e);
                }
                let url = client.current_url().await?;
                // 空白页 → 顺势导航到默认页（就地导航，不新开标签）。
                // 窗口已有真实页面（用户上次的浏览现场）时绝不打扰：按钮语义是
                // 「唤出/置前」，日常点击只置前不改动现场，避免标签堆积与页面顶掉。
                // 默认页导航裁决（纯函数 should_navigate_default）：仅「自管实例 +
                // 空白页 + 有默认页」才导。external（用户指纹浏览器）一律不导——
                // 用户的标签页是现场，about:blank 也可能是他正要用的新标签，
                // 置前即可；默认页只属于 Nuphus 自管实例的冷启动。
                if let Some(target) = should_navigate_default(
                    client.external_cdp_url().is_some(),
                    &url,
                    default_url.as_deref(),
                ) {
                    diag(&format!(
                        "step3: navigating blank page to default ({})",
                        ms()
                    ));
                    match client.navigate(&target).await {
                        Ok(_) => {
                            diag(&format!("default page loaded after {}ms", ms()));
                            return Ok(target);
                        }
                        // 导航失败不阻断置前成功的主流程：窗口已在前台，
                        // 用户看得见空白页本身，报错反而噪音。
                        Err(e) => diag(&format!("default page navigate FAILED: {e}")),
                    }
                }
                diag(&format!("window front after {}ms, url = {url}", ms()));
                Ok(url)
            })
            .map_err(|e| format!("打开浏览器窗口失败：{e}"))?;
        Ok(url)
    })
    .await
    .map_err(|e| format!("打开浏览器窗口失败：{e}"))?
}

/// 用户点击外链时的落点（裸 URL chip / markdown 链接 / 各页面外链共用）。
#[derive(serde::Serialize)]
pub struct OpenedUrl {
    /// 本次调用是否**新启动**了浏览器（false = 窗口本来就在）
    pub launched: bool,
    /// 实际落点 URL（前端回显用）
    pub url: String,
}

/// 在 Agent 浏览器（CDP 那个 Chrome）里**新开一个标签页**打开外链，并把窗口置前。
///
/// ## 为什么不再交给系统浏览器（2026-10-09 大王定调）
///
/// 用户点外链的常见动机是「Agent 让我登录某个网站」。登录态必须落在**工作流/Agent
/// 用的那个 profile** 里——用户在系统浏览器登录，工作流照样过不去。统一到 CDP 浏览器
/// 之后，用户在这个窗口里登录、授权、填表，Agent 随后直接复用同一份状态。
///
/// ## 为什么是新开标签而不是导航当前页
///
/// 点链接的时机常常正好是 Agent 在自动化某个页面时；导航会把它正在操作的那张页顶掉，
/// 轻则打断流程，重则让它的下一步操作落到用户的页面上。
///
/// ## 这里为什么没有「重连再试一次」
///
/// 曾经有（2026-10-09）：`get_or_launch` 拿到的连接可能已经死了（Chrome 被关掉、
/// 上次会话遗留），于是 `new_tab` 先撞一次 `receiver is gone`，再由调用方 reconnect
/// 重试——每次点外链白付 0–9s，还掩盖了真正的问题。
///
/// 根因在 `BrowserClient::launch`（crates/nuphus-browser/src/client.rs）：它把探活
/// 失败一律当成「连接还好」并返回 Ok。现已改为**证明式判死**——只有拿到浏览器进程
/// 确实消失的操作系统级证据（自启子进程已退出 / 记录端口不再接受连接）才 close 并
/// 重启，其余情况保留原连接。`get_or_launch` 因此要么给出能干活的连接，要么直接报错。
/// `new_tab` 也已在自己的契约里完成导航（不再指望调用方补 navigate）。
///
/// 所以这里不再自愈：连接是在本命令执行期间才被外力关掉的话，如实报错比假装成功好
/// ——重试治不好用户连续关窗口，而一次真实的失败至少能让用户看见发生了什么。
///
/// 校验复用 `external::is_allowed_external_url`（只放行 http/https、拒控制字符与超长）。
#[tauri::command]
pub async fn browser_open_url(url: String) -> Result<OpenedUrl, String> {
    let requested = url.trim().to_string();
    if !crate::commands::external::is_allowed_external_url(&requested) {
        return Err(format!("不支持的链接（仅允许 http/https）：{requested}"));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let ms = || t0.elapsed().as_millis();
        let out = runtime()
            .block_on(async move {
                // 判定「浏览器本来在不在」必须在**取锁之前**：拿不到锁说明此刻正被别人
                // 用着（多半是 Agent 在跑），保守当成"已在" → 前端不弹"正在启动"。
                let pre_existed = match nuphus::browser::shared_client().try_lock() {
                    Ok(guard) => guard.is_some(),
                    // tokio 的 TryLockError 只有 WouldBlock：拿不到锁 = 此刻正被别人用着
                    Err(_) => true,
                };

                let mut guard = get_or_launch(/* headless */ false)
                    .await
                    .map_err(BrowserError::Launch)?;
                let client = guard.as_mut().ok_or(BrowserError::NotStarted)?;
                // new_tab 返回时 self.page 一定停在 requested 上（见其契约）。
                if let Err(e) = client.new_tab(Some(&requested)).await {
                    diag(&format!(
                        "open_url: new_tab FAILED after {}ms: {e}",
                        ms()
                    ));
                    return Err(e);
                }
                if let Err(e) = client.bring_to_front().await {
                    diag(&format!(
                        "open_url: bring_to_front FAILED after {}ms: {e}",
                        ms()
                    ));
                    return Err(e);
                }
                let landed = match client.current_url().await {
                    Ok(url) => url,
                    Err(e) => {
                        diag(&format!(
                            "open_url: current_url FAILED after {}ms ({e}); falling back to requested",
                            ms()
                        ));
                        requested.clone()
                    }
                };
                diag(&format!(
                    "open_url done after {}ms: pre_existed={pre_existed} landed={landed}",
                    ms()
                ));
                Ok(OpenedUrl {
                    launched: !pre_existed,
                    url: landed,
                })
            })
            .map_err(|e| format!("在 Agent 浏览器中打开链接失败：{e}"))?;
        Ok(out)
    })
    .await
    .map_err(|e| format!("在 Agent 浏览器中打开链接失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端按 `launched` / `url` 取值（见 `api.ts` 的 `AgentBrowserOpened`），
    /// 字段名一旦漂移前端会静默拿到 undefined —— 这里把序列化形状钉住。
    #[test]
    fn opened_url_serializes_with_stable_field_names() {
        let json = serde_json::to_value(OpenedUrl {
            launched: true,
            url: "https://example.com/".into(),
        })
        .expect("serialize");
        assert_eq!(json["launched"], serde_json::json!(true));
        assert_eq!(json["url"], serde_json::json!("https://example.com/"));
    }

    /// 默认页导航裁决：external 一律不导、真实页不导、无默认页不导。
    #[test]
    fn default_page_navigation_only_for_managed_blank() {
        // 自管 + 空白页 + 有默认页 → 导（冷启动体验）
        assert_eq!(
            should_navigate_default(false, "about:blank", Some("https://www.nuphus.com/")),
            Some("https://www.nuphus.com/".to_string())
        );
        // 自管 + 真实页 → 不导（不顶用户现场）
        assert_eq!(
            should_navigate_default(false, "https://work.example", Some("https://x")),
            None
        );
        // external + 空白页 → 不导（用户的空白标签也是现场）
        assert_eq!(
            should_navigate_default(true, "about:blank", Some("https://x")),
            None
        );
        // 无默认页 → 不导
        assert_eq!(should_navigate_default(false, "about:blank", None), None);
    }

    /// 校验复用 external 的白名单：伪协议必须在进浏览器之前就被拒。
    #[test]
    fn rejects_non_http_urls_before_launching() {
        for bad in [
            "javascript:alert(1)",
            "file:///C:/Windows",
            "data:text/html,<script>",
            "https://",
        ] {
            assert!(
                !crate::commands::external::is_allowed_external_url(bad),
                "should reject: {bad}"
            );
        }
        assert!(crate::commands::external::is_allowed_external_url(
            "https://example.com/path?q=1"
        ));
    }
}
