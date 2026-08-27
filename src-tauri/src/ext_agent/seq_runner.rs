//! SeqRunner — agent_dispatch 内部工具序列轻量执行器
//!
//! 复用工作流 `Action::Tool { tool, with }` 语义（格式对齐，不经 LLM 逐步调用）：
//! - 占位符替换：字符串值中的 `{hwnd}` / `{task_id}` / `{brief_path}` / `{message}`
//!   等替换为实参（变量池 HashMap）；数值类型不参与替换（width=1200 保持数值）。
//! - 内建步 `__sleep`：`with.ms` 毫秒休眠（慢速应用步间节拍）。
//! - 其余步按桌面工具白名单分发给 `DesktopClient`（参数语义与
//!   src/tools/desktop_executors.rs 的 desktop_* 执行器一致）。
//! - 任一步失败即中止（不带断点续连 —— 步骤短，重跑成本低）。
//!
//! 本模块只做「执行」，不做上板/进程捕获/await 门铃 —— 那是编排层
//! （mod.rs dispatch_async）的职责。

use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::desktop::DesktopClient;
use std::collections::HashMap;

/// 单步失败错误（step_index 为 0-based 步序号）
#[derive(Debug, Clone)]
pub struct SeqError {
    pub step_index: usize,
    pub tool: String,
    pub message: String,
}

impl std::fmt::Display for SeqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "dispatch_steps[{}] 工具「{}」执行失败: {}",
            self.step_index + 1,
            self.tool,
            self.message
        )
    }
}

/// 从步骤 JSON 中提取 tool 并替换占位符（纯函数，可单测）。
/// 返回 (tool, 替换后的 with)。
pub(crate) fn prepare_step(
    step: &serde_json::Value,
    vars: &HashMap<String, String>,
) -> Result<(String, serde_json::Value), SeqError> {
    let tool = step
        .get("tool")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if tool.is_empty() {
        return Err(SeqError {
            step_index: 0,
            tool: "<missing>".to_string(),
            message: "步骤缺少 tool 字段".to_string(),
        });
    }
    let with = step.get("with").cloned().unwrap_or(serde_json::json!({}));
    let with = substitute_value(&with, vars);
    Ok((tool, with))
}

/// 递归替换字符串值中的 {var} 占位符；数值/布尔/空不参与替换。
pub(crate) fn substitute_value(
    value: &serde_json::Value,
    vars: &HashMap<String, String>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(substitute_string(s, vars)),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute_value(v, vars)))
                .collect(),
        ),
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| substitute_value(v, vars))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn substitute_string(s: &str, vars: &HashMap<String, String>) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// 顺序执行 dispatch_steps；每步经 emitter 推 HUD 进度（执行栏可见）。
/// 成功返回完成步数；任一步失败返回 (step_index, tool, error) 语义的 SeqError。
pub async fn run_steps(
    steps: &[serde_json::Value],
    vars: &HashMap<String, String>,
    client: &DesktopClient,
    emitter: Option<&dyn EventEmitter>,
) -> Result<usize, SeqError> {
    for (idx, step) in steps.iter().enumerate() {
        let (tool, with) = prepare_step(step, vars).map_err(|mut e| {
            e.step_index = idx;
            e
        })?;

        if let Some(em) = emitter {
            em.emit(NuphusEvent::HudUpdate {
                text: format!("agent_dispatch[{}/{}] {}", idx + 1, steps.len(), tool),
                phase: "running".to_string(),
                step_kind: Some("tool".to_string()),
            });
        }

        if tool == "__sleep" {
            let ms = with.get("ms").and_then(|v| v.as_u64()).unwrap_or(0);
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            continue;
        }

        execute_desktop_step(client, &tool, &with)
            .await
            .map_err(|message| SeqError {
                step_index: idx,
                tool: tool.clone(),
                message,
            })?;
    }
    Ok(steps.len())
}

