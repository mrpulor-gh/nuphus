// rec_browser.rs — 浏览器「网页点击」录制的后端捕获器（CDP 注入捕获）
//
// 与 rec.rs 桌面录制（rec_start：低层 hook 捕获真实桌面点击）不同，网页点击
// 录制面向 Nuphus managed 浏览器（CDP chromiumoxide）：向当前页注入一段
// 自包含的 click listener + CSS selector 生成器，用户在浏览器窗口里的真实
// 点击被捕获为「当前页唯一」的稳定 selector，供前端生成可运行的 browser_click
// 步骤（executor 已白名单 browser_click，运行侧零改动）。
//
// 三个命令（Batch A；Batch B 前端轮询消费）：
//   rec_browser_capture_click_start  校验录制会话 → 确保浏览器已启动(headed)
//                                     → 注入捕获脚本(幂等) → 返回当前页 url
//   rec_browser_capture_click_poll   读取最近一次点击捕获（读后清空；错误/丢失
//                                    注入亦有明确信号），前端 ~500ms 轮询
//   rec_browser_capture_cancel       清空捕获结果（保留注入与监听，幂等）
//
// 设计约束：
//   - 复用 rec.rs 录制会话：start 要求会话 active（busy/workflow-active 已被
//     rec_set_workflow 闸门拒绝 → 用户操作浏览器期间无 agent 并行，无需
//     automation_lock）；cancel 不依赖会话态（幂等清理）。
//   - 浏览器 client 复用进程级 shared 单例（nuphus::browser::shared_client /
//     get_or_launch）——与 browser_tools / web CDP 渲染同一实例同一 profile。
//   - 捕获脚本自包含，只依赖页面 DOM API，不引用任何 Nuphus 内部变量。
//   - 注入幂等：同 document 重复 start 不重复挂 listener（already 信号）。
//
// 已知边界（MVP 接受，见 start 返回 note 与模块 doc）：
//   - chromiumoxide evaluate 只在主 frame 执行。iframe 内的点击不跨 document
//     传播，主 frame listener 收不到；捕获不到时 poll 返回 { captured:false }，
//     前端按超时/提示处理（可引导用户改用页面内操作或交 WorkflowAgent）。
//   - 捕获脚本只存在于注入时的 document：整页导航/刷新后监听与标记均丢失，
//     poll 检测到后返回 { captured:false, need_reinject:true }，前端可自动
//     再次调 start（幂等重注入）。
//   - 浏览器内部页（chrome://、扩展页、PDF 查看器等）无法注入 evaluate，
//     start 返回明确中文错误。

use nuphus::browser::BrowserError;
use serde_json::Value;

// ═════════════════════════════════════════════════════════════
// 注入脚本（自包含、幂等、与页面运行时代码零耦合）
// ═════════════════════════════════════════════════════════════

