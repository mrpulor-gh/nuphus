//! desktop_schemas — 桌面 + 浏览器工具的 JSON Schema 定义
//!
//! 由 ToolRegistry::get_schemas() 渲染到 system prompt 的 <tools> 中。
//! 桌面工具通过 DesktopClient match 分发,不经过 ToolDef.executor;

use super::registry::ToolRegistry;

/// Helper macro to build a JSON object from key=value pairs.
macro_rules! obj {
    ($($k:literal = $v:expr),* $(,)?) => {{
        let mut m = serde_json::Map::new();
        $(m.insert($k.to_string(), serde_json::json!($v));)*
        serde_json::Value::Object(m)
    }};
}

/// Helper macro to build the properties object for tool parameters.
macro_rules! json_props {
    ($($k:literal => $v:expr),* $(,)?) => {{
        let mut m = serde_json::Map::new();
        $(m.insert($k.to_string(), $v);)*
        serde_json::Value::Object(m)
    }};
}

impl ToolRegistry {
    pub(super) fn get_desktop_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        let mut schemas = Vec::new();

        // Desktop 工具仅在 desktop_client 已连接时暴露
        let has_desktop = self
            .desktop_client
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if has_desktop {
            schemas.extend(self.desktop_tool_schemas());
        }

        // Browser 工具总是暴露（由 execute_browser_tool 惰性初始化）
        schemas.extend(self.browser_tool_schemas());

