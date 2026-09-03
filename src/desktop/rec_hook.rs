//! rec_hook.rs — 单次低层输入捕获（工作流录制单元底座）
//!
//! Windows-only：安装 WH_MOUSE_LL / WH_KEYBOARD_LL 低层钩子，
//! 捕获一次目标事件（鼠标点击/双击/滚轮/组合键）后立即卸载。
//! 会话外零监听——hook 只存活于 capture_once 阻塞期间。
//!
//! 线程模型：
//! - capture_once spawn 捕获线程：装 hook → 消息泵(GetMessageW) → 目标事件 → 卸载 → 返回
//! - 取消：CANCEL 标志 + PostThreadMessageW(WM_QUIT) 唤醒泵
//! - 超时：watch 线程 sleep 后 PostThreadMessageW(WM_QUIT)
//! - 回调内必须 CallNextHookEx 放行（绝不吞用户真实输入）
//! - 单击/滚动合成：hook 回调不 sleep——down1/首个 wheel 后用 SetTimer(NULL, …) 线程定时器
//!   装载合成窗口，WM_TIMER 由消息泵 DispatchMessageW 派发到回调内完成 click/scroll 返回；
//!   双击在窗口内同点 down2 直接合成（KillTimer 销毁候选）；取消/超时/事件收尾统一 KillTimer。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;

/// 单次捕获事件结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureEvent {
    pub kind: String,           // "click" | "double_click" | "scroll" | "hotkey"
    pub button: Option<String>, // "left" | "right" | "middle" | None
    pub x: i32,
    pub y: i32, // 屏幕绝对坐标
    pub wheel_delta: Option<i32>,
    pub keys: Vec<String>, // hotkey 按键名（如 ["ctrl","c"]）
    pub window_title: String,
    pub hwnd: isize, // 前台窗口句柄
    /// 前台窗口所属进程 PID（0 = 无前台/不可用）。标题不可靠时（点击桌面/空标题）记录真实归属。
    pub pid: u32,
    /// 进程可执行文件名（如 "explorer.exe"）；获取失败或无前台为 None
    pub process_name: Option<String>,
    pub ts_ms: u64,
}

/// 要捕获的目标事件种类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Click,
    Scroll,
    Hotkey,
    Any,
}

impl CaptureKind {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "click" => Ok(Self::Click),
            "scroll" => Ok(Self::Scroll),
            "hotkey" => Ok(Self::Hotkey),
            "any" => Ok(Self::Any),
            other => Err(format!(
                "未知捕获类型: {other}（可选 click/scroll/hotkey/any）"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Scroll => "scroll",
            Self::Hotkey => "hotkey",
            Self::Any => "any",
        }
    }
}

// ── 前台窗口快照（CaptureEvent 扩展进程归属字段的数据源）──
/// 事件发生时前台窗口快照（标题 + 句柄 + 父级进程 PID/进程名）。
/// 标题不可靠（桌面/空标题）时 pid/process_name 提供窗口真实归属。
#[derive(Clone, Default)]
struct ForegroundWindow {
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
    pub process_name: Option<String>,
}

// ── 合成窗口常量 ──
/// 滚动合并窗口：同向最后一个 wheel 事件后 200ms 内无继续滚动即返回合并事件。
/// 200ms：快速连续滚动可感知合并（反馈延迟 ≈ 一次滚轮停顿），又不把明显停顿的两个滚动粘成一次。
const SCROLL_WINDOW_MS: u64 = 200;

/// 单击/双击判定窗口：运行时取系统双击速度（GetDoubleClickTime，默认 ~500ms）。
/// 旧实现固定 300ms 把真实用户慢双击（第二下常 >300ms，Windows 默认双击阈值 ~500ms）
/// 误拆成两次单击 → 慢双击录成单击。现改为尊重系统设置，双击识别与 OS 语义一致。
#[cfg(windows)]
fn click_window_ms() -> u32 {
    // # Safety: GetDoubleClickTime 无参数、只读系统设置，无内存副作用。
    let ms = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    if ms == 0 {
        500 // 系统未配置（理论极罕见）时回退 Windows 默认双击阈值
    } else {
        ms
    }
}

