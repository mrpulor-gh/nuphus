//! Linux 原生遮罩（X11）—— 替代 WebView overlay 渲染。
//!
//! # 实现方式
//!
//! | 环节 | 手段 |
//! |---|---|
//! | 窗口 | `override_redirect` 全屏窗口（主屏物理尺寸），`StackMode::ABOVE` 置顶 |
//! | 呈现 | 单 `u32` 像素缓冲 + `put_image`（ZPixmap，depth = 根窗口深度）；冻结帧当背景，遮罩由 `paint::render_opaque` 画在其上 |
//! | 输入 | `grab_pointer` / `grab_keyboard`（`owner_events = false`）**独占并吞掉**输入，事件不传给其它窗口 |
//! | 节拍 | 16ms 轮询（`poll_for_event` + sleep），`dirty` 才重绘 |
//! | 抓屏 | 由 `session.rs` 用 xcap 完成（本模块只显示） |
//! | 视觉规格 | `surface::paint`（与 Windows 同一份颜色/尺寸/字形/HUD 实现） |
//!
//! # 平台硬约束（必须让用户看得见，不要静默失败）
//!
//! - **Wayland 没有全局遮罩窗口协议**（GNOME / KDE 现在默认 Wayland）：本实现只在
//!   X11 会话（或 XWayland 可用）下成立。会话切换/能力探测由上层决定，本模块失败时
//!   返回 `Cancelled`，由 `commands/toolbar.rs` 的跨平台 WebView 链路兜底。
//! - 坐标口径：屏幕**物理像素**（`screen_w/h` 由 xcap 抓屏结果给出；多显示器/混合 DPI
//!   需要按显示器分别取 frame，当前只覆盖主屏，与 Windows 实现同范围）。
//!
//! # 验证状态
//!
//! **本文件在 Windows 开发机上不参与编译**（`#[cfg(target_os = "linux")]`），
//! 只能由 CI（ubuntu `cargo check --workspace` + `cargo test`）验证编译，
//! 交互行为需 X11 真机验证。

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, GrabMode,
    ImageFormat, StackMode, WindowClass,
};
use x11rb::protocol::Event;

use super::{OverlayOutcome, OverlayParams};
use crate::commands::capture::geometry::Point;
use crate::commands::capture::paint;
use crate::commands::capture::state::{Action, App, Phase};

/// 硬超时：无论如何都能自退出，绝不要求用户杀进程（与 Windows 实现同值）。
const HARD_TIMEOUT_SECS: u64 = 180;
/// 渲染节拍（≈60fps 合并重绘）。
const TICK_MS: u64 = 16;
/// X11 键码（标准 keycode，非 keysym）：ESC / Return / space。
const KEY_ESC: u8 = 9;
const KEY_RETURN: u8 = 36;
const KEY_SPACE: u8 = 65;

/// 跨平台统一入口（契约见 `surface::mod`）：阻塞直到用户确认 / 取消。
pub fn run_overlay(params: OverlayParams) -> OverlayOutcome {
    match run(&params) {
        Ok(outcome) => outcome,
        Err(e) => {
            // 失败不 panic：返回取消，由上层的跨平台兜底链路接管。
            tracing::warn!("[capture/linux] 原生遮罩未能启动：{e}");
            OverlayOutcome::Cancelled
        }
    }
}