        schemas
    }

    fn desktop_tool_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        vec![
            // ═══ Desktop automation tools ═══
            tool_def("desktop_mouse",
                "鼠标操作：click/double_click/hover/scroll/move 需传入 (x,y)，操作前先移动光标到坐标再执行。position 为只读，返回当前光标 (x,y)，不移动光标。click 失败时用来判断是坐标偏差还是功能约束。所有写操作前需先通过 desktop_window_activate 激活目标窗口。macOS: 需先在 系统设置→隐私→辅助功能 授权",
                json_props! {
                    "action" => obj!("type"="string","enum"=["click","double_click","hover","scroll","position","move"],"description"="What to do. click/double_click/hover/scroll/move: write (needs x,y). position: read-only, returns {x,y}"),
                    "hwnd" => obj!("type"="integer","description"="Target window handle. Optional for all write-actions; skip for position."),
                    "x" => obj!("type"="integer","description"="X coordinate (click/double_click/hover/move)"),
                    "y" => obj!("type"="integer","description"="Y coordinate (click/double_click/hover/move)"),
                    "button" => obj!("type"="string","enum"=["left","right","middle"],"description"="Mouse button (click)"),
                    "clicks" => obj!("type"="integer","default"=1,"description"="Number of clicks (click)"),
                    "direction" => obj!("type"="string","enum"=["up","down"],"description"="Scroll direction (scroll)"),
                    "amount" => obj!("type"="integer","default"=3,"description"="Scroll ticks (scroll)")
                },
                &["action"]),
            tool_def("desktop_mouse_drag",
                "拖拽鼠标从起点到终点坐标（验证码滑块、滑块验证等场景）。macOS: 需先在 系统设置→隐私→辅助功能 授权",
                json_props! {
                    "start_x" => obj!("type"="integer","description"="Start X coordinate"),
                    "start_y" => obj!("type"="integer","description"="Start Y coordinate"),
                    "end_x" => obj!("type"="integer","description"="End X coordinate"),
                    "end_y" => obj!("type"="integer","description"="End Y coordinate")
                },
                &["start_x","start_y","end_x","end_y"]),
            tool_def("desktop_input",
                "向窗口输入文本（自动 UTF-8 编码），可选附带一个后续按键——原子操作。普通文本直接输入；超过 500 字符用 clipboard。操作前需先通过 desktop_window_activate 激活目标窗口。",
                json_props! {
                    "mode" => obj!("type"="string","enum"=["type","hotkey"],"description"="type: input text; hotkey: press keys only"),
                    "hwnd" => obj!("type"="integer","description"="Target window handle. Get from desktop_windows_list."),
                    "text" => obj!("type"="string","description"="Text to type (mode=type required)"),
                    "send" => obj!("type"="string","description"="Key to send after typing: \"enter\" (default), \"ctrl+enter\", \"tab\", or \"none\" to skip."),
                    "keys" => obj!("type"="array","items"=obj!("type"="string"),"description"="Key combo to press (mode=hotkey required). Single key: [\"enter\"],[\"f5\"],[\"esc\"]. Combo: [\"ctrl\",\"c\"],[\"alt\",\"tab\"].")
                },
                &["mode","hwnd"]),
            tool_def("desktop_screenshot",
                "全屏截图（支持 region 区域截图），保存为 BMP",
                json_props! {
                    "path" => obj!("type"="string","description"="保存路径（自动转为 .bmp）"),
                    "region" => obj!("type"="object","description"="裁剪区域 {x,y,width,height}，不传则全屏")
                },
                &[]),
            tool_def("desktop_screen_size",
                "获取屏幕分辨率 (宽 x 高)。",
                serde_json::json!({}),
                &[]),
            tool_def("desktop_windows_list",
                "列出所有可见操作系统窗口（hwnd/标题/位置）。macOS: 需先在 系统设置→隐私→辅助功能 中授权，否则返回空列表。",
                serde_json::json!({}),
                &[]),
            tool_def("desktop_window_activate",
                "激活窗口到前台（通过 hwnd）。窗口操作（截图/点击/输入/移动/调整大小）前必须先激活目标窗口，否则操作可能作用于错误窗口或失败。macOS: 需先在 系统设置→隐私→辅助功能 中授权。",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list")
                },
                &["hwnd"]),
            tool_def("desktop_window_screenshot",
                "截取指定窗口截图保存为 BMP（通过 hwnd 或 title 定位，至少提供一个）。操作前需先通过 desktop_window_activate 激活窗口。",
                json_props! {
                    "title" => obj!("type"="string","description"="Window title substring to find"),
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list"),
                    "path" => obj!("type"="string","description"="Save path (always BMP)")
                },
                &[]),
            tool_def("desktop_window_move",
                "移动窗口到指定坐标 (x,y)（通过 hwnd）。操作前需先通过 desktop_window_activate 激活窗口。",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list"),
                    "x" => obj!("type"="integer","description"="New X position"),
                    "y" => obj!("type"="integer","description"="New Y position")
                },
                &["hwnd","x","y"]),
            tool_def("desktop_window_resize",
                "调整窗口大小为 (w,h)（通过 hwnd）。操作前需先通过 desktop_window_activate 激活窗口。",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list"),
                    "width" => obj!("type"="integer","description"="New width in pixels"),
                    "height" => obj!("type"="integer","description"="New height in pixels")
                },
                &["hwnd","width","height"]),
            tool_def("desktop_window_info",
                "获取窗口详细信息（位置/大小/标题/可见性/进程/类/类型）（通过 hwnd）。",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list")
                },
                &["hwnd"]),
            tool_def("desktop_vision",
"AI 图像理解：将截图发送给视觉模型进行语义分析。识别界面布局、文字内容、图标功能。传 prompt 定向分析（如\"分析UI布局结构\"、\"识别所有图标功能\"），不传默认提取全部文字。⚠️ vision 返回的坐标偏差较大，不可用于点击——先 vision 理解全貌，再用 perceive 获取精确 center 坐标进行点击。",
                json_props! {
                    "image_path" => obj!("type"="string","description"="BMP 图片路径"),
                    "prompt" => obj!("type"="string","description"="定向分析提示（如\"分析UI布局结构\"、\"识别所有图标功能\"），不传默认提取全部文字")
                },
                &["image_path"]),
            tool_def("desktop_perceive",
"UI 元素定位：对截图执行本地 OCR+YOLO 检测，返回每个元素的精确坐标（rect）和 center 点击坐标。每个元素包含 rect{x,y,w,h} 和 center{x,y}（= x+w/2, y+h/2）。点击必须使用 center 坐标，不可用 rect.x/y（左上角）。先 vision 理解界面语义，再 perceive 拿坐标。注意：本地 OCR 文字可能有误，以 vision 识别的文字为准。",
                json_props! {
                    "image_path" => obj!("type"="string","description"="BMP 截图路径（来自 desktop_screenshot）")
                },
                &["image_path"]),
