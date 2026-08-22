//! Linux 窗口管理 — 纯 Rust X11 实现 (x11rb)
//! 零外部依赖，直接与 X11 socket 通信。
#![cfg(target_os = "linux")]

use crate::Result;
use serde_json::Value;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// 连接 X11 服务，失败时给出友好提示
fn connect() -> crate::Result<(RustConnection, usize)> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
        let msg = if std::env::var("WAYLAND_DISPLAY").is_ok() {
            "当前桌面环境为 Wayland，窗口管理功能不可用。请切换到 X11 会话。"
        } else {
            "无法连接 X11 服务，请确认 DISPLAY 环境变量已设置。"
        };
        crate::NuphusError::Tool(format!("{msg} ({e})"))
    })?;
    Ok((conn, screen_num))
}

/// 列出所有可见窗口
pub fn windows_list() -> Result<Value> {
    let (conn, screen_num) = connect()?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    // 通过 _NET_CLIENT_LIST 获取窗口列表（更可靠）
    let atom = intern_atom(&conn, "_NET_CLIENT_LIST")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1024)
        .map_err(|e| format!("get_property: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply: {e}"))?;

    let mut windows = Vec::new();
    if let Some(windows_raw) = reply.value32() {
        for w in windows_raw {
            if let Ok(info) = window_info_inner(&conn, w) {
                windows.push(info);
            }
        }
    }
    Ok(serde_json::json!({ "success": true, "result": windows }))
}

/// 激活窗口
pub fn window_activate(hwnd: i32) -> Result<Value> {
    let (conn, _) = connect()?;
    let win = hwnd as u32;

    // 通过 EWMH 协议激活窗口
    let net_active = intern_atom(&conn, "_NET_ACTIVE_WINDOW")?;
    let event = ClientMessageEvent {
        response_type: 0,
        format: 32,
        sequence: 0,
        window: win,
        type_: net_active,
        data: [1, 0, 0, 0, 0].into(), // source=1 (pager)
    };
    let screen = &conn.setup().roots[0];
    conn.send_event(
        false,
        screen.root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        &event,
    )
    .map_err(|e| format!("send_event: {e}"))?;
    conn.flush().map_err(|e| format!("flush: {e}"))?;
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(serde_json::json!({ "success": true, "result": { "hwnd": hwnd, "foreground": true } }))
}

/// 移动窗口
pub fn window_move(hwnd: i32, x: i32, y: i32) -> Result<Value> {
    let (conn, _) = connect()?;
    let win = hwnd as u32;
    conn.configure_window(win, &ConfigureWindowAux::default().x(x).y(y))
        .map_err(|e| format!("configure_window: {e}"))?;
    conn.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(serde_json::json!({ "success": true, "result": { "hwnd": hwnd, "x": x, "y": y } }))
}

/// 调整窗口大小
pub fn window_resize(hwnd: i32, width: i32, height: i32) -> Result<Value> {
    let (conn, _) = connect()?;
    let win = hwnd as u32;
    conn.configure_window(
        win,
        &ConfigureWindowAux::default()
            .width(width as u32)
            .height(height as u32),
    )
    .map_err(|e| format!("configure_window: {e}"))?;
    conn.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(
        serde_json::json!({ "success": true, "result": { "hwnd": hwnd, "width": width, "height": height } }),
    )
}

/// 获取窗口信息
pub fn window_info(hwnd: i32) -> Result<Value> {
    let (conn, _) = connect()?;
    let info = window_info_inner(&conn, hwnd as u32)?;
    Ok(info)
}

