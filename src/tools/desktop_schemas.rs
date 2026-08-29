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
                "鼠标操作。写操作(click/double_click/hover/scroll/move)传(x,y)并先激活窗口；position 只读返回光标位置。macOS 需辅助功能授权",
                json_props! {
                    "action" => obj!("type"="string","enum"=["click","double_click","hover","scroll","position","move"],"description"="click/double_click/hover/scroll/move 写操作需(x,y)；position 只读"),
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
                "拖拽鼠标起点→终点（验证码滑块等）。macOS 需辅助功能授权",
                json_props! {
                    "start_x" => obj!("type"="integer","description"="Start X coordinate"),
                    "start_y" => obj!("type"="integer","description"="Start Y coordinate"),
                    "end_x" => obj!("type"="integer","description"="End X coordinate"),
                    "end_y" => obj!("type"="integer","description"="End Y coordinate")
                },
                &["start_x","start_y","end_x","end_y"]),
            tool_def("desktop_input",
                "向窗口输入文本(UTF-8)，可附带后续按键。>500 字符用 clipboard。需先激活目标窗口",
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
                "列出可见窗口(hwnd/标题/位置)。macOS 需辅助功能授权",
                serde_json::json!({}),
                &[]),
            tool_def("desktop_window_activate",
                "激活窗口到前台(hwnd)。窗口操作前必须先激活，否则可能作用于错误窗口。macOS 需辅助功能授权",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list")
                },
                &["hwnd"]),
            tool_def("desktop_window_screenshot",
                "截取窗口截图存 BMP（hwnd 或 title 定位）。需先激活窗口",
                json_props! {
                    "title" => obj!("type"="string","description"="Window title substring to find"),
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list"),
                    "path" => obj!("type"="string","description"="Save path (always BMP)")
                },
                &[]),
            tool_def("desktop_window_move",
                "移动窗口到(x,y)(hwnd)。需先激活窗口",
                json_props! {
                    "hwnd" => obj!("type"="integer","description"="Window handle from windows_list"),
                    "x" => obj!("type"="integer","description"="New X position"),
                    "y" => obj!("type"="integer","description"="New Y position")
                },
                &["hwnd","x","y"]),
            tool_def("desktop_window_resize",
                "调整窗口大小(w,h)(hwnd)。需先激活窗口",
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
"AI 图像理解(布局/文字/图标)。传 prompt 定向分析，不传提取全部文字。⚠️坐标偏差大不可用于点击——用 perceive 取精确坐标",
                json_props! {
                    "image_path" => obj!("type"="string","description"="BMP 图片路径"),
                    "prompt" => obj!("type"="string","description"="定向分析提示（如\"分析UI布局结构\"），不传默认提取全部文字")
                },
                &["image_path"]),
            tool_def("desktop_perceive",
"本地 OCR+YOLO 元素定位，返回 rect{x,y,w,h} 与 center 点击坐标。点击必须用 center。OCR 文字可能有误，以 vision 为准",
                json_props! {
                    "image_path" => obj!("type"="string","description"="BMP 截图路径（来自 desktop_screenshot）")
                },
                &["image_path"]),
            tool_def("desktop_clipboard_clean",
                "清空剪贴板。粘贴敏感内容后必须调用防泄漏。仅清除用，勿读取",
                serde_json::json!({}),
                &[]),
            tool_def("desktop_clipboard_write",
                "写长文本(>500字符)到剪贴板粘贴。普通文本用 desktop_input。粘贴后必须 clean。禁止密码/敏感数据",
                json_props! {
                    "text" => obj!("type"="string","description"="Text to write")
                },
                &["text"]),

            // ═══ Vision / Locate tools ═══
            tool_def("desktop_find_image",
                "屏幕上找静态图片（模板匹配）。按原尺寸匹配、不缩放；支持 PNG/JPG/BMP/GIF。未命中返回最近候选+置信度+diagnostic。建议传 region 加速",
                json_props! {
                    "template_path" => obj!("type"="string","description"="模板图片路径，多个用 | 分隔"),
                    "region" => obj!("type"="object","description"="搜索区域{x,y,width,height}，推荐传以加速"),
                    "threshold" => obj!("type"="number","default"=0.9,"description"="相似度阈值 0-1，默认 0.9；匹配不上可调低")
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
                "区域找字（须传 region），需本地字库",
                json_props! {
                    "dict_name" => obj!("type"="string","description"="字库名（{name}.dict，可用 glob_search('**/*.dict') 列出）"),
                    "words" => obj!("type"="string","description"="查找文字，多个用 | 分隔，精确匹配"),
                    "region" => obj!("type"="object","description"="搜索区域{x,y,width,height}"),
                    "sim" => obj!("type"="number","default"=1.0,"description"="相似度 0-1，默认 1.0（精确）")
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
                "Text snapshot of visible interactive elements via AX tree: @N [role] \"name\". Use @N refs for click/type. Falls back to DOM traversal if AX unavailable.",
                json_props! {
                    "full" => obj!("type"="boolean","default"=false,"description"="Include hidden elements too"),
                    "selector" => obj!("type"="string","description"="Scope snapshot to this subtree")
                },
                &[]),
            tool_def("browser_exec",
                "Run multi-step batch script in ONE CDP round trip (form filling, multi-click). Helpers: h.click('@N'|'selector'), h.fill(sel, text), h.scroll(px), h.wait(ms), h.extract(sel), h.snapshot(). h.click/h.fill auto-wait up to 5s. Returns [{op, ref, success, detail}] per step. For nav/screenshot use browser_navigate/browser_screenshot.",
                json_props! {
                    "script" => obj!("type"="string","description"="JS using window.__nuphus helpers (alias 'h'). e.g. await h.click('@1'); await h.fill('@2', 'test@example.com'); await h.click('#submit');")
                },
                &["script"]),
            tool_def("browser_click",
                "Click element by CSS selector or ref ID (@N). Auto-waits for visibility (5s). Default JS click ignores overlays but lacks user activation; trusted=true sends real CDP mouse events (for autoplay-gated media / gesture-gated features).",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector or ref ID (e.g. @1, @e0, 'button')"),
                    "trusted" => obj!("type"="boolean","description"="Real trusted CDP mouse events at element center (user activation). For autoplay-gated media. Default false (JS click).")
                },
                &["selector"]),
            tool_def("browser_type",
                "Type text into input by CSS selector or @N ref. Auto-waits for visibility (5s).",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector or @N ref of input field"),
                    "text" => obj!("type"="string","description"="Text to type")
                },
                &["selector","text"]),
            tool_def("browser_press",
                "Press physical key or chord on focused element (click/type first to focus). Supports named keys, single chars, chords (Control+c, Shift+Tab, Meta+ArrowLeft). Does not verify DOM change (terminal/canvas may update outside DOM).",
                json_props! {
                    "key" => obj!("type"="string","minLength"=1,"description"="Key or chord, e.g. Enter, ArrowUp, Control+c"),
                    "snapshot" => obj!("type"="boolean","default"=false,"description"="Include post-key snapshot (default false; avoids disturbing terminal/canvas state)")
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
                "Wait for CSS selector to reach a state (up to timeout). Note: click/type already auto-wait 5s; use for custom states or longer delays.",
                json_props! {
                    "selector" => obj!("type"="string","description"="CSS selector to wait for"),
                    "timeout_ms" => obj!("type"="integer","default"=5000,"description"="Max wait time in ms"),
                    "state" => obj!("type"="string","enum"=["attached","visible","hidden"],"default"="attached","description"="attached=in DOM (default); visible=in DOM+visible; hidden=absent or hidden")
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
                "Import cookies from user's Chrome profile into current session. Optional domain filter.",
                json_props! {
                    "domain" => obj!("type"="string","description"="Optional domain filter (e.g. 'github.com')")
                },
                &[]),
            tool_def("browser_upload_file",
                "Upload a file to <input type=file>. Use @N ref or CSS selector.",
                json_props! {
                    "selector" => obj!("type"="string","description"="@N ref or CSS selector of file input"),
                    "file_path" => obj!("type"="string","description"="Absolute path to file on disk")
                },
                &["selector","file_path"]),
            tool_def("browser_drag_files",
                "Drag local files/dirs onto a browser element (native CDP drag). Unlike browser_upload_file, no input[type=file] needed.",
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