tool_def("desktop_clipboard_clean",
                "清空系统剪贴板。粘贴完敏感内容（密码/Token/验证码）后必须调用，防止残留泄漏。⚠️ 仅用于清除，不要用于读取剪贴板内容。",
                serde_json::json!({}),
                &[]),
            tool_def("desktop_clipboard_write",
                "写入长文本（>500 字符）到剪贴板用于粘贴。普通文本用 desktop_input 直接输入，无需剪贴板。粘贴后必须调用 desktop_clipboard_clean 清除残留。⚠️ 禁止用于密码/敏感数据。",
                json_props! {
                    "text" => obj!("type"="string","description"="Text to write")
                },
                &["text"]),

            // ═══ Vision / Locate tools ═══
            tool_def("desktop_find_image",
                "在屏幕上查找模板图片",
                json_props! {
                    "template_path" => obj!("type"="string","description"="Path to template BMP image. Multiple templates separated by |"),
                    "region" => obj!("type"="object","description"="Search region {x,y,width,height} for faster matching"),
                    "threshold" => obj!("type"="number","default"=0.9,"description"="Match threshold 0.0-1.0")
                },
                &["template_path"]),
            tool_def("desktop_find_color",
                "在屏幕上查找指定 RGB 颜色。",
                json_props! {
                    "color" => obj!("type"="string","description"="Target color: 'R,G,B' (e.g. '59,130,246'), hex '3B82F6', or '#3B82F6'. Supports delta: 'R,G,B,Dr,Dg,Db' or '3B82F6-0A0A0A'"),
                    "region" => obj!("type"="object","description"="Search region {x,y,width,height}"),
                    "direction" => obj!("type"="string","enum"=["left_top","right_top","left_bottom","right_bottom"],"default"="left_top","description"="Scan direction from corner")
                },
                &["color"]),
            tool_def("desktop_find_multi_color",
                "通过锚点颜色 + 偏移点颜色模式在屏幕上定位。",
                json_props! {
                    "anchor" => obj!("type"="string","description"="Anchor color: 'R,G,B' (e.g. '59,130,246'), hex '3B82F6', or '#3B82F6'"),
                    "offsets" => obj!("type"="string","description"="偏移点序列，格式: dx|dy|color,dx|dy|color,...  如 '1|0|FF0000,0|1|00FF00'。color 前加 ! 表示该点不应为此色"),
                    "region" => obj!("type"="object","description"="Search region {x,y,width,height}"),
                    "min_match_ratio" => obj!("type"="number","default"=0.8,"description"="Min ratio of matching offset points (0.0-1.0)"),
                    "direction" => obj!("type"="string","enum"=["left_top","right_top","left_bottom","right_bottom"],"default"="left_top","description"="Scan direction from corner")
                },
                &["anchor","offsets"]),

            // ═══ Dict OCR / Find Text ═══
            tool_def("desktop_find_text",
                "精准区域找字（必须传 region 限定搜索区域），需用户自定义添加本地字库定义文字。",
                json_props! {
                    "dict_name" => obj!("type"="string","description"="字库名称（不带后缀，对应字库目录下的 {name}.dict 文件）。可用 glob_search('**/*.dict') 列出已有字库。"),
                    "words" => obj!("type"="string","description"="要查找的文字。多个词用 | 分隔，精确匹配。如 系统|文件 表示同时查找系统和文件"),
                    "region" => obj!("type"="object","description"="搜索区域 {x,y,width,height}，不传则全屏"),
                    "sim" => obj!("type"="number","default"=1.0,"description"="相似度阈值 0.0-1.0，默认 1.0（精确匹配）")
                },
                &["dict_name","words","region"]),
        ]
    }

    fn browser_tool_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        vec![
            // ═══ Browser automation tools ═══
            tool_def("browser_navigate",
                "Open URL in browser",
                json_props! {
                    "url" => obj!("type"="string","description"="URL to navigate to")
                },
                &["url"]),
            tool_def("browser_snapshot",
                "Get text snapshot of visible interactive elements using Chrome Accessibility Tree. Outputs @N [role] \"name\" format (e.g. @1 [button] \"Submit\"). Falls back to DOM traversal if AX tree unavailable. Use @N refs for click/type.",
                json_props! {
                    "full" => obj!("type"="boolean","default"=false,"description"="Include hidden elements too"),
                    "selector" => obj!("type"="string","description"="CSS selector to scope snapshot (e.g. '#quiz', '.main-content'). Only elements within this subtree are numbered.")
                },
                &[]),
            tool_def("browser_exec",
                "Execute a multi-step batch script in ONE CDP round trip. Use for form filling, multi-click workflows. Script uses `h.click('@N'|'selector')`, `h.fill('@N'|'selector', text)`, `h.scroll(px)`, `h.wait(ms)`, `h.extract('selector')`, `h.snapshot()`. h.click/h.fill auto-wait for the element to appear and become visible (up to 5s, optional per-call timeoutMs). Returns [{op, ref, success, detail}] per step. For navigation/screenshot, still use browser_navigate/browser_screenshot.",
                json_props! {
                    "script" => obj!("type"="string","description"="JS script using window.__nuphus helpers (aliased as 'h'). Example: await h.click('@1'); await h.fill('@2', 'test@example.com'); await h.click('#submit');")
                },
                &["script"]),
            tool_def("browser_click",
                "Click element by CSS selector or ref ID from snapshot (e.g. @1, @e0, 'button'). CSS selector path auto-waits for the element to appear and become visible (up to 5s) before clicking. Default clicks are JS-synthesized (reliable, ignore overlays) but do NOT produce user activation; pass trusted=true to dispatch real CDP mouse events (isTrusted=true) instead — required to unlock autoplay-gated audio/video playback and other gesture-gated features.",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector or ref ID (e.g. @1, @e0, 'button')"),
                    "trusted" => obj!("type"="boolean","description"="Dispatch real trusted CDP mouse events at the element's center (produces user activation). Use for autoplay-gated media playback and gesture-gated features. Default false (JS click).")
                },
                &["selector"]),
            tool_def("browser_type",
                "Type text into input field by CSS selector or ref ID from snapshot. CSS selector path auto-waits for the element to appear and become visible (up to 5s) before typing.",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector or ref ID of input field (e.g. @1, @e0)"),
                    "text" => obj!("type"="string","description"="Text to type")
                },
                &["selector","text"]),
            tool_def("browser_press",
                "Press a trusted physical keyboard key or chord on the currently focused page element. Use browser_click or browser_type first when a specific element needs focus. Supports named keys (Enter, Tab, Escape, ArrowUp, PageDown, F1, Space), single US-keyboard characters, and modifier chords such as Control+c, Shift+Tab, or Meta+ArrowLeft. Does not verify a DOM change because terminal/canvas handlers may update outside the DOM.",
                json_props! {
                    "key" => obj!("type"="string","minLength"=1,"description"="Key or chord to press, e.g. Enter, ArrowUp, Control+c, Shift+Tab, Meta+ArrowLeft"),
                    "snapshot" => obj!("type"="boolean","default"=false,"description"="Include a post-key page snapshot. Defaults to false so terminal/canvas state and transient UI are not disturbed.")
                },
                &["key"]),
            tool_def("browser_scroll",
                "Scroll page up/down by N pixels.",
                json_props! {
                    "direction" => obj!("type"="string","enum"=["up","down"],"description"="Scroll direction"),
                    "amount" => obj!("type"="integer","default"=500,"description"="Pixels to scroll")
                },
                &["direction"]),
            tool_def("browser_extract",
                "Extract readable text from current page (strips nav/ads).",
                json_props! {
                    "max_chars" => obj!("type"="integer","default"=8000,"description"="Max characters to extract")
                },
                &[]),
            tool_def("browser_screenshot",
                "Screenshot the current browser page.",
                json_props! {
                    "path" => obj!("type"="string","description"="Save path")
                },
                &[]),
            tool_def("browser_close",
                "Close browser and free resources.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_evaluate",
                "Execute arbitrary JavaScript in the current page.",
                json_props! {
                    "script" => obj!("type"="string","description"="JavaScript code")
                },
                &["script"]),
            tool_def("browser_back",
                "Navigate back in browser history.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_forward",
                "Navigate forward in browser history.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_wait_for",
                "Wait for CSS selector to reach the given state on page (up to timeout). Note: browser_click/browser_type CSS path already auto-waits (presence+visible, 5s), so explicit waits are usually only needed for custom states or longer delays.",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector to wait for"),
                    "timeout_ms" => obj!("type"="integer","default"=5000,"description"="Max wait time in ms"),
                    "state" => obj!("type"="string","enum"=["attached","visible","hidden"],"default"="attached","description"="Target state: attached=in DOM (default), visible=in DOM and visible, hidden=absent or not visible")
                },
                &["selector"]),
            tool_def("browser_cookies_get",
                "Get all cookies for the current page.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_cookies_set",
                "Set a cookie for the current domain.",
                json_props! {
                    "name" => obj!("type"="string","description"="Cookie name"),
                    "value" => obj!("type"="string","description"="Cookie value"),
                    "domain" => obj!("type"="string","description"="Domain"),
                    "path" => obj!("type"="string","description"="Path")
                },
                &["name","value"]),
            tool_def("browser_import_cookies",
                "Import cookies from user's Chrome profile into current browser session. Reads Chrome's SQLite Cookies DB, decrypts via DPAPI (Windows), and injects via CDP Network.setCookie. Optional domain filter.",
                json_props! {
                    "domain" => obj!("type"="string","description"="Optional domain filter (e.g. 'github.com')")
                },
                &[]),
            tool_def("browser_upload_file",
                "Upload a file to a <input type=file> element. Reads file from disk, base64-encodes, sets via JS DataTransfer. Use @N ref or CSS selector to target file input.",
                json_props! {
                    "selector" => obj!("type"="string","description"="@N ref or CSS selector of file input"),
                    "file_path" => obj!("type"="string","description"="Absolute path to file on disk")
                },
                &["selector","file_path"]),
            tool_def("browser_drag_files",
                "Drag one or more existing local files or directories onto a browser element using native Chrome DevTools drag events. Unlike browser_upload_file, this does not require an input[type=file] element and does not base64-encode file contents.",
                json_props! {
                    "selector" => obj!("type"="string","minLength"=1,"description"="CSS selector or ref ID of the drop target (e.g. @1, @e0, '.explorer-viewlet')"),
                    "ref" => obj!("type"="string","minLength"=1,"description"="Ref ID from snapshot; alias of selector — provide either one"),
                    "file_paths" => obj!("type"="array","items"=obj!("type"="string"),"minItems"=1,"description"="Absolute paths of existing local files or directories to drag")
                },
                &["file_paths"]),
            tool_def("browser_list_downloads",
                "List files in the browser download directory.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_new_tab",
                "Open new browser tab",
                json_props! {
                    "url" => obj!("type"="string","description"="URL to open in new tab")
                },
                &[]),
            tool_def("browser_list_tabs",
                "List all open tabs with IDs, URLs, and titles.",
                serde_json::json!({}),
                &[]),
            tool_def("browser_switch_tab",
                "Switch focus to tab by index.",
                json_props! {
                    "index" => obj!("type"="integer","description"="Tab index from list_tabs")
                },
                &["index"]),
        ]
    }
}

fn tool_def(
    name: &str,
    description: &str,
    properties: serde_json::Value,
    required: &[&str],
) -> crate::api::ToolDefinition {
    let required: Vec<String> = required.iter().map(|s| s.to_string()).collect();
    crate::api::ToolDefinition {
        tool_type: "function".to_string(),
        function: crate::api::FunctionDefinition {
            name: name.to_string(),
            description: Some(description.to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
            }),
            permission: None,
        },
    }
}