/// 内部：获取单个窗口信息
fn window_info_inner(conn: &RustConnection, win: u32) -> std::result::Result<Value, String> {
    let geom = conn
        .get_geometry(win)
        .map_err(|e| format!("get_geometry: {e}"))?
        .reply()
        .map_err(|e| format!("get_geometry reply: {e}"))?;

    let title = get_window_title(conn, win).unwrap_or_default();
    let pid = get_window_pid(conn, win).unwrap_or(0);

    // Read process name from /proc/{pid}/comm (Linux-specific)
    let process_name = if pid > 0 {
        std::fs::read_to_string(format!("/proc/{}/comm", pid))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    Ok(serde_json::json!({
        "hwnd": win as i32,
        "title": title,
        "x": geom.x, "y": geom.y,
        "width": geom.width, "height": geom.height,
        "process_id": pid,
        "process_name": process_name,
    }))
}

/// 获取窗口标题（优先 _NET_WM_NAME，回退 WM_NAME）
fn get_window_title(conn: &RustConnection, win: u32) -> std::result::Result<String, String> {
    let atom = intern_atom(conn, "_NET_WM_NAME")?;
    let reply = conn
        .get_property(false, win, atom, AtomEnum::ANY, 0, 256)
        .map_err(|e| format!("get_property: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply: {e}"))?;
    if let Some(bytes) = reply.value8() {
        let bytes: Vec<u8> = bytes.collect();
        return Ok(String::from_utf8_lossy(&bytes).to_string());
    }
    // Fallback: WM_NAME
    let reply = conn
        .get_property(false, win, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)
        .map_err(|e| format!("get_property WM_NAME: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply: {e}"))?;
    if let Some(bytes) = reply.value8() {
        let bytes: Vec<u8> = bytes.collect();
        return Ok(String::from_utf8_lossy(&bytes).to_string());
    }
    Ok(String::new())
}

/// 获取窗口所属进程 PID
fn get_window_pid(conn: &RustConnection, win: u32) -> std::result::Result<u32, String> {
    let atom = intern_atom(conn, "_NET_WM_PID")?;
    let reply = conn
        .get_property(false, win, atom, AtomEnum::CARDINAL, 0, 1)
        .map_err(|e| format!("get_property PID: {e}"))?
        .reply()
        .map_err(|e| format!("get_property PID reply: {e}"))?;
    if let Some(mut values) = reply.value32() {
        if let Some(pid) = values.next() {
            return Ok(pid);
        }
    }
    Ok(0)
}

/// 获取当前前台（活动）窗口句柄
/// 通过 _NET_ACTIVE_WINDOW 属性查询，Wayland 下会因 X11 连接失败而返回明确错误
pub fn foreground_hwnd() -> Result<Value> {
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let atom = intern_atom(&conn, "_NET_ACTIVE_WINDOW")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|e| format!("get_property _NET_ACTIVE_WINDOW: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply _NET_ACTIVE_WINDOW: {e}"))?;
    if let Some(mut values) = reply.value32() {
        if let Some(active) = values.next() {
            return Ok(serde_json::json!({ "success": true, "result": { "hwnd": active as i64 } }));
        }
    }
    Ok(serde_json::json!({
        "success": true,
        "result": { "hwnd": 0 },
        "note": "未检测到活动窗口（_NET_ACTIVE_WINDOW 属性为空）。如使用 Wayland 桌面环境，窗口管理功能不可用，请切换到 X11 会话。"
    }))
}

/// 检查指定窗口是否为当前前台（活动）窗口
pub fn window_is_foreground(hwnd: i32) -> Result<Value> {
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let atom = intern_atom(&conn, "_NET_ACTIVE_WINDOW")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|e| format!("get_property _NET_ACTIVE_WINDOW: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply _NET_ACTIVE_WINDOW: {e}"))?;
    let foreground = reply
        .value32()
        .and_then(|mut values| values.next())
        .map(|active| active == hwnd as u32)
        .unwrap_or(false);
    Ok(serde_json::json!({
        "success": true,
        "result": { "hwnd": hwnd, "foreground": foreground }
    }))
}

/// 缓存常用 Atom
fn intern_atom(conn: &RustConnection, name: &str) -> std::result::Result<Atom, String> {
    conn.intern_atom(false, name.as_bytes())
        .map_err(|e| format!("intern_atom {name}: {e}"))?
        .reply()
        .map_err(|e| format!("intern_atom reply {name}: {e}"))
        .map(|r| r.atom)
}
