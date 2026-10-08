//! preview:// 自定义协议 — 沙箱化本地文件运行底座
//!
//! 设计定位（大王定调）：功能底座，不绑死 HTML。通用能力 = 路径 → 文件字节
//! + mime 推断 + 独立安全头。任何前端组件（当前 PreviewOverlay，未来画廊 /
//!   音视频面板等）经 convertFileSrc(path, 'preview') 即可获得可运行文档。
//!
//! 安全模型（与主应用 CSP 隔离）：
//! - iframe `sandbox` 属性：无相同源泄漏面，预览内容碰不到主应用与系统
//! - 响应头 `Content-Security-Policy: sandbox ...`：协议层兜底沙箱——即使
//!   前端 iframe 属性被遗漏，文档仍被强制沙箱化（纵深防御）
//! - 独立宽松 CSP 只作用于 preview 响应本身：agent 产出的 HTML 游戏/交互
//!   demo 可执行内联脚本、引用 CDN 引擎与同目录资源，主应用 CSP 一字不动

use std::path::PathBuf;

use tauri::http::{header, Request, Response, StatusCode};

/// 单文件预览大小上限：覆盖大型游戏/音视频产物，防误读超大文件耗尽内存
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;

/// HTML 预览的「画面标注」overlay 脚本（自包含单文件常量）。
///
/// 设计约束（与 serve() 的注入策略配套，勿单独改动）：
/// - **仅 HTML 按需追加**：插在文档最后一个 `</body>` 之前，无该标签的片段型文档
///   追加到末尾；非 HTML 文件零注入。判定见 `is_html_document`。
/// - **失败静默降级**：脚本自身异常只 `console.error`，绝不影响预览本身；父窗口
///   （PreviewOverlay）以「2s 未收到 ready 握手」兜底提示「标注器未就绪」。
/// - **协议层禁网**：只用 DOM API + postMessage，禁止 fetch/XHR/信标。
/// - **shadow DOM 挂载**：宿主页面样式污染不到 overlay，overlay 样式也不泄漏进宿主。
///
/// 载荷协议（顶层 type / 字段名已钉死，P2「标注进对话」直接依赖，勿改名）：
/// `{ type: 'nuphus:annotations', file, annotations: [{ css_selector,
///    outer_html_snippet, rect: { x, y, w, h }, dpr, comment }] }`
/// 另有握手消息 `{ type: 'nuphus:annotator-ready' }` 供父窗口判定就绪。
///
/// P3 体验打磨（既有字段零改，全部为追加）：
/// - 区域框选：标注模式下拖拽画矩形 → 松手弹批注 → 区域条 **附加** `region: {x,y,w,h}`
///   （视口坐标），selector 仍取区域中心点命中元素；区域视觉 = 琥珀虚线框 + 编号
///   （元素 = 蓝圆标，视觉区分）。
/// - 多标记管理：标注模式侧边列表（序号 + 批注摘要 + ◎ 定位闪烁 + × 单条删除，
///   删除后序号重排）。
/// - 持久化：markers 每次变更向父窗口广播同型快照（父窗口落 localStorage）；
///   父窗口重开预览时下发 `{ type: 'nuphus:annotations-restore', annotations }`
///   （新增下行指令，source 必须 === window.parent，与父窗口侧铁律对称）。
///
/// P4 标注 UI 重构（大王定案四条铁律，勿推翻）：
/// - **iframe = 纯渲染层，默认零常驻 UI 且完全 inert**：胶囊「标注/发送N」、顶部提示条、
///   侧边标记列表、浮动批注框全部移除（编辑入口与修改列表改由父窗口承载）。
///   非编辑态下所有 document 监听器第一行即返回，宿主页面的链接/按钮保持原生可用。
/// - **编辑态拦截走 capture 阶段 preventDefault**（不是 CSS pointer-events——那挡不住跳转）：
///   标注模式下吃下页面自身的点击与拖拽起点，避免「点标注却跳转」。
/// - **高亮一律 overlay 矩形，宿主 DOM 零写入**：P3 用内联 `el.style.outline` 画高亮、
///   清除时写回空串，宿主持久残留 `style=""`（实测 outer_html_snippet 带 `style=""`，
///   宿主若用 `[style]` 选择器会被误判）。本版只读宿主的 `getBoundingClientRect`。
/// - **单击即标注重定语义**：同元素再次点击 = 取消该标记；区域框选照旧。
///
/// 下行指令（新增 type，协议面**仅增不改**；source 必须 === window.parent）：
///
/// - `nuphus:annotate-mode` { on } —— 编辑态开关；off 时清视觉但保留 markers
///   （持久化是父窗口职责，iframe 不碰 localStorage）。
/// - `nuphus:annotate-focus` { selector, seq } —— 定位闪烁（父窗口列表项的 ◎）。
/// - `nuphus:annotate-comment` { index, comment } —— 父窗口列表内批注回写。
/// - `nuphus:annotate-remove` { index } —— 父窗口列表单条删除。
///
/// 上行除既有 `nuphus:annotations` / `nuphus:annotator-ready` 外新增
/// `nuphus:annotate-exit`（iframe 内按 Esc 请求父窗口退出编辑态——焦点在 iframe 内，
/// 父窗口收不到 keydown，不加这条用户会被困在编辑态）。
// pub(crate)：commands/browser.rs 的 content webview 复用同一份标注脚本
// （initialization_script 注入），禁止手抄副本——脚本真源只有这一处。
/// 标注 overlay 脚本：唯一真源在 `nuphus-browser`（CDP 与 preview 两个宿主共用）。
/// 生成物，真源是 `frontend/tools/overlay-script.src.js`；
/// `frontend/tools/sync-overlay-script.mjs` 负责搬运与 `--check` 防漂移。
pub(crate) use nuphus::browser::ANNOTATION_OVERLAY_SCRIPT;