/// start 注入的点击捕获脚本。
///
/// selector 生成优先级（每级都经 querySelectorAll 全局唯一性验证，不唯一降级）：
///   1. 稳定属性：id（仅 CSS 合法形式）→ data-testid → data-cy → name → aria-label
///   2. class：单 class 唯一 → 全部 class 组合 → 逐步缩短仍唯一
///   3. 兜底：el → body 的 `tag:nth-of-type(n) > ...` 层级路径，超过 5 层放弃
///      （避免超长脆弱 selector），仍不唯一返回 null
/// 点击以 capture 阶段监听（addEventListener 第三参 true）：先于页面自身
/// handler 且对 document 必达；只记录最后一次未消费点击（录制 UI 单步语义）。
const INJECT_CAPTURE_JS: &str = r#"(() => {
  if (window.__nuphus_rec_injected) {
    return JSON.stringify({ ok: true, already: true });
  }
  window.__nuphus_rec_injected = true;
  window.__nuphus_rec_capture = null;

  function isUnique(sel) {
    try {
      return document.querySelectorAll(sel).length === 1;
    } catch (err) {
      return false;
    }
  }

  function escapeAttrValue(v) {
    return v
      .replace(/\\/g, '\\\\')
      .replace(/"/g, '\\"')
      .replace(/\n/g, '\\a ')
      .replace(/\r/g, '\\d ');
  }

  // 1) 稳定属性候选：按稳定性降序，每个候选都必须全局唯一才采用。
  //    id 仅接受 CSS 合法形式（字母/下划线开头，后接字母数字 _ -），
  //    含特殊字符的 id 放弃（避免 querySelector 抛错/转义脆弱）。
  function attrSelector(el) {
    var attrs = ['id', 'data-testid', 'data-cy', 'name', 'aria-label'];
    for (var i = 0; i < attrs.length; i++) {
      var attr = attrs[i];
      var raw = el.getAttribute ? el.getAttribute(attr) : null;
      if (raw === null || raw === undefined) continue;
      var val = ('' + raw).trim();
      if (!val) continue;
      var sel = null;
      if (attr === 'id') {
        if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(val)) continue;
        sel = '#' + val;
      } else {
        sel = '[' + attr + '="' + escapeAttrValue(val) + '"]';
      }
      if (isUnique(sel)) return sel;
    }
    return null;
  }

  // 2) class 候选：单 class 唯一 → 全部 class 组合 → 从右端逐步缩短
  //    （优先保留最具体且仍唯一的组合；组合数受元素实际 class 数限制）。
  function classSelector(el) {
    var classes = [];
    var list = el.classList || [];
    for (var i = 0; i < list.length; i++) {
      var c = ('' + list[i]).trim();
      if (c) classes.push(c);
    }
    if (!classes.length) return null;
    for (var j = 0; j < classes.length; j++) {
      var one = '.' + CSS.escape(classes[j]);
      if (isUnique(one)) return one;
    }
    var combo = classes.slice();
    while (combo.length > 1) {
      var parts = [];
      for (var k = 0; k < combo.length; k++) parts.push('.' + CSS.escape(combo[k]));
      var joined = parts.join('');
      if (isUnique(joined)) return joined;
      combo.pop();
    }
    return null;
  }

  // 同级同 tag 元素序号（1-based，:nth-of-type 语义）
  function nthOfType(el) {
    var n = 1;
    var sib = el;
    while ((sib = sib.previousElementSibling)) {
      if (sib.tagName === el.tagName) n++;
    }
    return n;
  }

  // 3) 兜底：el → body/html 的 'tag:nth-of-type(n)' 层级路径。
  //    段数超过 5（el 距离根过深）直接放弃，防超长脆弱 selector。
  function pathSelector(el) {
    var segs = [];
    var node = el;
    while (node && node.nodeType === 1 && node.tagName !== 'BODY' && node.tagName !== 'HTML') {
      segs.unshift(node.tagName.toLowerCase() + ':nth-of-type(' + nthOfType(node) + ')');
      if (segs.length > 5) return null;
      node = node.parentElement;
    }
    if (!node) return null; // detached 元素
    if (node.tagName !== 'BODY' && node.tagName !== 'HTML') return null;
    if (!segs.length) return null; // 目标即 body/html 本身：无法表达为可点击 selector
    var sel = (node.tagName === 'BODY' ? 'body' : 'html') + ' > ' + segs.join(' > ');
    return isUnique(sel) ? sel : null;
  }

  function genSelector(el) {
    if (!el || el.nodeType !== 1) return null;
    var byAttr = attrSelector(el);
    if (byAttr) return byAttr;
    var byClass = classSelector(el);
    if (byClass) return byClass;
    return pathSelector(el);
  }

  document.addEventListener('click', function (e) {
    var el = e.target && e.target.nodeType === 1 ? e.target : (e.target ? e.target.parentElement : null);
    if (!el || el.nodeType !== 1) return;
    // disabled 控件点击无意义（真实浏览器对 disabled 控件不派发 click，此处仅防御）
    if (el.disabled === true) {
      window.__nuphus_rec_capture = { error: '点击了 disabled 元素，已忽略该次点击' };
      return;
    }
    var selector = genSelector(el);
    if (!selector) {
      window.__nuphus_rec_capture = { error: '无法为该元素生成稳定 selector（元素层级过深或缺少稳定标识）' };
      return;
    }
    var text = (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 120);
    window.__nuphus_rec_capture = {
      selector: selector,
      tag: el.tagName.toLowerCase(),
      text: text,
      href: el.tagName === 'A' ? el.getAttribute('href') : null,
      ts: Date.now()
    };
  }, true); // capture 阶段：先于页面自身 handler，且对 document 必达

  return JSON.stringify({ ok: true, already: false });
})()"#;