/// 单击/双击合成候选（down1 之后等待双击窗口）
struct ClickWait {
    /// SetTimer(NULL,…) 返回的线程定时器 id；0 = 未装载
    timer_id: usize,
    bcode: u8, // 0 left / 1 right / 2 middle
    x: i32,
    y: i32,
    ts: u64,
    fg: ForegroundWindow,
}

/// 滚动同向合并累积（首个 wheel 事件装载，窗口内同向续滚重置窗口）
struct ScrollWait {
    timer_id: usize,
    /// 起点坐标（首个 wheel 事件位置；合并事件返回此坐标）
    x: i32,
    y: i32,
    /// 同向累积 delta（上滚为正）
    delta: i32,
    fg: ForegroundWindow,
}

// ── 跨线程共享状态 ──
static CANCEL: AtomicBool = AtomicBool::new(false);
/// 超时 watch 线程到点置位（主循环在捕获线程退出后据此区分「取消 / 超时 / 异常」）
static TIMEOUT_FLAG: AtomicBool = AtomicBool::new(false);
/// true = 忽略 Nuphus 自身窗口的输入事件（前端 isSelfCapture 的后端根治，捕获会话级开关）
static IGNORE_SELF: AtomicBool = AtomicBool::new(false);
static CAPTURE_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static RESULT_TX: std::sync::OnceLock<std::sync::Mutex<Option<mpsc::Sender<CaptureEvent>>>> =
    std::sync::OnceLock::new();
static TARGET: AtomicU32 = AtomicU32::new(0); // CaptureKind as u32
/// 单击/双击合成候选（down1 后等待窗口期，None = 无候选）
static CLICK_WAIT: std::sync::OnceLock<std::sync::Mutex<Option<ClickWait>>> =
    std::sync::OnceLock::new();
/// 滚动同向累积（合并窗口内）
static SCROLL_WAIT: std::sync::OnceLock<std::sync::Mutex<Option<ScrollWait>>> =
    std::sync::OnceLock::new();
static MOD_KEYS: AtomicU32 = AtomicU32::new(0); // bit0 ctrl bit1 shift bit2 alt bit3 win

fn result_tx() -> &'static std::sync::Mutex<Option<mpsc::Sender<CaptureEvent>>> {
    RESULT_TX.get_or_init(|| std::sync::Mutex::new(None))
}

fn click_wait() -> &'static std::sync::Mutex<Option<ClickWait>> {
    CLICK_WAIT.get_or_init(|| std::sync::Mutex::new(None))
}

fn scroll_wait() -> &'static std::sync::Mutex<Option<ScrollWait>> {
    SCROLL_WAIT.get_or_init(|| std::sync::Mutex::new(None))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 取消进行中的 capture_once（幂等）
pub fn rec_cancel_current() {
    CANCEL.store(true, Ordering::SeqCst);
    wake_pump();
}

fn wake_pump() {
    let tid = CAPTURE_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// 阻塞直到捕获一次目标事件/超时/取消。timeout_secs 为 0 时默认 60 秒。
///
/// `ignore_self_window`：true 时忽略 Nuphus 自身窗口（前台窗口标题含 "Nuphus"）产生的
/// 输入事件——用户在 Nuphus 主窗内的点击/按键不再返回为捕获事件，根治「自窗口误捕获」
/// 导致的录制循环（前端 isSelfCapture 保留为第二道防线）。
pub fn capture_once(
    kind: CaptureKind,
    timeout_secs: u64,
    ignore_self_window: bool,
) -> Result<CaptureEvent, String> {
    #[cfg(windows)]
    {
        capture_once_windows(kind, timeout_secs, ignore_self_window)
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, timeout_secs, ignore_self_window);
        Err("rec_hook 仅支持 Windows".to_string())
    }
}

// ═══════════════════════════════════════════════════════════
// Windows 实现
// ═══════════════════════════════════════════════════════════