/// 注册 preview 协议到 Builder（main.rs Builder 链首调用）
pub fn register<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol("preview", move |_ctx, request, responder| {
        // 文件 IO 移出协议回调线程，避免大文件阻塞事件循环
        tauri::async_runtime::spawn_blocking(move || {
            responder.respond(serve(request));
        });
    })
}

fn serve(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    // convertFileSrc 产物：Windows http://preview.localhost/<percent-encoded>
    //            Unix  preview://localhost/<percent-encoded>——path 段结构一致
    let raw = request.uri().path().trim_start_matches('/');
    let decoded = percent_decode(raw);
    let path = PathBuf::from(&decoded);

    if decoded.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "缺少文件路径");
    }
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &format!("无法读取文件: {e}")),
    };
    if !meta.is_file() {
        return error_response(StatusCode::BAD_REQUEST, "路径不是文件");
    }
    if meta.len() > MAX_PREVIEW_BYTES {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "文件超过预览大小上限（64 MB）",
        );
    }

    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = guess_mime(&path);
            // 标注 overlay：仅 HTML 注入（插在最后一个 </body> 前，无该标签则追加
            // 末尾）。按需追加、失败静默降级——注入异常不能影响预览本身。
            let bytes = if is_html_document(&path, mime) {
                inject_annotation_overlay(bytes)
            } else {
                bytes
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                // 文档级沙箱兜底 + 资源自由加载（内联脚本/CDN/同目录相对引用）
                .header(
                    "Content-Security-Policy",
                    "sandbox allow-scripts allow-same-origin allow-pointer-lock allow-modals \
                     allow-forms; default-src * 'unsafe-inline' 'unsafe-eval' data: blob:; \
                     img-src * data: blob:; media-src * data: blob:; font-src * data:; \
                     connect-src * data: blob:",
                )
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(bytes)
                .unwrap_or_else(|_| {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "响应构建失败")
                })
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("读取失败: {e}")),
    }
}

fn error_response(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_else(|_| Response::builder().status(status).body(Vec::new()).unwrap())
}