/// poll 读取脚本：有捕获则消费一次并清空（单步语义）；无捕获返回 false；
/// 注入标记丢失（页面刷新/整页导航）返回 need_reinject:true；error 亦消费。
const POLL_CAPTURE_JS: &str = r#"(() => {
  var c = window.__nuphus_rec_capture;
  if (!c) {
    if (window.__nuphus_rec_injected !== true) {
      return JSON.stringify({ captured: false, need_reinject: true });
    }
    return JSON.stringify({ captured: false });
  }
  window.__nuphus_rec_capture = null;
  if (c.error) return JSON.stringify({ captured: false, error: c.error });
  return JSON.stringify({
    captured: true,
    selector: c.selector,
    tag: c.tag,
    text: c.text || '',
    href: c.href || null,
    ts: c.ts || 0
  });
})()"#;

/// cancel 清理脚本：只清捕获结果，保留注入标记与监听（幂等；下一轮
/// start 幂等检测 already 直接复用同一监听）。
const CANCEL_CAPTURE_JS: &str = r#"(() => {
  if (window.__nuphus_rec_capture !== undefined) window.__nuphus_rec_capture = null;
  return JSON.stringify({ ok: true });
})()"#;

/// 页面 evaluate 统一以 JSON 文本返回，Rust 侧解析成 Value。
fn parse_eval_json(value: Value) -> Result<Value, String> {
    match value {
        Value::String(s) => {
            serde_json::from_str(&s).map_err(|e| format!("解析页面返回 JSON 失败: {e}"))
        }
        other => Ok(other),
    }
}

// ═════════════════════════════════════════════════════════════
// 命令
// ═════════════════════════════════════════════════════════════