#[cfg(windows)]
fn capture_once_windows(
    kind: CaptureKind,
    timeout_secs: u64,
    ignore_self_window: bool,
) -> Result<CaptureEvent, String> {
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowTextW,
        GetWindowThreadProcessId, KillTimer, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
        HHOOK, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
        WM_MBUTTONDOWN, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    // 重置共享状态
    CANCEL.store(false, Ordering::SeqCst);
    TIMEOUT_FLAG.store(false, Ordering::SeqCst);
    IGNORE_SELF.store(ignore_self_window, Ordering::SeqCst);
    CAPTURE_THREAD_ID.store(0, Ordering::SeqCst);
    TARGET.store(kind as u32, Ordering::SeqCst);
    // 清空上一会话残留的合成候选（线程定时器随上一捕获线程退出已销毁，这里只复位状态；
    // 上一会话若有未触发的定时器，其 KillTimer 已在消息泵收尾统一执行）
    if let Ok(mut g) = click_wait().lock() {
        *g = None;
    }
    if let Ok(mut g) = scroll_wait().lock() {
        *g = None;
    }
    MOD_KEYS.store(0, Ordering::SeqCst);

    let (tx, rx) = mpsc::channel::<CaptureEvent>();
    if let Ok(mut g) = result_tx().lock() {
        *g = Some(tx);
    }

    let kind_val = kind as u32;
    let want = move |k: u32| kind_val == CaptureKind::Any as u32 || kind_val == k;

    // ── 前台窗口信息（事件发生时取；含父级 PID/进程名，供 draft 记录窗口真实归属）──
    unsafe fn foreground_info() -> ForegroundWindow {
        let hwnd = GetForegroundWindow();
        if hwnd.0 == 0 {
            return ForegroundWindow {
                hwnd: 0,
                ..Default::default()
            };
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        let end = (len as usize).min(511);
        let (pid, process_name) = process_of_window(hwnd);
        ForegroundWindow {
            hwnd: hwnd.0 as isize,
            title: String::from_utf16_lossy(&buf[..end]),
            pid,
            process_name,
        }
    }

    /// 取前台窗口所属进程 PID + 可执行文件名（标题不可靠场景的窗口真实归属）。
    /// best-effort：OpenProcess / QueryFullProcessImageNameW 失败返回 (0, None)，不阻塞主功能。
    unsafe fn process_of_window(hwnd: HWND) -> (u32, Option<String>) {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
        if pid == 0 {
            return (0, None);
        }
        let name = process_name_of_pid(pid);
        (pid, name)
    }

    /// 进程可执行文件名（"explorer.exe" 粒度）。用 PROCESS_QUERY_LIMITED_INFORMATION +
    /// QueryFullProcessImageNameW：对提升/受保护进程比 GetModuleBaseNameW 更易成功；
    /// 两者均在已启用 feature（Threading/Foundation）内，无需新增 windows feature。
    unsafe fn process_name_of_pid(pid: u32) -> Option<String> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let queried = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        if queried.is_err() {
            return None;
        }
        let len = buf[..size as usize]
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(size as usize);
        if len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len]);
        Some(
            std::path::Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(path),
        )
    }

    /// 销毁线程定时器（id 0 = 未装载，幂等）。hwnd 传 HWND::default()（NULL）即线程定时器。
    unsafe fn kill_thread_timer(timer_id: usize) {
        if timer_id != 0 {
            let _ = KillTimer(HWND::default(), timer_id);
        }
    }

    /// bcode → 按键名（与 mouse_proc btn 映射同源）
    fn btn_name(bcode: u8) -> &'static str {
        match bcode {
            0 => "left",
            1 => "right",
            _ => "middle",
        }
    }

    /// 单击合成定时回调：down1 后窗口期到点且仍是本候选 → 返回 click（单击确认）。
    /// 由消息泵 DispatchMessageW 派发 WM_TIMER 时在捕获线程内同步调用——回调零 sleep/阻塞。
    unsafe extern "system" fn click_timer_proc(_hwnd: HWND, _msg: u32, idevent: usize, _time: u32) {
        let wait = match click_wait().lock() {
            Ok(mut g) => match g.as_ref() {
                Some(cw) if cw.timer_id == idevent => g.take(),
                _ => None,
            },
            Err(_) => None,
        };
        if let Some(cw) = wait {
            let fg = cw.fg;
            send_event(CaptureEvent {
                kind: "click".into(),
                button: Some(btn_name(cw.bcode).to_string()),
                x: cw.x,
                y: cw.y,
                wheel_delta: None,
                keys: vec![],
                window_title: fg.title,
                hwnd: fg.hwnd,
                pid: fg.pid,
                process_name: fg.process_name,
                ts_ms: now_ms(),
            });
        }
    }

    /// 滚动合并定时回调：同向合并窗口到点且仍是本累积 → 返回合并 scroll 事件。
    unsafe extern "system" fn scroll_timer_proc(
        _hwnd: HWND,
        _msg: u32,
        idevent: usize,
        _time: u32,
    ) {
        let wait = match scroll_wait().lock() {
            Ok(mut g) => match g.as_ref() {
                Some(sw) if sw.timer_id == idevent => g.take(),
                _ => None,
            },
            Err(_) => None,
        };
        if let Some(sw) = wait {
            let fg = sw.fg;
            send_event(CaptureEvent {
                kind: "scroll".into(),
                button: None,
                x: sw.x,
                y: sw.y,
                wheel_delta: Some(sw.delta),
                keys: vec![],
                window_title: fg.title,
                hwnd: fg.hwnd,
                pid: fg.pid,
                process_name: fg.process_name,
                ts_ms: now_ms(),
            });
        }
    }

    /// 事件发送辅助
    /// Nuphus 自身窗口判据（收敛在 rec_hook 一处）：前台窗口标题含 "Nuphus"。
    /// 与前端 recorderMap.isSelfCapture(/nuphus/i) 同源逻辑；空标题无法可靠判定 → 不忽略。
    fn is_self_window(title: &str) -> bool {
        let t = title.trim();
        if t.is_empty() {
            return false;
        }
        t.to_ascii_lowercase().contains("nuphus")
    }

    fn send_event(ev: CaptureEvent) {
        // 后端自窗口过滤（根治误捕获）：IGNORE_SELF 会话内，用户在 Nuphus 主窗内的点击/按键
        // 一律不返回为捕获事件，继续等待目标窗口操作——「取消」按钮等 Nuphus 内交互不再依赖
        // 事件返回竞态，只走 rec_cancel_current → CANCEL 标志 → capture_once Err(cancelled)。
        if IGNORE_SELF.load(Ordering::SeqCst) && is_self_window(&ev.window_title) {
            return;
        }
        if let Ok(guard) = result_tx().lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(ev);
            }
        }
    }

    /// 捕获线程已退出且结果通道关闭后的终止原因判定（取消 / 超时 / 异常）
    fn termination_reason() -> String {
        if CANCEL.load(Ordering::SeqCst) {
            "capture cancelled".to_string()
        } else if TIMEOUT_FLAG.load(Ordering::SeqCst) {
            "capture timeout".to_string()
        } else {
            "hook 线程异常退出".to_string()
        }
    }

    // ── 鼠标回调 ──
    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let release = || CallNextHookEx(None, code, wparam, lparam);
        if code < 0 {
            return release();
        }
        let msg = wparam.0 as u32;
        let info = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::MSLLHOOKSTRUCT);
        let ts = now_ms();
        let target = TARGET.load(Ordering::SeqCst);

        // 滚轮（同向合并窗口）：首个 wheel 装载 ScrollWait + 200ms 定时器；窗口内同向 wheel
        // 累积 delta 并重置窗口；定时到（停滚）→ 合并事件返回；方向反转 → 立即返回当前累积方向。
        // 定时器由消息泵派发（scroll_timer_proc 在捕获线程内同步回调），hook 回调零 sleep/阻塞。
        if msg == WM_MOUSEWHEEL {
            if target == CaptureKind::Scroll as u32 || target == CaptureKind::Any as u32 {
                let delta = ((info.mouseData >> 16) as u16) as i16 as i32;
                if delta == 0 {
                    return release();
                }
                // IGNORE_SELF：Nuphus 窗内滚动不进入合并（send_event 过滤兜底保留）
                let fg = foreground_info();
                if IGNORE_SELF.load(Ordering::SeqCst) && is_self_window(&fg.title) {
                    return release();
                }
                let cur = match scroll_wait().lock() {
                    Ok(mut g) => g.take(),
                    Err(_) => None,
                };
                match cur {
                    Some(mut sw) if sw.delta != 0 && (sw.delta > 0) == (delta > 0) => {
                        // 同向：累积 delta（起点坐标保持首个 wheel 位置）并重置合并窗口
                        sw.delta += delta;
                        kill_thread_timer(sw.timer_id);
                        sw.timer_id = SetTimer(
                            HWND::default(),
                            0,
                            SCROLL_WINDOW_MS as u32,
                            Some(scroll_timer_proc),
                        );
                        if sw.timer_id == 0 {
                            // 定时器创建失败：不悬挂——立即返回累积结果
                            let fg = sw.fg;
                            send_event(CaptureEvent {
                                kind: "scroll".into(),
                                button: None,
                                x: sw.x,
                                y: sw.y,
                                wheel_delta: Some(sw.delta),
                                keys: vec![],
                                window_title: fg.title,
                                hwnd: fg.hwnd,
                                pid: fg.pid,
                                process_name: fg.process_name,
                                ts_ms: ts,
                            });
                        } else if let Ok(mut g) = scroll_wait().lock() {
                            *g = Some(sw);
                        }
                    }
                    Some(sw) => {
                        // 方向反转：立即返回已累积方向（保留用户已完成的第一段滚动），
                        // 并以反向 wheel 为新累积起点（新起点坐标 = 反转事件位置）
                        kill_thread_timer(sw.timer_id);
                        let fg_old = sw.fg;
                        send_event(CaptureEvent {
                            kind: "scroll".into(),
                            button: None,
                            x: sw.x,
                            y: sw.y,
                            wheel_delta: Some(sw.delta),
                            keys: vec![],
                            window_title: fg_old.title,
                            hwnd: fg_old.hwnd,
                            pid: fg_old.pid,
                            process_name: fg_old.process_name,
                            ts_ms: ts,
                        });
                        let timer_id = SetTimer(
                            HWND::default(),
                            0,
                            SCROLL_WINDOW_MS as u32,
                            Some(scroll_timer_proc),
                        );
                        if timer_id != 0 {
                            if let Ok(mut g) = scroll_wait().lock() {
                                *g = Some(ScrollWait {
                                    timer_id,
                                    x: info.pt.x,
                                    y: info.pt.y,
                                    delta,
                                    fg,
                                });
                            }
                        }
                    }
                    None => {
                        // 首个 wheel：装载累积 + 窗口定时器
                        let timer_id = SetTimer(
                            HWND::default(),
                            0,
                            SCROLL_WINDOW_MS as u32,
                            Some(scroll_timer_proc),
                        );
                        if timer_id == 0 {
                            // 定时器创建失败：不悬挂——按单次 wheel 立即返回
                            send_event(CaptureEvent {
                                kind: "scroll".into(),
                                button: None,
                                x: info.pt.x,
                                y: info.pt.y,
                                wheel_delta: Some(delta),
                                keys: vec![],
                                window_title: fg.title.clone(),
                                hwnd: fg.hwnd,
                                pid: fg.pid,
                                process_name: fg.process_name.clone(),
                                ts_ms: ts,
                            });
                        } else if let Ok(mut g) = scroll_wait().lock() {
                            *g = Some(ScrollWait {
                                timer_id,
                                x: info.pt.x,
                                y: info.pt.y,
                                delta,
                                fg,
                            });
                        }
                    }
                }
            }
            return release();
        }

        // 鼠标按键（down 事件）：单击/双击合成状态机——down1 装载候选 + 双击窗口线程定时器
        // （窗口 = 系统 GetDoubleClickTime，默认 ~500ms；不立即返回）；
        // 窗口内同点 down2 → double_click；定时到（无 down2）→ click。
        // 单次捕获只返回一次事件；等待由线程定时器完成，回调内零 sleep/阻塞。
        let btn: Option<(u8, &str)> = match msg {
            WM_LBUTTONDOWN => Some((0, "left")),
            WM_RBUTTONDOWN => Some((1, "right")),
            WM_MBUTTONDOWN => Some((2, "middle")),
            _ => None,
        };
        if let Some((bcode, _bname)) = btn {
            if target == CaptureKind::Click as u32 || target == CaptureKind::Any as u32 {
                // IGNORE_SELF：Nuphus 窗内 down 不进入合成（避免无谓定时器；send_event 过滤兜底保留）
                let fg = foreground_info();
                if IGNORE_SELF.load(Ordering::SeqCst) && is_self_window(&fg.title) {
                    return release();
                }
                let prev = match click_wait().lock() {
                    Ok(mut g) => g.take(),
                    Err(_) => None,
                };
                if let Some(cw) = prev {
                    if cw.bcode == bcode
                        && (cw.x - info.pt.x).abs() <= 4
                        && (cw.y - info.pt.y).abs() <= 4
                        && ts.saturating_sub(cw.ts) < u64::from(click_window_ms())
                    {
                        // 窗口内同点 down2 → double_click（合成完成，销毁候选定时器）
                        kill_thread_timer(cw.timer_id);
                        let fg2 = cw.fg;
                        send_event(CaptureEvent {
                            kind: "double_click".into(),
                            button: Some(btn_name(bcode).to_string()),
                            x: cw.x,
                            y: cw.y,
                            wheel_delta: None,
                            keys: vec![],
                            window_title: fg2.title,
                            hwnd: fg2.hwnd,
                            pid: fg2.pid,
                            process_name: fg2.process_name,
                            ts_ms: ts,
                        });
                        return release();
                    }
                    // 不同点 / 超窗：旧候选作废（销毁其定时器），以本次 down 为新候选
                    kill_thread_timer(cw.timer_id);
                }
                // down1（或作废后的 down2）：装载候选 + 合成窗口定时器
                let timer_id = SetTimer(
                    HWND::default(),
                    0,
                    click_window_ms(),
                    Some(click_timer_proc),
                );
                if timer_id == 0 {
                    // 定时器创建失败：不悬挂——按单次单击立即返回
                    let fg2 = fg;
                    send_event(CaptureEvent {
                        kind: "click".into(),
                        button: Some(btn_name(bcode).to_string()),
                        x: info.pt.x,
                        y: info.pt.y,
                        wheel_delta: None,
                        keys: vec![],
                        window_title: fg2.title,
                        hwnd: fg2.hwnd,
                        pid: fg2.pid,
                        process_name: fg2.process_name,
                        ts_ms: ts,
                    });
                } else if let Ok(mut g) = click_wait().lock() {
                    *g = Some(ClickWait {
                        timer_id,
                        bcode,
                        x: info.pt.x,
                        y: info.pt.y,
                        ts,
                        fg,
                    });
                }
                return release();
            }
        }
        release()
    }

    // ── 键盘回调：修饰键 + 主键 → 组合键事件 ──
    unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let release = || CallNextHookEx(None, code, wparam, lparam);
        if code < 0 {
            return release();
        }
        let msg = wparam.0 as u32;
        let info = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::KBDLLHOOKSTRUCT);
        let vk = info.vkCode as u32;
        let target = TARGET.load(Ordering::SeqCst);

        // VK codes
        const VK_CONTROL: u32 = 0x11;
        const VK_SHIFT: u32 = 0x10;
        const VK_MENU: u32 = 0x12; // Alt
        const VK_LWIN: u32 = 0x5B;
        const VK_RWIN: u32 = 0x5C;
        let is_mod = matches!(vk, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN);
        let mod_bit = |v: u32| -> u32 {
            if v == VK_CONTROL {
                1
            } else if v == VK_SHIFT {
                2
            } else if v == VK_MENU {
                4
            } else {
                8
            }
        };

        if is_mod {
            match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    MOD_KEYS.fetch_or(mod_bit(vk), Ordering::SeqCst);
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    MOD_KEYS.fetch_and(!mod_bit(vk), Ordering::SeqCst);
                }
                _ => {}
            }
            return release();
        }

        // 主键按下 → 组合当前修饰键
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            if target == CaptureKind::Hotkey as u32 || target == CaptureKind::Any as u32 {
                let mods = MOD_KEYS.load(Ordering::SeqCst);
                let mut keys: Vec<String> = Vec::new();
                if mods & 1 != 0 {
                    keys.push("ctrl".into());
                }
                if mods & 2 != 0 {
                    keys.push("shift".into());
                }
                if mods & 4 != 0 {
                    keys.push("alt".into());
                }
                if mods & 8 != 0 {
                    keys.push("win".into());
                }
                // 主键名：字母/数字/常用功能键
                let name = if (0x41..=0x5A).contains(&vk) {
                    Some(((vk - 0x41 + b'a' as u32) as u8) as char)
                } else if (0x30..=0x39).contains(&vk) {
                    Some(vk as u8 as char)
                } else {
                    match vk {
                        0x0D => Some('⏎'), // enter
                        0x1B => Some('⎋'), // esc
                        0x09 => Some('⇥'), // tab
                        0x20 => Some('␣'), // space
                        0x08 => Some('⌫'), // backspace
                        0x2E => Some('⌦'), // delete
                        0x24 => Some('↖'), // home
                        0x23 => Some('↘'), // end
                        0x21 => Some('⇞'), // pageup
                        0x22 => Some('⇟'), // pagedown
                        0x25 => Some('←'),
                        0x26 => Some('↑'),
                        0x27 => Some('→'),
                        0x28 => Some('↓'),
                        0x70..=0x87 => None, // F1..F24 单独处理
                        _ => None,
                    }
                };
                if let Some(c) = name {
                    keys.push(c.to_string());
                } else if (0x70..=0x87).contains(&vk) {
                    keys.push(format!("f{}", vk - 0x70 + 1));
                } else {
                    return release();
                }

                let fg = foreground_info();
                send_event(CaptureEvent {
                    kind: "hotkey".into(),
                    button: None,
                    x: 0,
                    y: 0,
                    wheel_delta: None,
                    keys,
                    window_title: fg.title,
                    hwnd: fg.hwnd,
                    pid: fg.pid,
                    process_name: fg.process_name,
                    ts_ms: now_ms(),
                });
            }
        }
        release()
    }

    // ── 捕获线程：装 hook + 消息泵 ──
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let capture_thread = std::thread::spawn(move || unsafe {
        // 低层 hook（WH_MOUSE_LL/WH_KEYBOARD_LL）hmod 可为 NULL：
        // 回调运行在当前进程上下文，无需 DLL 句柄。GetModuleHandleW 需要
        // LibraryLoader feature，不引入——用 HINSTANCE::default()（null）。
        let hmod: HINSTANCE = HINSTANCE::default();
        let mouse_hook: Option<HHOOK> = if want(CaptureKind::Click as u32)
            || want(CaptureKind::Scroll as u32)
            || want(CaptureKind::Any as u32)
        {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0).ok()
        } else {
            None
        };
        let key_hook: Option<HHOOK> =
            if want(CaptureKind::Hotkey as u32) || want(CaptureKind::Any as u32) {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), hmod, 0).ok()
            } else {
                None
            };

        if mouse_hook.is_none() && key_hook.is_none() {
            // 两类 hook 都未装上：关闭结果通道让主等待循环确定性返回错误（而非永久空转）
            if let Ok(mut g) = result_tx().lock() {
                *g = None;
            }
            let _ = started_tx.send(());
            return;
        }
        CAPTURE_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        let _ = started_tx.send(());

        let mut msg = MSG::default();
        loop {
            let res = GetMessageW(&mut msg, None, 0, 0);
            // BOOL 0 = WM_QUIT; -1 = error
            if res.0 == 0 {
                break;
            }
            DispatchMessageW(&msg);
        }

        // 销毁未触发的合成窗口定时器（正常/取消/超时三路统一收尾）。线程定时器随本线程退出
        // 会自动失效，显式 KillTimer 是为防止收尾竞态内队列残留 WM_TIMER 再次派发回调。
        if let Ok(mut g) = click_wait().lock() {
            if let Some(cw) = g.take() {
                kill_thread_timer(cw.timer_id);
            }
        }
        if let Ok(mut g) = scroll_wait().lock() {
            if let Some(sw) = g.take() {
                kill_thread_timer(sw.timer_id);
            }
        }

        // 卸载（正常/取消/超时三路统一在此）
        if let Some(h) = mouse_hook {
            let _ = UnhookWindowsHookEx(h);
        }
        if let Some(h) = key_hook {
            let _ = UnhookWindowsHookEx(h);
        }
        CAPTURE_THREAD_ID.store(0, Ordering::SeqCst);
        // 关闭结果通道让主等待循环确定性收尾（rx 收到 Disconnected）。
        // 注意：不复位 CANCEL——终止原因由主循环在 Disconnected 后按 CANCEL/TIMEOUT_FLAG
        // 判定，残留标志在下次 capture_once_windows 开头统一重置，避免取消/超时竞态悬空。
        if let Ok(mut g) = result_tx().lock() {
            *g = None;
        }
    });

    // 等待 hook 装好（3s 上限；失败则立即返回错误）
    match started_rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(()) => {}
        Err(_) => {
            rec_cancel_current();
            let _ = capture_thread.join();
            return Err("hook 安装失败或超时".to_string());
        }
    }

    // 超时 watch 线程：到点先置超时标志再唤醒泵（主循环据此区分「超时」与「异常退出」）。
    // 分段 sleep + 停止信号：收到事件/取消提前结束时让 watch 立即退出，避免 join 阻塞到原
    // 超时点——旧实现 watch.join() 无条件等满 timeout 秒，成功捕获/取消都要空等数十秒才返回
    // （「点了动作没反应 / 取消无效」的直接观感来源，退出链路断裂）。
    let timeout = if timeout_secs == 0 { 60 } else { timeout_secs };
    let (watch_stop_tx, watch_stop_rx) = mpsc::channel::<()>();
    let watch = {
        let timeout = timeout;
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let remain = deadline - now;
                let wait = remain.min(std::time::Duration::from_millis(50));
                match watch_stop_rx.recv_timeout(wait) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return, // 提前停止
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                }
            }
            TIMEOUT_FLAG.store(true, Ordering::SeqCst);
            wake_pump();
        })
    };

    // 阻塞等待事件（含取消：CANCEL 置位也会 wake_pump，但 rx 无消息 → recv_timeout 判定）
    let recv_timeout = std::time::Duration::from_secs(timeout + 5);
    let result = loop {
        if CANCEL.load(Ordering::SeqCst) {
            break Err("capture cancelled".to_string());
        }
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(ev) => break Ok(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // 捕获线程已退出且通道已关闭：按标志判定终止原因（取消/超时/异常），
                // 保证 rec_cancel / 超时 watch 都确定性地结束等待（此前依赖 CANCEL 检查竞态，
                // 捕获线程退出时即复位，主循环可能永久空转——「取消无效/超时不返回」根因之一）。
                break Err(termination_reason());
            }
        }
    };

    // 结束捕获线程（收到事件/取消/超时后立即唤醒退出，避免钩子滞留）
    let _ = recv_timeout;
    wake_pump();
    drop(watch_stop_tx); // 通知 watch 提前退出（若仍在睡眠），join 不会阻塞到原超时点
    let _ = watch.join();
    let _ = capture_thread.join();
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::click_window_ms;

    #[test]
    fn double_click_window_respects_os_setting() {
        // GetDoubleClickTime 语义区间：Windows 设置允许 100–5000ms，系统未配置时回退 500。
        let ms = click_window_ms();
        assert!(
            (100..=5000).contains(&ms),
            "click_window_ms() 应落在系统双击速度区间，实际 {ms}"
        );
    }
}