/// 手写 percent-decode（避免为单一用途引入 url crate）
/// 注意：不做 `+` → 空格转换——那是 form 编码规则，路径中的 `+`（如 C++ 目录）必须原样保留
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 扩展名 → mime。覆盖网页游戏/交互 demo 常见资源类型，未知类型走
/// octet-stream（浏览器下载或忽略，不 crash）
fn guess_mime(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "txt" | "md" | "log" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

/// HTML 判定（双保险）：mime 以 `text/html` 开头，或扩展名为 html/htm（大小写不敏感）。
/// 命中任一即注入——mime 推断漏判或扩展名缺失时另一条路径兜底。
fn is_html_document(path: &std::path::Path, mime: &str) -> bool {
    mime.starts_with("text/html")
        || path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

/// 定位文档中最后一个 `</body>`（大小写不敏感）的起始偏移。
fn find_last_close_body(bytes: &[u8]) -> Option<usize> {
    const NEEDLE: &[u8] = b"</body>";
    if bytes.len() < NEEDLE.len() {
        return None;
    }
    (0..=bytes.len() - NEEDLE.len())
        .rev()
        .find(|&i| bytes[i..i + NEEDLE.len()].eq_ignore_ascii_case(NEEDLE))
}

/// 把标注 overlay 脚本注入 HTML 字节流：插在最后一个 `</body>` 之前，没有该标签的
/// 片段型文档直接追加到末尾。纯字节操作，不解析 DOM，也不做可能失败的转换——
/// 「注入不坏预览」优先于「注入得漂亮」。
fn inject_annotation_overlay(bytes: Vec<u8>) -> Vec<u8> {
    let script = ANNOTATION_OVERLAY_SCRIPT.as_bytes();
    match find_last_close_body(&bytes) {
        Some(pos) => {
            let mut out = Vec::with_capacity(bytes.len() + script.len());
            out.extend_from_slice(&bytes[..pos]);
            out.extend_from_slice(script);
            out.extend_from_slice(&bytes[pos..]);
            out
        }
        None => {
            let mut out = bytes;
            out.extend_from_slice(script);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn percent_decode_windows_path() {
        assert_eq!(
            percent_decode("C%3A%5CUsers%5Cgame.html"),
            "C:\\Users\\game.html"
        );
    }

    #[test]
    fn percent_decode_unix_path() {
        assert_eq!(
            percent_decode("%2FUsers%2Fme%2Findex.html"),
            "/Users/me/index.html"
        );
    }

    #[test]
    fn percent_decode_preserves_plus() {
        assert_eq!(
            percent_decode("C%3A%5C C%2B%2B%5Cindex.html"),
            "C:\\ C++\\index.html"
        );
    }

    #[test]
    fn percent_decode_invalid_escape_kept() {
        assert_eq!(percent_decode("100%zz"), "100%zz");
    }

    #[test]
    fn inject_overlay_before_close_body() {
        let html = "<html><body><h1>hi</h1></body></html>".as_bytes().to_vec();
        let text = String::from_utf8(inject_annotation_overlay(html)).unwrap();
        // 原内容完整保留，脚本插在最后一个 </body> 之前、文档尾部结构不被破坏
        assert!(text.starts_with("<html><body><h1>hi</h1>"));
        assert!(text.trim_end().ends_with("</body></html>"));
        let script_pos = text.find("<script").expect("注入 <script> 起始标签");
        let body_pos = text.rfind("</body>").expect("原 </body> 保留");
        assert!(script_pos < body_pos, "脚本必须插在 </body> 之前");
    }

    #[test]
    fn inject_overlay_appends_without_close_body() {
        let html = "<html><h1>fragment</h1>".as_bytes().to_vec();
        let text = String::from_utf8(inject_annotation_overlay(html)).unwrap();
        assert!(text.starts_with("<html><h1>fragment</h1>"));
        assert!(
            text.trim_end().ends_with("</script>"),
            "无 body 标签时追加到末尾"
        );
    }

    #[test]
    fn inject_overlay_close_body_case_insensitive() {
        let html = "<HTML><BODY>x</BODY></HTML>".as_bytes().to_vec();
        let text = String::from_utf8(inject_annotation_overlay(html)).unwrap();
        let script_pos = text.find("<script").unwrap();
        let body_pos = text.to_ascii_lowercase().rfind("</body>").unwrap();
        assert!(script_pos < body_pos);
    }

    #[test]
    fn html_detection_gates_injection() {
        // HTML 双路径判定：扩展名与 mime 任一命中即注入（大小写不敏感）
        let html = Path::new("C:/a/game.html");
        let htm = Path::new("C:/a/demo.HTM");
        let noext = Path::new("C:/a/noext");
        assert!(is_html_document(html, guess_mime(html)));
        assert!(is_html_document(htm, guess_mime(htm)));
        assert!(is_html_document(noext, "text/html; charset=utf-8"));
        // 非 HTML 零注入：mime 与扩展名两条路径都不能误判
        for p in [
            "C:/a/app.js",
            "C:/a/style.css",
            "C:/a/game.wasm",
            "C:/a/pic.png",
            "C:/a/data.json",
        ] {
            let path = Path::new(p);
            let mime = guess_mime(path);
            assert!(!is_html_document(path, mime), "{p} 不应判定为 HTML");
        }
    }

    #[test]
    fn injected_script_pins_protocol_and_forbids_network() {
        // P2「标注进对话」依赖的协议标识钉在脚本常量里，防误改
        for pinned in [
            "nuphus:annotations",
            "nuphus:annotator-ready",
            "css_selector",
            "outer_html_snippet",
            "dpr",
            "window.parent.postMessage",
        ] {
            assert!(
                ANNOTATION_OVERLAY_SCRIPT.contains(pinned),
                "脚本必须包含 {pinned}"
            );
        }
        // P3 体验能力锚点（零改既有字段）：
        // - 区域框选：region 附加字段 + elementFromPoint 中心点命中
        // - 多标记管理：删除 / 定位闪烁（P4 起 UI 迁至父窗口，视觉走 overlay 浮层）
        // - 持久化：下行恢复指令 nuphus:annotations-restore + source 铁律
        for p3_pinned in [
            "region",
            "elementFromPoint",
            "nuphus:annotations-restore",
            "event.source !== window.parent",
            "regionbox",
        ] {
            assert!(
                ANNOTATION_OVERLAY_SCRIPT.contains(p3_pinned),
                "脚本必须包含 P3 锚点 {p3_pinned}"
            );
        }
        // 协议层禁网：overlay 只用 DOM + postMessage
        for banned in [
            "fetch(",
            "XMLHttpRequest",
            "sendBeacon",
            "import(",
            "WebSocket",
        ] {
            assert!(
                !ANNOTATION_OVERLAY_SCRIPT.contains(banned),
                "脚本不得出现 {banned}"
            );
        }
    }

    /// 回归防线：脚本字符串的**词法平衡**。
    ///
    /// 2026-10-08 实机报障「标注器未就绪」：三元表达式 else 分支误写 `'''`
    /// （三个单引号），整个脚本语法错误 → 浏览器拒绝执行 → 连握手都发不出，
    /// 而当时全部测试仍绿——因为 cargo test 只做子串断言、前端 vitest 从不
    /// 执行这段 Rust 内嵌 JS。语法错误能溜过所有既有断言，必须独立设防。
    ///
    /// 这里不解析 JS（无 JS 引擎），而是断言「单引号/双引号/花括号/圆括号/
    /// 方括号计数为偶数」+「不得出现连续三个以上单引号」——足以拦住
    /// `'''` / `""""` / 缺右括号这类词法级错误（真正的解析器级校验由
    /// frontend 的 PreviewOverlay.annotations.test.ts 用 new Function 兜）。
    #[test]
    fn overlay_script_lexical_balance_is_guarded() {
        let s = ANNOTATION_OVERLAY_SCRIPT;
        for (open, close, name) in [
            ('\'', '\'', "单引号"),
            ('"', '"', "双引号"),
            ('{', '}', "花括号"),
            ('(', ')', "圆括号"),
            ('[', ']', "方括号"),
        ] {
            let count = s.chars().filter(|c| *c == open).count();
            let close_count = s.chars().filter(|c| *c == close).count();
            assert_eq!(
                        count, close_count,
                        "{name} 不配对：{open} ×{count} vs {close} ×{close_count}（脚本词法已破，页面内将整体不执行）"
                    );
        }
        // 连续三个以上同类引号 = 词法必错（`'''` 曾真实发生过）
        for (q, name) in [('\'', "单引号"), ('"', "双引号")] {
            let triple: String = std::iter::repeat(q).take(3).collect();
            assert!(
                !s.contains(&triple),
                "脚本出现连续三个{name}（{triple}）——必然语法错误"
            );
        }
    }

    /// P4 架构铁律（大王定案）钉死在脚本常量里，防回退。
    ///
    /// 1. **iframe 零常驻 UI**：胶囊 / 提示条 / 侧边列表 / 批注框的 DOM 与文案全部消失，
    ///    编辑入口与修改列表改由父窗口承载。
    /// 2. **宿主 DOM 零写入**：P3 的内联 `el.style.outline` 清除时写回空串，宿主持久
    ///    残留 `style=""`（实测 outer_html_snippet 带 `style=""`，宿主用 `[style]`
    ///    选择器会被误判）。高亮改为 overlay 矩形后，脚本不得再碰宿主任何属性。
    /// 3. **默认 inert**：`annotating` 初值 false，非编辑态不拦事件不显形。
    #[test]
    fn p4_overlay_has_no_resident_ui_and_never_writes_host_dom() {
        let s = ANNOTATION_OVERLAY_SCRIPT;

        // 铁律一：iframe 侧不得再有任何常驻 UI 的痕迹
        for banned in [
            "capsule",
            "mgr-item",
            "mgr-empty",
            "class=\"hint\"",
            "class=\"comment\"",
            "data-toggle",
            "data-send",
            "openComment",
            "closeComment",
            "renderList",
            "openRegionComment",
            "textarea",
        ] {
            assert!(!s.contains(banned), "iframe 侧不得残留常驻 UI：{banned}");
        }

        // 铁律二：宿主元素属性零写入。
        // 注意只禁「写宿主」的模式：overlay 自己的节点用 classList/className 驱动视觉是
        // 允许的（shadow 内），buildSelector 读 node.classList 生成 selector 也是只读。
        for banned in [
            "style.outline",
            "__nuphusOutline",
            "setAttribute(",
            "el.classList",
            "marker.el.style",
            "el.className",
        ] {
            assert!(
                !s.contains(banned),
                "脚本不得写宿主 DOM（{banned}）——高亮走 overlay 矩形"
            );
        }

        // 铁律三：默认非编辑态（inert）。setHighlight 之外必须有 annotating 闸门。
        assert!(
            s.contains("var annotating = false"),
            "annotating 必须默认 false（非编辑态 iframe 完全 inert）"
        );
        for gated in [
            "if (!annotating) return",
            "if (!annotating || inOverlay(event)) return",
        ] {
            assert!(
                s.contains(gated),
                "document 监听器必须带编辑态闸门：{gated}"
            );
        }
    }

    /// P4 下行/上行指令锚点：协议面**仅增不改**，新增 type 钉死防误改。
    #[test]
    fn p4_annotation_instructions_are_pinned() {
        let s = ANNOTATION_OVERLAY_SCRIPT;
        // 下行（父→iframe，source 铁律已在上一用例钉住）
        for pinned in [
            "nuphus:annotate-mode",
            "nuphus:annotate-focus",
            "nuphus:annotate-comment",
            "nuphus:annotate-remove",
            "nuphus:annotate-exit",
            "highlightrect",
        ] {
            assert!(s.contains(pinned), "脚本必须包含 P4 指令锚点 {pinned}");
        }
        // 上行快照与握手仍在（字段零改）
        for pinned in [
            "type: 'nuphus:annotations'",
            "type: 'nuphus:annotator-ready'",
        ] {
            assert!(s.contains(pinned), "上行协议被改动：{pinned}");
        }
    }
}