fn run(params: &OverlayParams) -> Result<OverlayOutcome, Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num].clone();
    let depth = screen.root_depth;
    let (w, h) = (params.screen_w.max(1), params.screen_h.max(1));

    // ① 全屏 override-redirect 窗口（不参与 WM 管理、无装饰、可覆盖所有窗口）
    let wid = conn.generate_id()?;
    let aux = CreateWindowAux::new()
        .override_redirect(1)
        .background_pixel(screen.black_pixel)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION,
        );
    conn.create_window(
        depth,
        wid,
        screen.root,
        0,
        0,
        w as u16,
        h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &aux,
    )?
    .check()?;
    conn.map_window(wid)?.check()?;
    conn.configure_window(wid, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?
        .check()?;

    // ② 独占输入：鼠标 + 键盘都被本窗口吞掉，绝不落到下层应用/桌面
    //    （这是 Linux 侧的「右键只是退出键」保障：事件根本不会到达其它窗口）
    conn.grab_pointer(
        false,
        screen.root,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        x11rb::NONE,
        x11rb::CURRENT_TIME,
    )?
    .check()?;
    conn.grab_keyboard(
        false,
        screen.root,
        x11rb::CURRENT_TIME,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
    )?
    .check()?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, wid, &CreateGCAux::new())?.check()?;

    // ③ 状态机（平台无关）+ 像素缓冲
    let mut buf = vec![0u32; (w as usize) * (h as usize)];
    let mut app = App::new(w, h);
    if let Ok(pointer) = conn.query_pointer(screen.root)?.reply() {
        app.last_mouse = Point {
            x: pointer.root_x as i32,
            y: pointer.root_y as i32,
        };
    }

    let mut outcome = OverlayOutcome::Cancelled;
    let mut dirty = true;
    let mut done = false;
    let started = Instant::now();

    // ④ 交互循环：事件 → 状态机；dirty → render_opaque + put_image
    while !done && started.elapsed().as_secs() < HARD_TIMEOUT_SECS {
        while let Some(ev) = conn.poll_for_event()? {
            match ev {
                Event::MotionNotify(e) => {
                    let (px, py) = (e.event_x as i32, e.event_y as i32);
                    app.last_mouse = Point { x: px, y: py };
                    app.on_move(px, py, w, h);
                    dirty = true;
                }
                Event::ButtonPress(e) => {
                    let (px, py) = (e.event_x as i32, e.event_y as i32);
                    match e.detail {
                        1 => {
                            // 左键：走与 Windows 相同的命中判定（OK / X / 节点 / 洞内）
                            match app.on_ldown(px, py, w, h) {
                                Action::ConfirmSave => {
                                    outcome = OverlayOutcome::Confirmed {
                                        sel: (app.sel.x, app.sel.y, app.sel.w, app.sel.h),
                                    };
                                    done = true;
                                }
                                Action::Cancel => done = true,
                                Action::None => {}
                            }
                            dirty = true;
                        }
                        3 => {
                            // 右键 = 退出（不是右键菜单：事件已被 grab 吞掉）
                            app.enter_exiting();
                            done = true;
                        }
                        _ => {}
                    }
                }
                Event::ButtonRelease(e) => {
                    let (px, py) = (e.event_x as i32, e.event_y as i32);
                    if e.detail == 1 {
                        if params.point_pick {
                            // 单点拾取（取色 / 鼠标坐标）：松手即确认，选区 = 该点 1×1
                            app.pick_point_at(px, py);
                            outcome = OverlayOutcome::Confirmed {
                                sel: (px, py, 1, 1),
                            };
                            done = true;
                        } else {
                            app.on_lup(px, py);
                            dirty = true;
                        }
                    }
                }
                Event::KeyPress(e) => match e.detail {
                    KEY_ESC => {
                        app.enter_exiting();
                        done = true;
                    }
                    KEY_RETURN | KEY_SPACE => {
                        if matches!(app.phase, Phase::Confirm | Phase::Moving | Phase::Adjusting) {
                            outcome = OverlayOutcome::Confirmed {
                                sel: (app.sel.x, app.sel.y, app.sel.w, app.sel.h),
                            };
                            done = true;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if done {
            break;
        }
        if dirty {
            paint::render_opaque(
                &mut buf,
                w,
                h,
                &mut app,
                params.point_pick,
                params.pick_color,
                &params.frozen_bg,
            );
            // u32 缓冲按字节直接交给 X（ZPixmap）：depth 24/32 下 4 字节/像素，
            // 小端序字节为 B,G,R,A —— 与 `paint` 的 0xAARRGGBB 一致。
            let bytes =
                unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 4) };
            conn.put_image(
                wid,
                gc,
                depth,
                w as u16,
                h as u16,
                0,
                0,
                0,
                ImageFormat::Z_PIXMAP,
                bytes,
            )?
            .check()?;
            conn.flush()?;
            dirty = false;
        }
        std::thread::sleep(Duration::from_millis(TICK_MS));
    }

    // ⑤ 收尾（幂等）：释放抓取 → 销毁窗口（失败也不 panic，进程不退出）
    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = conn.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = conn.free_gc(gc);
    let _ = conn.destroy_window(wid);
    let _ = conn.flush();

    Ok(outcome)
}