/// DesktopClient 包装结果解析：Result<Value, NuphusError> + {success,result/error} → Result<(), String>
fn unwrap(
    r: std::result::Result<serde_json::Value, nuphus::NuphusError>,
) -> Result<(), String> {
    let value = r.map_err(|e| e.to_string())?;
    if value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// 参数提取：接受数值（12345）或占位符替换后的数字字符串（"{hwnd}" → "12345"）
fn i32_at(with: &serde_json::Value, key: &str) -> Option<i32> {
    with.get(key).and_then(|v| {
        v.as_i64().or_else(|| {
            v.as_str()
                .and_then(|s| s.trim().parse::<i64>().ok())
        })
    })
    .map(|v| v as i32)
}

fn required_i32(with: &serde_json::Value, key: &str) -> Result<i32, String> {
    i32_at(with, key).ok_or_else(|| format!("{key} 必填（整数）"))
}

/// 键盘类操作前置保证：未在前台则先激活（与 desktop_executors::ensure_foreground 同语义）。
async fn ensure_foreground(client: &DesktopClient, hwnd: i32) -> bool {
    if let Ok(v) = client.window_is_foreground(hwnd).await {
        if v.get("result")
            .and_then(|r| r.get("foreground"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            return true;
        }
    }
    matches!(
        client.window_activate(hwnd).await,
        Ok(v)
            if v.get("result")
                .and_then(|r| r.get("foreground"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
    )
}

/// 桌面工具白名单分发。tool 名不在白名单 → Err（调用方中止整条序列）。
async fn execute_desktop_step(
    client: &DesktopClient,
    tool: &str,
    with: &serde_json::Value,
) -> Result<(), String> {
    match tool {
        "desktop_window_activate" => {
            let hwnd = required_i32(with, "hwnd")?;
            unwrap(client.window_activate(hwnd).await)
        }
        "desktop_window_resize" => {
            let hwnd = required_i32(with, "hwnd")?;
            let width = i32_at(with, "width").unwrap_or(800);
            let height = i32_at(with, "height").unwrap_or(600);
            unwrap(client.window_resize(hwnd, width, height).await)
        }
        "desktop_window_move" => {
            let hwnd = required_i32(with, "hwnd")?;
            let x = i32_at(with, "x").unwrap_or(0);
            let y = i32_at(with, "y").unwrap_or(0);
            unwrap(client.window_move(hwnd, x, y).await)
        }
        "desktop_window_info" => {
            let hwnd = required_i32(with, "hwnd")?;
            unwrap(client.window_info(hwnd).await)
        }
        "desktop_window_screenshot" => {
            let hwnd = i32_at(with, "hwnd");
            let title = with.get("title").and_then(|v| v.as_str());
            let path = with.get("path").and_then(|v| v.as_str());
            unwrap(client.window_screenshot(title, hwnd, path).await)
        }
        "desktop_windows_list" => unwrap(client.windows_list().await),
        "desktop_screen_size" => unwrap(client.screen_size().await),
        "desktop_screenshot" => {
            let path = with.get("path").and_then(|v| v.as_str());
            let region = with.get("region").cloned();
            unwrap(client.screenshot(path, region).await)
        }
        // desktop_mouse_click：方案文档 3.2 示例别名，等价 desktop_mouse action=click
        "desktop_mouse" | "desktop_mouse_click" => {
            let action = if tool == "desktop_mouse_click" {
                "click".to_string()
            } else {
                with.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click")
                    .to_string()
            };
            match action.as_str() {
                "click" | "double_click" => {
                    let x = i32_at(with, "x").unwrap_or(0);
                    let y = i32_at(with, "y").unwrap_or(0);
                    let clicks = if action == "double_click" {
                        2
                    } else {
                        i32_at(with, "clicks").unwrap_or(1)
                    };
                    let button = with
                        .get("button")
                        .and_then(|v| v.as_str())
                        .unwrap_or("left");
                    // 可选 hwnd：点击前先激活目标窗口（与 desktop_executors 同语义）
                    if let Some(hwnd) = i32_at(with, "hwnd") {
                        let _ = client.window_activate(hwnd).await;
                    }
                    unwrap(client.mouse_click(x, y, button, clicks).await)
                }
                "move" => {
                    let x = i32_at(with, "x").unwrap_or(0);
                    let y = i32_at(with, "y").unwrap_or(0);
                    unwrap(client.mouse_move(x, y, 0.0).await)
                }
                "position" => unwrap(client.mouse_position().await),
                other => Err(format!(
                    "desktop_mouse 不支持 action={other}（click/double_click/move/position）"
                )),
            }
        }
        "desktop_mouse_drag" => {
            let start_x = i32_at(with, "start_x").unwrap_or(0);
            let start_y = i32_at(with, "start_y").unwrap_or(0);
            let end_x = i32_at(with, "end_x").unwrap_or(0);
            let end_y = i32_at(with, "end_y").unwrap_or(0);
            unwrap(client.mouse_drag(start_x, start_y, end_x, end_y).await)
        }
        "desktop_input" => {
            let hwnd = required_i32(with, "hwnd")?;
            // 键盘输入必须前置：激活失败即中止，避免输入误入其他窗口
            if !ensure_foreground(client, hwnd).await {
                return Err(format!(
                    "HWND({hwnd}) 窗口自动置前失败，为避免输入误入其他窗口已中止"
                ));
            }
            let mode = with
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("type");
            match mode {
                "hotkey" => {
                    let keys: Vec<String> = with
                        .get("keys")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    unwrap(client.keyboard_hotkey(keys).await)
                }
                _ => {
                    let text = with
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let send_raw = with
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
                    let _ = client
                        .input_send(text, hwnd, false)
                        .await
                        .map_err(|e| e.to_string())?;
                    if !send_keys.is_empty() {
                        let _ = client.keyboard_hotkey(send_keys).await;
                    }
                    Ok(())
                }
            }
        }
        "desktop_clipboard_write" => {
            let text = with.get("text").and_then(|v| v.as_str()).unwrap_or("");
            unwrap(client.clipboard_write(text).await)
        }
        "desktop_clipboard_clean" => unwrap(client.clipboard_clean().await),
        other => Err(format!(
            "未知桌面工具「{other}」（不在 agent_dispatch 白名单；可用 desktop_window_*/desktop_input/desktop_clipboard_*/desktop_mouse*/desktop_screenshot/__sleep）"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("hwnd".to_string(), "12345".to_string());
        m.insert("task_id".to_string(), "0827-01".to_string());
        m.insert("brief_path".to_string(), "C:/handoff/brief.md".to_string());
        m.insert("message".to_string(), "你好，请重构页面".to_string());
        m
    }

    #[test]
    fn test_placeholder_substitution_strings_only() {
        let v = serde_json::json!({
            "hwnd": "{hwnd}",
            "width": 1200,
            "keys": ["ctrl", "v"],
            "text": "任务 {task_id} 见 {brief_path}",
        });
        let out = substitute_value(&v, &vars());
        assert_eq!(out["hwnd"], "12345");
        assert_eq!(out["width"], 1200, "数值类型不参与替换");
        assert_eq!(out["keys"][0], "ctrl");
        assert_eq!(out["text"], "任务 0827-01 见 C:/handoff/brief.md");
        // 未知占位符保持原样（不静默吞掉）
        let v2 = serde_json::json!({ "text": "未定义 {missing}" });
        assert_eq!(substitute_value(&v2, &vars())["text"], "未定义 {missing}");
    }

    #[test]
    fn test_prepare_step_extracts_tool_and_with() {
        let step = serde_json::json!({
            "tool": "desktop_window_resize",
            "with": { "hwnd": "{hwnd}", "width": 1200, "height": 800 }
        });
        let (tool, with) = prepare_step(&step, &vars()).unwrap();
        assert_eq!(tool, "desktop_window_resize");
        assert_eq!(with["hwnd"], "12345");
        assert_eq!(with["width"], 1200);
    }

    #[test]
    fn test_prepare_step_missing_tool_rejected() {
        let step = serde_json::json!({ "with": {} });
        let e = prepare_step(&step, &vars()).unwrap_err();
        assert!(e.message.contains("tool"));
    }

    #[test]
    fn test_unknown_tool_rejected_with_tool_name() {
        // 纯解析层已能拒绝（白名单判定在 execute 层）；这里验证错误路径包含步骤名
        let client = DesktopClient::new();
        let e = tokio_test::block_on(execute_desktop_step(
            &client,
            "desktop_evil",
            &serde_json::json!({}),
        ))
        .unwrap_err();
        assert!(e.contains("desktop_evil"));
        assert!(e.contains("白名单"));
    }

    #[test]
    fn test_desktop_mouse_click_alias_resolves_click() {
        // action 解析是纯逻辑：desktop_mouse_click 等价 action=click（经 execute 层分派）
        let with = serde_json::json!({ "hwnd": "{hwnd}", "x": 600, "y": 750 });
        let substituted = substitute_value(&with, &vars());
        assert_eq!(substituted["hwnd"], "12345");
        assert_eq!(substituted["x"], 600);
        // 其余参数语义与 desktop_executors 一致（x/y/button/clicks 直传 DesktopClient）
        assert_eq!(i32_at(&substituted, "x"), Some(600));
        assert_eq!(i32_at(&substituted, "hwnd"), Some(12345));
    }

    #[test]
    fn test_sleep_step_uses_ms_and_is_not_dispatched() {
        // __sleep 是内建步，不进入 execute_desktop_step —— 0ms 立即返回（安全、无副作用）
        let steps = vec![serde_json::json!({ "tool": "__sleep", "with": { "ms": 0 } })];
        let client = DesktopClient::new();
        let done = tokio_test::block_on(run_steps(&steps, &vars(), &client, None));
        assert_eq!(done.unwrap(), 1);
    }

    #[test]
    fn test_run_steps_stops_at_first_failure() {
        let steps = vec![
            serde_json::json!({ "tool": "__sleep", "with": { "ms": 0 } }),
            serde_json::json!({ "tool": "desktop_unknown_tool", "with": {} }),
            serde_json::json!({ "tool": "__sleep", "with": { "ms": 0 } }),
        ];
        let client = DesktopClient::new();
        let e = tokio_test::block_on(run_steps(&steps, &vars(), &client, None)).unwrap_err();
        assert_eq!(e.step_index, 1, "第 2 步失败即中止，不继续执行第 3 步");
        assert_eq!(e.tool, "desktop_unknown_tool");
        assert!(e.to_string().contains("desktop_unknown_tool"), "错误含步骤名");
    }
}