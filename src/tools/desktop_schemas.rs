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
        // 自动化开关关闭（ExecAgent）→ desktop_* 与 browser_* 一个都不暴露。
        // 注意 browser 在下方是无条件附加的，此处必须在最前面拦下。
        if !self.automation_tools_enabled {
            return Vec::new();
        }
        let mut schemas = Vec::new();

        // Accessibility/UIA semantic tools are independent of the legacy
        // coordinate/OCR DesktopClient path.
        if self.semantic_desktop.is_some() {
            schemas.extend(self.semantic_desktop_tool_schemas());
        }

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

    fn semantic_desktop_tool_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        let mut schemas = vec![
            tool_def(
                "desktop_targets_list",
                "查询本机运行窗口和已登记应用，返回 app_ref/window_ref 与 is_self。先依据用户任务选择目标；提交任务时前台通常是 Nuphus，不代表它是任务目标。",
                json_props! { "query" => obj!("type"="string","description"="可选应用名称或窗口标题查询"),
                    "cursor" => obj!("type"="integer","minimum"=0,"description"="下一页使用返回的 next_cursor 并保持相同 query") },
                &[],
            ),
            tool_def(
                "desktop_target_bind",
                "绑定查询返回的应用/窗口；本地必要时启动、还原并激活。多个窗口返回候选供选择。成功返回 target_token，后续观察传入此令牌。不接受脚本或任意启动路径。",
                json_props! {
                    "app_ref" => obj!("type"="string"),
                    "window_ref" => obj!("type"="string","description"="多窗口时选择列表返回的引用")
                },
                &["app_ref"],
            ),
            tool_def(
                "desktop_semantic_observe",
                "读取绑定目标的 UIA/Accessibility 元素，返回完整 JSON 候选页；未传 target_token 兼容观察前台。翻页传同次 observation_token 与 next_cursor，不重新采集。详情和可保存 workflow_step 使用 desktop_semantic_candidate 查询。普通模式首选；候选不足可查看菜单或局部区域，不等于原生不可用。",
                json_props! {
                    "goal" => obj!("type"="string","description"="当前要推进的桌面任务；仅用于构造和说明有界候选动作"),
                    "target_token" => obj!("type"="string","description"="desktop_target_bind 返回的任务目标令牌"),
                    "scope" => obj!("type"="string","enum"=["window","menu"],"description"="默认窗口内容区；menu 按需读取菜单"),
                    "subtree_id" => obj!("type"="string","description"="本地观察返回的元素/区域 ID；不接受编造的选择器"),
                    "observation_token" => obj!("type"="string","description"="翻页时传入；此时不改变目标或观察范围"),
                    "cursor" => obj!("type"="integer","minimum"=0)
                    ,"view" => obj!("type"="string","enum"=["candidates","regions"],"description"="同一观察可分页查看候选或可深入查询的区域")
                },
                &[],
            ),
            tool_def(
                "desktop_semantic_candidate",
                "只读查询同一次观察的候选详情和可保存 workflow_step。保存时使用返回的稳定步骤，不保存候选 ID 或 token。",
                json_props! {
                    "observation_token" => obj!("type"="string"),
                    "candidate_id" => obj!("type"="string")
                },
                &["observation_token", "candidate_id"],
            ),
            tool_def(
                "desktop_semantic_execute",
                "执行 desktop_semantic_observe 最近一次返回的一个 candidate_id。必须回传同次 observation_token；SetValue 候选可附带 value，该文本只交给本地执行器且不会发送给增强判断模型。执行前会重新读取 UI 并拒绝过期动作，不得传坐标、选择器或脚本。",
                json_props! {
                    "observation_token" => obj!("type"="string","description"="最近一次语义观察返回的不可预测短期令牌"),
                    "candidate_id" => obj!("type"="string","description"="最近一次语义观察返回的候选动作 ID"),
                    "value" => obj!("type"="string","description"="仅用于 SetValue 候选的本地文本；保持原始空白，不会发送给增强判断模型","maxLength"=16384)
                },
                &["observation_token", "candidate_id"],
            ),
            tool_def(
                "desktop_semantic_action",
                "执行已保存工作流中的稳定 UIA 语义动作。运行时重新读取当前前台应用并解析 locator，不依赖临时 observation_token、candidate_id、坐标、选择器或脚本；SetValue 可附带 value。",
                json_props! {
                    "locator" => obj!(
                        "type"="object",
                        "description"="由 desktop_semantic_observe 的 workflow_step 返回并原样保存的稳定语义定位器",
                        "properties"=json_props! {
                            "app_id" => obj!("type"="string"),
                            "window_id" => obj!("type"="string","description"="本地派生的稳定窗口身份；存在时优先于标题提示"),
                            "window_title" => obj!("type"="string"),
                            "role" => obj!("type"="string","enum"=["window","button","text_field","check_box","radio_button","list","list_item","menu","menu_item","tab","document","other"]),
                            "automation_id" => obj!("type"="string"),
                            "accessible_name" => obj!("type"="string"),
                            "ancestor_chain" => obj!(
                                "type"="array",
                                "description"="稳定的祖先/行上下文；用于区分重复控件，不包含坐标或运行时句柄",
                                "items"=obj!(
                                    "type"="object",
                                    "properties"=json_props! {
                                        "role" => obj!("type"="string","enum"=["window","button","text_field","check_box","radio_button","list","list_item","menu","menu_item","tab","document","other"]),
                                        "automation_id" => obj!("type"="string"),
                                        "accessible_name" => obj!("type"="string")
                                    }
                                )
                            ),
                            "supported_action" => obj!("type"="string","enum"=["invoke","toggle","select","expand","collapse","focus","set_value"]),
                            "ordinal_hint" => obj!("type"="integer","minimum"=0,"maximum"=65535,"description"="旧工作流诊断提示；不会用于消解歧义")
                        },
                        "required"=["app_id"]
                    ),
                    "action" => obj!("type"="string","enum"=["invoke","toggle","select","expand","collapse","focus","set_value"],"description"="本地允许的 UIA 原生动作"),
                    "launch_ref" => obj!("type"="string","description"="本地返回的可选稳定应用启动引用；不得自行编造"),
                    "value" => obj!("type"="string","description"="仅用于 set_value；文本只交给本地 UIA 执行器","maxLength"=16384)
                },
                &["locator", "action"],
            ),
        ];
        if self.enhanced_mode {
            schemas.push(tool_def(
                "desktop_agent_step",
                "增强模式下新桌面动作选择的首选入口：本地读取 UIA、构造候选动作，增强判断模型只能选择一个 candidate_id，本地复核后执行并重新观察验证。一次调用最多执行一个原生动作；未配置、服务不可用或语义树不适用时，由主模型从同一有限候选空间继续判断。",
                json_props! {
                    "goal" => obj!("type"="string","description"="当前桌面任务目标；增强判断模型仅据此从本地候选集合中选择"),
                    "target_token" => obj!("type"="string","description"="先通过 desktop_target_bind 确定任务目标"),
                    "scope" => obj!("type"="string","enum"=["window","menu"]),
                    "subtree_id" => obj!("type"="string","description"="本地观察返回的区域/元素 ID")
                },
                &["goal"],
            ));
        }
        schemas
    }

    fn desktop_tool_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        vec![
            // ═══ Desktop automation tools ═══
            tool_def("desktop_mouse",
                "鼠标操作。点击/悬停/移动优先传本次 perceive 的 capture_id + element_id，由本地换算并验证目标。旧 x/y 是屏幕绝对坐标（Windows 原生屏幕坐标，macOS 逻辑点），不是窗口/图片相对坐标。事件发送不代表业务成功。position 只读；macOS 需辅助功能授权。",
                json_props! {
                    "action" => obj!("type"="string","enum"=["click","double_click","hover","scroll","position","move"],"description"="写操作需 capture_id + element_id，或旧接口 (x,y)；position 只读"),
                    "hwnd" => obj!("type"="integer","description"="Target window handle. Optional for all write-actions; skip for position."),
                    "capture_id" => obj!("type"="string","description"="perceive 返回的本次本地捕获 ID，与 element_id 成对；不能同时传 x/y"),
                    "element_id" => obj!("type"="integer","minimum"=0,"description"="本次 perceive 返回的元素 ID；不保存到工作流常量"),
                    "x" => obj!("type"="integer","description"="旧接口：屏幕绝对 X，不能直接传图片内 center"),
                    "y" => obj!("type"="integer","description"="旧接口：屏幕绝对 Y，不能直接传图片内 center"),
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
                "向窗口输入文本或快捷键。保存工作流优先传成功语义动作的 target_locator，由本地重定位、激活并验证目标；不固化 hwnd。输入文本前先聚焦输入控件；快捷键可直接定位目标窗口。旧 hwnd 接口保留。",
                json_props! {
                    "mode" => obj!("type"="string","enum"=["type","hotkey"],"description"="type: input text; hotkey: press keys only"),
                    "target_locator" => obj!("type"="object","description"="成功 workflow_step.params.locator 的原样副本；与 hwnd 二选一，用于跨次运行的稳定定位"),
                    "launch_ref" => obj!("type"="string","description"="可选：成功 workflow_step.params.launch_ref，应用未运行时使用本地目录启动"),
                    "hwnd" => obj!("type"="integer","description"="Target window handle. Get from desktop_windows_list."),
                    "text" => obj!("type"="string","description"="Text to type (mode=type required)"),
                    "send" => obj!("type"="string","description"="Key to send after typing: \"enter\" (default), \"ctrl+enter\", \"tab\", or \"none\" to skip."),
                    "keys" => obj!("type"="array","items"=obj!("type"="string"),"description"="Key combo to press (mode=hotkey required). Single key: [\"enter\"],[\"f5\"],[\"esc\"]. Combo: [\"ctrl\",\"c\"],[\"alt\",\"tab\"].")
                },
                &["mode"]),
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
"本地 OCR+YOLO 元素定位。rect/center/image_center 是图片内坐标，不可直接当屏幕点。已登记截图附带 capture_id、element_id 和本地换算的 screen_center，优先用这两个 ID 调用 desktop_mouse。未知来源图片 screen_center 为 null。",
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
