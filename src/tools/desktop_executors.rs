//! desktop_executors — 桌面工具的 async executor
//!
//! 桌面工具不通过 ToolDef.executor 注册，而是由 ToolRegistry::execute() 检测到
//! desktop_ 前缀后，调用 execute_desktop_tool 传入 DesktopClient 执行。

use super::registry::ToolRegistry;
use crate::desktop::DesktopClient;
use crate::ToolResult;

/// Check if window is foreground without activation (post-operation verification)
async fn check_foreground(client: &DesktopClient, hwnd: i32) -> bool {
    match client.window_is_foreground(hwnd).await {
        Ok(val) => val
            .get("result")
            .and_then(|r| r.get("foreground"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 操作前置前：先检查，未前置则 executor 内部自动激活（不再让 LLM 多跑一轮 activate）。
/// 返回最终是否前置。键盘类操作必须以 true 为前提（输入流进焦点窗口）；
/// 鼠标类操作 false 时仍可执行——点击可见的后台窗口，Windows 会激活并投递点击，
/// 仅在结果中标注警告。
async fn ensure_foreground(client: &DesktopClient, hwnd: i32) -> bool {
    if check_foreground(client, hwnd).await {
        return true;
    }
    match client.window_activate(hwnd).await {
        Ok(v) => v
            .get("result")
            .and_then(|r| r.get("foreground"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 操作后的前台变化说明——纯信息提示，永不作为失败判据。
/// 点击/输入可能触发弹窗、对话框、应用内跳转或窗口关闭，这些都是操作成功的正常表现。
/// 前台移到同进程窗口 → 判定为弹窗/对话框/应用内跳转，返回新 hwnd 供后续操作切换；
/// 移到其他进程 → 提示外部跳转/弹窗；无法判定 → 仅提示不在前台。
async fn foreground_note(client: &DesktopClient, hwnd: i32) -> String {
    if check_foreground(client, hwnd).await {
        return String::new();
    }
    let fg = match client.foreground_hwnd().await {
        Ok(v) => v
            .get("result")
            .and_then(|r| r.get("hwnd"))
            .and_then(|h| h.as_i64())
            .unwrap_or(0) as i32,
        Err(_) => 0,
    };
    if fg == 0 || fg == hwnd {
        return "；提示：目标窗口当前不在前台（可能被遮挡/最小化/已关闭）".to_string();
    }
    let same_proc = match (client.window_info(hwnd).await, client.window_info(fg).await) {
        (Ok(a), Ok(b)) => {
            let pa = a
                .get("result")
                .and_then(|r| r.get("process_id"))
                .and_then(|v| v.as_u64());
            let pb = b
                .get("result")
                .and_then(|r| r.get("process_id"))
                .and_then(|v| v.as_u64());
            matches!((pa, pb), (Some(x), Some(y)) if x == y)
        }
        _ => false,
    };
    if same_proc {
        format!("；提示：操作后前台变为同应用的窗口 hwnd={}（可能是弹窗/对话框/应用内跳转），后续操作可改用该 hwnd", fg)
    } else {
        format!(
            "；提示：操作后前台切换到其他应用的窗口 hwnd={}（可能是外部跳转/弹窗）",
            fg
        )
    }
}

impl ToolRegistry {
    /// 将 DesktopClient 的 serde_json::Value 结果转为人类可读的文本
    pub(super) fn wrap_desktop_result(
        result: std::result::Result<serde_json::Value, crate::NuphusError>,
    ) -> std::result::Result<ToolResult, String> {
        result
            .map(|v| {
                let text = match &v {
                    serde_json::Value::Null => String::new(),
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .map(|item| match item {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::Value::Object(obj) => {
                        if let Some(msg) = obj.get("message").and_then(|v| v.as_str()) {
                            msg.to_string()
                        } else if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                            text.to_string()
                        } else if let Some(output) = obj.get("output").and_then(|v| v.as_str()) {
                            output.to_string()
                        } else if obj.len() <= 3 {
                            obj.iter()
                                .map(|(k, v)| {
                                    format!("{}: {}", k, v.as_str().unwrap_or(&v.to_string()))
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            serde_json::to_string_pretty(&v).unwrap_or_default()
                        }
                    }
                    _ => v.to_string(),
                };
                ToolResult::success(text)
            })
            .map_err(|e| e.to_string())
    }

    /// 桌面工具异步执行器
    pub(super) async fn execute_desktop_tool(
        &self,
        client: &DesktopClient,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> std::result::Result<ToolResult, String> {
        match tool_name {
            "desktop_mouse" => {
                let action = params
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("position");
                // Optional hwnd for foreground verification and coordinate validation before click
                let hwnd_opt = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32);
                match action {
                    "click" | "double_click" => {
                        let clicks = if action == "double_click" {
                            2
                        } else {
                            params.get("clicks").and_then(|v| v.as_i64()).unwrap_or(1) as i32
                        };
                        let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        // Verify foreground + coordinate bounds before clicking
                        let mut activated = true;
                        if let Some(hwnd) = hwnd_opt {
                            activated = ensure_foreground(client, hwnd).await;
                        }
                        let button = params
                            .get("button")
                            .and_then(|v| v.as_str())
                            .unwrap_or("left");
                        let click_result = client.mouse_click(x, y, button, clicks).await;
                        if let Some(hwnd) = hwnd_opt {
                            let mut msg = format!(
                                "已完成对 HWND({}) 窗口的点击操作！hwnd: {}, 参数: x={}, y={}, button={}, clicks={}",
                                hwnd, hwnd, x, y, button, clicks
                            );
                            if !activated {
                                msg.push_str("；警告：自动置前失败，点击依赖目标点可见未被遮挡");
                            }
                            msg.push_str(&foreground_note(client, hwnd).await);
                            return Ok(ToolResult::success(msg));
                        }
                        Self::wrap_desktop_result(click_result)
                    }
                    "hover" => {
                        let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        // Verify foreground + coordinate bounds before hover
                        let mut activated = true;
                        if let Some(hwnd) = hwnd_opt {
                            activated = ensure_foreground(client, hwnd).await;
                            let info = client
                                .window_info(hwnd)
                                .await
                                .map_err(|e| format!("获取窗口信息失败: {}", e))?;
                            let win = info
                                .get("result")
                                .and_then(|r| r.get("window"))
                                .ok_or("无法解析窗口信息")?;
                            let wx = win.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let wy = win.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let ww = win.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let wh = win.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            if x < wx || x > wx + ww || y < wy || y > wy + wh {
                                return Err(format!(
                                    "悬停坐标({},{})不在目标窗口范围内(窗口: x={} y={} w={} h={})，窗口可能已被移动",
                                    x, y, wx, wy, ww, wh
                                ));
                            }
                        }
                        let _hover_result = client.mouse_hover(x, y).await;
                        if let Some(hwnd) = hwnd_opt {
                            let mut msg = format!(
                                "已完成对 HWND({}) 窗口的悬停操作！hwnd: {}, 参数: x={}, y={}",
                                hwnd, hwnd, x, y
                            );
                            if !activated {
                                msg.push_str("；警告：自动置前失败，悬停依赖目标点可见未被遮挡");
                            }
                            msg.push_str(&foreground_note(client, hwnd).await);
                            return Ok(ToolResult::success(msg));
                        }
                        Self::wrap_desktop_result(_hover_result)
                    }
                    "scroll" => {
                        let direction = params
                            .get("direction")
                            .and_then(|v| v.as_str())
                            .unwrap_or("down");
                        let amount =
                            params.get("amount").and_then(|v| v.as_i64()).unwrap_or(3) as i32;
                        // Scroll uses SendInput which goes to foreground window — must verify
                        let mut activated = true;
                        if let Some(hwnd) = hwnd_opt {
                            activated = ensure_foreground(client, hwnd).await;
                        }
                        let _scroll_result = client.mouse_scroll(direction, amount).await;
                        if let Some(hwnd) = hwnd_opt {
                            let mut msg = format!(
                                "已完成对 HWND({}) 窗口的滚轮操作！hwnd: {}, 参数: direction={}, amount={}",
                                hwnd, hwnd, direction, amount
                            );
                            if !activated {
                                msg.push_str("；警告：自动置前失败，滚轮依赖目标点可见未被遮挡");
                            }
                            msg.push_str(&foreground_note(client, hwnd).await);
                            return Ok(ToolResult::success(msg));
                        }
                        Self::wrap_desktop_result(_scroll_result)
                    }
                    "move" => {
                        let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        // Verify foreground + coordinate bounds before move
                        let mut activated = true;
                        if let Some(hwnd) = hwnd_opt {
                            activated = ensure_foreground(client, hwnd).await;
                            let info = client
                                .window_info(hwnd)
                                .await
                                .map_err(|e| format!("获取窗口信息失败: {}", e))?;
                            let win = info
                                .get("result")
                                .and_then(|r| r.get("window"))
                                .ok_or("无法解析窗口信息")?;
                            let wx = win.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let wy = win.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let ww = win.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let wh = win.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            if x < wx || x > wx + ww || y < wy || y > wy + wh {
                                return Err(format!(
                                    "移动坐标({},{})不在目标窗口范围内(窗口: x={} y={} w={} h={})，窗口可能已被移动",
                                    x, y, wx, wy, ww, wh
                                ));
                            }
                        }
                        let _move_result = client.mouse_move(x, y, 0.0).await;
                        if let Some(hwnd) = hwnd_opt {
                            let mut msg = format!(
                                "已完成对 HWND({}) 窗口的鼠标移动！hwnd: {}, 参数: x={}, y={}",
                                hwnd, hwnd, x, y
                            );
                            if !activated {
                                msg.push_str("；警告：自动置前失败，移动依赖目标点可见未被遮挡");
                            }
                            msg.push_str(&foreground_note(client, hwnd).await);
                            return Ok(ToolResult::success(msg));
                        }
                        Self::wrap_desktop_result(_move_result)
                    }
                    _ => Self::wrap_desktop_result(client.mouse_position().await),
                }
            }
            "desktop_mouse_drag" => {
                let start_x = params.get("start_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let start_y = params.get("start_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let end_x = params.get("end_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let end_y = params.get("end_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                Self::wrap_desktop_result(client.mouse_drag(start_x, start_y, end_x, end_y).await)
            }
            "desktop_input" => {
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32)
                    .ok_or_else(|| "hwnd is required".to_string())?;
                // 键盘输入必须前置：executor 内部自动置前，失败才中止（避免输入误入其他窗口）
                if !ensure_foreground(client, hwnd).await {
                    return Err(format!("HWND({}) 窗口自动置前失败，为避免输入误入其他窗口已中止，请检查窗口状态后重试", hwnd));
                }
                let mode = params
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("type");
                match mode {
                    "hotkey" => {
                        let keys: Vec<String> = params
                            .get("keys")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let keys_display = keys.join("+");
                        let _hk_result = client.keyboard_hotkey(keys).await;
                        let mut msg = format!(
                            "已完成对 HWND({}) 窗口的热键操作！hwnd: {}, 参数: keys={}",
                            hwnd, hwnd, keys_display
                        );
                        msg.push_str(&foreground_note(client, hwnd).await);
                        Ok(ToolResult::success(msg))
                    }
                    _ => {
                        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        let send_raw = params
                            .get("send")
                            .and_then(|v| v.as_str())
                            .unwrap_or("enter");
                        let send_keys: Vec<String> = if send_raw == "none" {
                            vec![]
                        } else {
                            send_raw
                                .split('+')
                                .map(|s| s.trim().to_lowercase())
                                .filter(|s| !s.is_empty())
                                .collect()
                        };
                        let _result = client.input_send(text, hwnd, false).await;
                        if !send_keys.is_empty() {
                            let _ = client.keyboard_hotkey(send_keys).await;
                        }
                        let mut msg = format!(
                            "已完成对 HWND({}) 窗口的输入操作！hwnd: {}, 参数: text={}, send={}",
                            hwnd, hwnd, text, send_raw
                        );
                        msg.push_str(&foreground_note(client, hwnd).await);
                        Ok(ToolResult::success(msg))
                    }
                }
            }
            "desktop_screenshot" => {
                let path = params.get("path").and_then(|v| v.as_str());
                let region = params.get("region").cloned();
                Self::wrap_desktop_result(client.screenshot(path, region).await)
            }
            "desktop_screen_size" => Self::wrap_desktop_result(client.screen_size().await),
            "desktop_windows_list" => Self::wrap_desktop_result(client.windows_list().await),
            "desktop_window_activate" => {
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32)
                    .ok_or_else(|| "hwnd is required".to_string())?;
                Self::wrap_desktop_result(client.window_activate(hwnd).await)
            }
            "desktop_window_screenshot" => {
                let title = params.get("title").and_then(|v| v.as_str());
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32);
                let path = params.get("path").and_then(|v| v.as_str());
                Self::wrap_desktop_result(client.window_screenshot(title, hwnd, path).await)
            }
            "desktop_window_move" => {
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32)
                    .ok_or_else(|| "hwnd is required".to_string())?;
                let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                Self::wrap_desktop_result(client.window_move(hwnd, x, y).await)
            }
            "desktop_window_resize" => {
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32)
                    .ok_or_else(|| "hwnd is required".to_string())?;
                let width = params.get("width").and_then(|v| v.as_i64()).unwrap_or(800) as i32;
                let height = params.get("height").and_then(|v| v.as_i64()).unwrap_or(600) as i32;
                Self::wrap_desktop_result(client.window_resize(hwnd, width, height).await)
            }
            "desktop_window_info" => {
                let hwnd = params
                    .get("hwnd")
                    .and_then(|v| v.as_i64())
                    .map(|h| h as i32)
                    .ok_or_else(|| "hwnd is required".to_string())?;
                Self::wrap_desktop_result(client.window_info(hwnd).await)
            }
            "desktop_vision" => {
                let image_path = params
                    .get("image_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let prompt = params.get("prompt").and_then(|v| v.as_str());
                Self::wrap_desktop_result(client.ocr("vision", image_path, false, prompt).await)
            }
            "desktop_perceive" => {
                let image_path = params
                    .get("image_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Self::wrap_desktop_result(client.perceive(image_path).await)
            }
            "desktop_clipboard_clean" => Self::wrap_desktop_result(client.clipboard_clean().await),
            "desktop_clipboard_write" => {
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                Self::wrap_desktop_result(client.clipboard_write(text).await)
            }
            "desktop_find_image" => {
                let template_path = params
                    .get("template_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let region = params.get("region").cloned();
                let threshold = params.get("threshold").and_then(|v| v.as_f64());
                let client = client.clone();

                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Handle::current();
                        rt.block_on(client.find_image(&template_path, region, threshold))
                    }),
                )
                .await
                .map_err(|_| "find_image 超时（30秒）".to_string())
                .and_then(|join| join.map_err(|e| format!("find_image 线程异常: {e}")))
                .and_then(Self::wrap_desktop_result)
            }
            "desktop_find_color" => {
                let color = params
                    .get("color")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let region = params.get("region").cloned();
                let direction = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let client = client.clone();

                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Handle::current();
                        rt.block_on(client.find_color(&color, region, direction.as_deref()))
                    }),
                )
                .await
                .map_err(|_| "find_color 超时（30秒）".to_string())
                .and_then(|join| join.map_err(|e| format!("find_color 线程异常: {e}")))
                .and_then(Self::wrap_desktop_result)
            }
            "desktop_find_multi_color" => {
                let anchor = params
                    .get("anchor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let offsets = params
                    .get("offsets")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let region = params.get("region").cloned();
                let min_match_ratio = params.get("min_match_ratio").and_then(|v| v.as_f64());
                let direction = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let client = client.clone();

                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Handle::current();
                        rt.block_on(client.find_multi_color(
                            &anchor,
                            &offsets,
                            region,
                            min_match_ratio,
                            direction.as_deref(),
                        ))
                    }),
                )
                .await
                .map_err(|_| "find_multi_color 超时（30秒）".to_string())
                .and_then(|join| join.map_err(|e| format!("find_multi_color 线程异常: {e}")))
                .and_then(Self::wrap_desktop_result)
            }
            "desktop_find_text" => {
                let dict_name = params
                    .get("dict_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let words = params
                    .get("words")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let region = params.get("region").cloned();
                let sim = params.get("sim").and_then(|v| v.as_f64()).map(|v| v as f32);
                let client = client.clone();

                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Handle::current();
                        rt.block_on(client.find_text(&dict_name, &words, region, sim))
                    }),
                )
                .await
                .map_err(|_| "find_text 超时（30秒）".to_string())
                .and_then(|join| join.map_err(|e| format!("find_text 线程异常: {e}")))
                .and_then(Self::wrap_desktop_result)
            }
            _ => Err(format!("Unknown desktop tool: {}", tool_name)),
        }
    }
}
