//! desktop_executors — 桌面工具的 async executor
//!
//! 桌面工具不通过 ToolDef.executor 注册，而是由 ToolRegistry::execute() 检测到
//! desktop_ 前缀后，调用 execute_desktop_tool 传入 DesktopClient 执行。

use super::registry::ToolRegistry;
use crate::desktop::DesktopClient;
use crate::desktop_automation::{DesktopActionError, DispatchState};
use crate::ToolResult;

/// Preserve delivery metadata across legacy string-only tool callbacks.
fn action_error(state: DispatchState, message: impl Into<String>) -> String {
    let message = message.into();
    if DesktopActionError::decode(&message).is_some() {
        message
    } else {
        DesktopActionError::encode(state, message)
    }
}

fn native_action_error(error: crate::NuphusError) -> String {
    let message = match error {
        crate::NuphusError::Tool(message) => message,
        other => other.to_string(),
    };
    action_error(DispatchState::Unknown, message)
}

fn ensure_dispatch_response(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err(action_error(
            DispatchState::Unknown,
            value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("桌面动作发送结果不明"),
        ));
    }
    Ok(value)
}

fn capture_reference<'a>(
    params: &'a serde_json::Value,
    capture_key: &str,
    element_key: &str,
) -> Result<(&'a str, u32), String> {
    let capture = params
        .get(capture_key)
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{capture_key} 必须为非空字符串"))?;
    let element = params
        .get(element_key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format!("{element_key} 必须为非负整数"))?;
    Ok((capture, element))
}