/// 开始捕获网页点击：录制会话必须 active；浏览器未启动则以有界面(headed)
/// 模式启动 managed Chrome（用户可见窗口，可被真实点击）。
///
/// 返回：`{ ok: true, url, injected, already, note? }`
///   - url:      当前页地址（前端可据此向用户确认目标页）
///   - injected: true=本次完成注入；false=同 document 已注入（复用）
///   - already:  幂等信号（同 injected 互补，前端一般无需区分）
///   - note:     可选提示（iframe 等已知边界说明）
/// 错误（Err 中文）：
///   - 录制会话未初始化
///   - 浏览器启动失败（Chrome/Edge 未安装、profile 锁等）
///   - 尚未打开任何页面 / 当前页是 about:blank（提示先「打开网址」）
///   - 特殊页面（chrome://、扩展页、PDF 等）无法注入
#[tauri::command]
pub async fn rec_browser_capture_click_start() -> Result<Value, String> {
    crate::commands::rec::rec_session_ensure_active()?;

    // 1) 浏览器就绪：进程级单例 + headed 可见窗口。录制会话建立时全局闸门已
    //    拒绝 busy/workflow-active → 此处持锁期间无 agent 并行操作浏览器。
    let guard = nuphus::browser::get_or_launch(false).await.map_err(|e| {
        format!("浏览器启动失败: {e}（请确认已安装 Google Chrome / Microsoft Edge）")
    })?;
    let client = guard
        .as_ref()
        .ok_or_else(|| "浏览器客户端不可用".to_string())?;

    // 2) 页面检查：无页面 / 空白页直接给指引（避免对空 document 注入后用户困惑）。
    let url = match client.current_url().await {
        Ok(u) => u,
        Err(BrowserError::NoPage) | Err(BrowserError::NotStarted) => {
            return Err(
                "浏览器尚未打开任何页面：请先在浏览器中打开目标网页（可先录制/执行「打开网址」步骤），再开始捕获网页点击"
                    .to_string(),
            );
        }
        Err(e) => return Err(format!("获取当前页面地址失败: {e}")),
    };
    let trimmed = url.trim().to_string();
    if trimmed.is_empty() || trimmed == "about:blank" {
        return Err(
            "当前页面为空白页(about:blank)，无法捕获点击：请先在浏览器中打开目标网页再开始"
                .to_string(),
        );
    }

    // 3) 注入捕获脚本（幂等：同 document 已注入则返回 already）。
    let value = client.evaluate(INJECT_CAPTURE_JS).await.map_err(|e| {
        format!(
            "向当前页面注入点击捕获脚本失败: {e}。若当前是浏览器内部页面 \
             （如 chrome://、扩展页、PDF/开发者工具）则无法注入，请切换到普通网页后再试"
        )
    })?;
    let parsed = parse_eval_json(value)?;
    let already = parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        && parsed
            .get("already")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    tracing::info!(
        "[rec_browser] click capture injected on '{}' (already={})",
        trimmed,
        already
    );

    Ok(serde_json::json!({
        "ok": true,
        "url": trimmed,
        "injected": !already,
        "already": already,
        "note": "请在浏览器窗口中对目标元素点击；若目标在内嵌 iframe 中，当前版本无法捕获该点击（主 frame 注入边界），请改用网页内容获取或交给 WorkflowAgent 处理"
    }))
}

/// 轮询读取最近一次被捕获的点击（读后清空，单步只消费一次）。
/// 前端按 ~500ms 间隔轮询，超时上限与录制捕获 hook 对齐（60s）。
///
/// 返回：
///   - `{ captured: true, selector, tag, text, href, ts }`  捕获到点击
///   - `{ captured: false }`                                 尚无新点击
///   - `{ captured: false, error }`                          点击了不可定位/disabled 元素
///   - `{ captured: false, need_reinject: true }`            页面已刷新/整页导航，注入丢失
///     （前端可自动再次调 start 幂等重注入后继续提示用户点击）
/// 错误（Err 中文）：浏览器未启动（请先 start）/ 页面已关闭 / CDP 连接异常。
#[tauri::command]
pub async fn rec_browser_capture_click_poll() -> Result<Value, String> {
    let shared = nuphus::browser::shared_client();
    let guard = shared.lock().await;
    let client = guard.as_ref().ok_or_else(|| {
        "浏览器未启动：请先调用 rec_browser_capture_click_start 开始捕获".to_string()
    })?;

    let value = client
        .evaluate(POLL_CAPTURE_JS)
        .await
        .map_err(|e| format!("读取点击捕获状态失败: {e}（页面可能已关闭或浏览器连接异常）"))?;
    parse_eval_json(value)
}

/// 取消/清理当前点击捕获：清空 `window.__nuphus_rec_capture`（幂等），保留注入
/// 与监听——cancel 后再次 start 直接复用（already），无需重新注入。
/// 录制会话非 active 也返回 Ok（幂等清理）；浏览器不可用仅告警不报错。
#[tauri::command]
pub async fn rec_browser_capture_cancel() -> Result<(), String> {
    let shared = nuphus::browser::shared_client();
    let guard = shared.lock().await;
    if let Some(client) = guard.as_ref() {
        if let Err(e) = client.evaluate(CANCEL_CAPTURE_JS).await {
            tracing::warn!("[rec_browser] capture cancel evaluate failed (ignored): {e}");
        }
    }
    Ok(())
}