fn optional_i32(params: &serde_json::Value, key: &str) -> Result<Option<i32>, String> {
    params
        .get(key)
        .map(|value| {
            value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| format!("{key} 必须为有效的 32 位整数"))
        })
        .transpose()
}

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
/// 显式指定目标窗口的鼠标操作同样必须以 true 为前提。
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
            .and_then(|value| {
                if value.get("success").and_then(|value| value.as_bool()) == Some(false) {
                    Err(crate::NuphusError::Tool(
                        value
                            .get("error")
                            .and_then(|value| value.as_str())
                            .unwrap_or("桌面操作失败")
                            .into(),
                    ))
                } else {
                    Ok(value)
                }
            })
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
        let result = self
            .execute_desktop_tool_inner(client, tool_name, params)
            .await;
        if matches!(
            tool_name,
            "desktop_mouse" | "desktop_mouse_drag" | "desktop_input"
        ) {
            // Native calls below tag sent/unknown errors explicitly. Everything
            // else failed during argument/target validation, before dispatch.
            result.map_err(|error| action_error(DispatchState::NotSent, error))
        } else {
            result
        }
    }

    async fn execute_desktop_tool_inner(
        &self,
        client: &DesktopClient,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> std::result::Result<ToolResult, String> {
        match tool_name {
            "desktop_mouse" => {
                let action = params
                    .get("action")
                    .and_then(|value| value.as_str())
                    .unwrap_or("position");
                if action == "position" {
                    return Self::wrap_desktop_result(client.mouse_position().await);
                }
                if !matches!(
                    action,
                    "click" | "double_click" | "hover" | "move" | "scroll"
                ) {
                    return Err(format!("unknown mouse action: {action}"));
                }
                // Validate before activating a window or consuming a one-use observation.
                let button = params
                    .get("button")
                    .map(|value| value.as_str().ok_or("button 必须为字符串"))
                    .transpose()?
                    .unwrap_or("left");
                let clicks = if action == "double_click" {
                    2
                } else {
                    optional_i32(params, "clicks")?.unwrap_or(1)
                };
                if matches!(action, "click" | "double_click")
                    && (!matches!(button, "left" | "right" | "middle")
                        || !(1..=2).contains(&clicks))
                {
                    return Err("button 必须为 left/right/middle，clicks 必须为 1 或 2".into());
                }
                let direction = params
                    .get("direction")
                    .map(|value| value.as_str().ok_or("direction 必须为字符串"))
                    .transpose()?
                    .unwrap_or("down");
                let amount = optional_i32(params, "amount")?.unwrap_or(3);
                if action == "scroll"
                    && (!matches!(direction, "up" | "down" | "left" | "right") || amount < 0)
                {
                    return Err("direction 必须为 up/down/left/right，amount 必须为非负整数".into());
                }
                let explicit_hwnd = optional_i32(params, "hwnd")?;
                let (context, resolved_point) =
                    match (params.get("capture_id"), params.get("element_id")) {
                        (None, None) => (None, None),
                        (Some(id), Some(element)) => {
                            if params.get("x").is_some() || params.get("y").is_some() {
                                return Err("capture_id + element_id 不能与 x/y 混用".into());
                            }
                            let id = id.as_str().ok_or("capture_id 必须为字符串")?;
                            let element = element
                                .as_u64()
                                .and_then(|value| u32::try_from(value).ok())
                                .ok_or("element_id 必须为非负整数")?;
                            let (context, point) = client
                                .resolve_capture_element(id, element)
                                .map_err(|e| e.to_string())?;
                            (Some(context), Some(point))
                        }
                        _ => return Err("capture_id 与 element_id 必须同时提供".into()),
                    };
                let capture_hwnd = context
                    .as_ref()
                    .and_then(|context| context.target.as_ref())
                    .map(|target| target.hwnd);
                if explicit_hwnd.is_some()
                    && capture_hwnd.is_some()
                    && explicit_hwnd != capture_hwnd
                {
                    return Err("hwnd 不属于该捕获".into());
                }
                let hwnd = explicit_hwnd.or(capture_hwnd);
                let before = match hwnd {
                    Some(hwnd) => Some(
                        client
                            .window_snapshot(hwnd)
                            .await
                            .map_err(|e| e.to_string())?,
                    ),
                    None => None,
                };
                if let Some(hwnd) = hwnd {
                    if !ensure_foreground(client, hwnd).await {
                        return Err("目标窗口激活失败，未发送鼠标动作；请刷新窗口状态".into());
                    }
                    let after = client
                        .window_snapshot(hwnd)
                        .await
                        .map_err(|e| e.to_string())?;
                    if before.as_ref() != Some(&after) {
                        return Err("窗口在激活期间发生变化，未发送鼠标动作；请重新观察".into());
                    }
                }
                if let Some(context) = &context {
                    client
                        .validate_capture_context(context)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                let point = match resolved_point {
                    Some(point) => Some(point),
                    None => match (optional_i32(params, "x")?, optional_i32(params, "y")?) {
                        (Some(x), Some(y)) => Some((x, y)),
                        (None, None) if action == "scroll" => before.as_ref().map(|window| {
                            (
                                (i64::from(window.bounds.x) + i64::from(window.bounds.width) / 2)
                                    as i32,
                                (i64::from(window.bounds.y) + i64::from(window.bounds.height) / 2)
                                    as i32,
                            )
                        }),
                        _ => {
                            return Err(
                                "请提供本次 capture_id + element_id，或完整的屏幕绝对坐标 x/y"
                                    .into(),
                            )
                        }
                    },
                };
                if let (Some(window), Some((x, y))) = (&before, point) {
                    if (explicit_hwnd.is_some()
                        || context
                            .as_ref()
                            .is_some_and(|context| context.scope == "window"))
                        && !window.bounds.contains(x, y)
                    {
                        return Err(format!(
                            "屏幕坐标 ({x}, {y}) 不在目标窗口中；未发送鼠标动作，请重新观察"
                        ));
                    }
                }
                // Consume before dispatch. An ambiguous OS failure must not replay a click.
                if matches!(action, "click" | "double_click" | "scroll") {
                    if let Some(context) = &context {
                        client
                            .consume_capture(&context.capture_id)
                            .map_err(|e| e.to_string())?;
                    }
                }
                let result = match action {
                    "click" | "double_click" => {
                        let (x, y) = point.ok_or("点击缺少目标")?;
                        client.mouse_click(x, y, button, clicks).await
                    }
                    "hover" => {
                        let (x, y) = point.ok_or("悬停缺少目标")?;
                        client.mouse_hover(x, y).await
                    }
                    "move" => {
                        let (x, y) = point.ok_or("移动缺少目标")?;
                        client.mouse_move(x, y, 0.0).await
                    }
                    "scroll" => {
                        if let Some((x, y)) = point {
                            client
                                .mouse_move(x, y, 0.0)
                                .await
                                .map_err(native_action_error)
                                .and_then(ensure_dispatch_response)?;
                        }
                        client.mouse_scroll(direction, amount).await
                    }
                    _ => unreachable!(),
                }
                .map_err(native_action_error)
                .and_then(ensure_dispatch_response)?;
                let _ = result;
                Self::wrap_desktop_result(Ok(serde_json::json!({"success":true,"result":{
                    "status":"dispatched","dispatch_state":"sent","effect":"unverifiable","action":action,"hwnd":hwnd,"coordinate_space":"screen",
                    "coordinate_units":crate::desktop::capture_context::screen_coordinate_units(),
                    "point":point.map(|(x,y)|serde_json::json!({"x":x,"y":y})),
                    "capture_id":context.as_ref().map(|context|&context.capture_id),
                    "verified":false,"note":"已发送输入事件；请重新观察确认目标应用的结果"
                }})))
            }
            "desktop_mouse_drag" => {
                let capture_keys = [
                    "start_capture_id",
                    "start_element_id",
                    "end_capture_id",
                    "end_element_id",
                ];
                let ((start_x, start_y), (end_x, end_y)) =
                    if capture_keys.iter().any(|key| params.get(*key).is_some()) {
                        if ["start_x", "start_y", "end_x", "end_y"]
                            .iter()
                            .any(|key| params.get(*key).is_some())
                        {
                            return Err("拖拽元素引用不能与坐标混用".into());
                        }
                        let (start_id, start_element) =
                            capture_reference(params, "start_capture_id", "start_element_id")?;
                        let (end_id, end_element) =
                            capture_reference(params, "end_capture_id", "end_element_id")?;
                        let (start_context, start) = client
                            .resolve_capture_element(start_id, start_element)
                            .map_err(|e| e.to_string())?;
                        let (end_context, end) = client
                            .resolve_capture_element(end_id, end_element)
                            .map_err(|e| e.to_string())?;
                        if let Some(target) = &start_context.target {
                            if !ensure_foreground(client, target.hwnd).await {
                                return Err("拖拽起点窗口激活失败；未发送拖拽，请重新观察".into());
                            }
                        }
                        // Both endpoints belong to recent local captures. Validate
                        // both before consuming either, including cross-window drag.
                        client
                            .validate_capture_context(&start_context)
                            .await
                            .map_err(|e| e.to_string())?;
                        client
                            .validate_capture_context(&end_context)
                            .await
                            .map_err(|e| e.to_string())?;
                        client
                            .consume_capture(start_id)
                            .map_err(|e| e.to_string())?;
                        if end_id != start_id {
                            client.consume_capture(end_id).map_err(|e| e.to_string())?;
                        }
                        (start, end)
                    } else {
                        (
                            (
                                optional_i32(params, "start_x")?.ok_or("start_x required")?,
                                optional_i32(params, "start_y")?.ok_or("start_y required")?,
                            ),
                            (
                                optional_i32(params, "end_x")?.ok_or("end_x required")?,
                                optional_i32(params, "end_y")?.ok_or("end_y required")?,
                            ),
                        )
                    };
                let response = client
                    .mouse_drag(start_x, start_y, end_x, end_y)
                    .await
                    .map_err(native_action_error)
                    .and_then(ensure_dispatch_response)?;
                Self::wrap_desktop_result(Ok(response))
            }
            "desktop_input" => {
                let mode = params
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("type");
                if !matches!(mode, "type" | "hotkey") {
                    return Err("mode 必须为 type 或 hotkey".into());
                }
                let send_raw = params
                    .get("send")
                    .and_then(|v| v.as_str())
                    .unwrap_or("enter");
                let keys: Vec<String> = if mode == "hotkey" {
                    params
                        .get("keys")
                        .and_then(|v| v.as_array())
                        .ok_or("keys 必须为字符串数组")?
                        .iter()
                        .map(|v| v.as_str().map(String::from).ok_or("keys 只能包含字符串"))
                        .collect::<Result<_, _>>()?
                } else if send_raw == "none" {
                    vec![]
                } else {
                    send_raw
                        .split('+')
                        .map(|key| key.trim().to_lowercase())
                        .collect()
                };
                if mode == "hotkey" || !keys.is_empty() {
                    let key_refs: Vec<_> = keys.iter().map(String::as_str).collect();
                    desktop_api::input::keyboard::validate_keys(&key_refs)
                        .map_err(|e| e.to_string())?;
                }
                let hwnd = if params.get("target_locator").is_some() {
                    self.semantic_input_window(params).await?
                } else {
                    optional_i32(params, "hwnd")?.ok_or("需要 target_locator 或当前 hwnd")?
                };
                // 键盘输入必须前置：executor 内部自动置前，失败才中止（避免输入误入其他窗口）
                if !ensure_foreground(client, hwnd).await {
                    return Err(format!("HWND({}) 窗口自动置前失败，为避免输入误入其他窗口已中止，请检查窗口状态后重试", hwnd));
                }
                match mode {
                    "hotkey" => {
                        let keys_display = keys.join("+");
                        client
                            .keyboard_hotkey(keys)
                            .await
                            .map_err(native_action_error)
                            .and_then(ensure_dispatch_response)?;
                        let mut msg = format!(
                            "已发送 HWND({}) 窗口的热键事件；业务结果尚未验证。hwnd: {}, keys={}",
                            hwnd, hwnd, keys_display
                        );
                        msg.push_str(&foreground_note(client, hwnd).await);
                        Ok(ToolResult::success(serde_json::json!({"status":"dispatched", "dispatch_state":"sent", "effect":"unverifiable", "hwnd":hwnd, "keys":keys_display, "message":msg}).to_string()))
                    }
                    _ => {
                        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        client
                            .input_send(text, hwnd, false)
                            .await
                            .map_err(native_action_error)
                            .and_then(ensure_dispatch_response)?;
                        if !keys.is_empty() {
                            client
                                .keyboard_hotkey(keys)
                                .await
                                .map_err(|e| {
                                    action_error(
                                        DispatchState::Partial,
                                        format!("文本已输入，后续按键结果不明: {e}"),
                                    )
                                })
                                .and_then(|response| {
                                    ensure_dispatch_response(response).map_err(|e| {
                                        action_error(
                                            DispatchState::Partial,
                                            format!("文本已输入: {e}"),
                                        )
                                    })
                                })?;
                        }
                        let mut msg = format!(
                            "已发送 HWND({}) 窗口的输入事件；业务结果尚未验证。hwnd: {}, chars={}, send={}",
                            hwnd, hwnd, text.chars().count(), send_raw
                        );
                        msg.push_str(&foreground_note(client, hwnd).await);
                        Ok(ToolResult::success(serde_json::json!({"status":"dispatched", "dispatch_state":"sent", "effect":"unverifiable", "hwnd":hwnd, "chars":text.chars().count(), "send":send_raw, "message":msg}).to_string()))
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

#[cfg(test)]
mod mouse_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absolute_coordinates_preserve_negative_monitor_origins_and_reject_overflow() {
        assert_eq!(
            optional_i32(&json!({"x": -1920}), "x").unwrap(),
            Some(-1920)
        );
        assert!(optional_i32(&json!({"x": 4294967296_i64}), "x").is_err());
        assert!(optional_i32(&json!({"x": "300"}), "x").is_err());
        assert_eq!(optional_i32(&json!({}), "x").unwrap(), None);
    }

    #[test]
    fn desktop_failure_payload_is_not_a_successful_tool_result() {
        let result =
            ToolRegistry::wrap_desktop_result(Ok(json!({"success":false,"error":"capture moved"})));
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "Tool error: capture moved");
    }

    #[test]
    fn native_delivery_errors_keep_their_state_through_string_callbacks() {
        for state in [
            DispatchState::NotSent,
            DispatchState::Sent,
            DispatchState::Partial,
            DispatchState::Unknown,
        ] {
            let encoded = DesktopActionError::encode(state, "native failure");
            let native = native_action_error(crate::NuphusError::Tool(encoded.clone()));
            assert_eq!(native, encoded);
            assert_eq!(action_error(DispatchState::NotSent, native), encoded);
        }
        let unknown = ensure_dispatch_response(json!({"success":false,"error":"native timeout"}))
            .unwrap_err();
        assert_eq!(
            DesktopActionError::decode(&unknown).unwrap().dispatch_state,
            DispatchState::Unknown
        );
    }

    #[test]
    fn drag_capture_references_require_valid_complete_pairs() {
        let params = json!({"start_capture_id":"local-capture", "start_element_id":3});
        assert_eq!(
            capture_reference(&params, "start_capture_id", "start_element_id").unwrap(),
            ("local-capture", 3)
        );
        for params in [
            json!({}),
            json!({"start_capture_id":"id"}),
            json!({"start_capture_id":"id","start_element_id":-1}),
            json!({"start_capture_id":"","start_element_id":0}),
        ] {
            assert!(capture_reference(&params, "start_capture_id", "start_element_id").is_err());
        }
    }
}
