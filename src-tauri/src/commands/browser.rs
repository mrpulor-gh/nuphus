// browser.rs — 应用内浏览器：一个 Window + 两个子 webview（壳 frame + 内容 content）。
//
// 架构（2026-10-08 起重构，规范见 docs/browser-shell-arch.md）：
//
//   ┌───────────────────────────────────────────────┐
//   │ 原生 title 栏（系统，关闭/最小化/最大化照常）        │
//   ├───────────────────────────────────────────────┤
//   │ 子 webview A「frame」  browser-frame.html      │ ← 固定 44px，我们的 React 控制器
//   ├───────────────────────────────────────────────┤
//   │ 子 webview B「content」 External(远程 URL)      │ ← 填满剩余，注入 overlay 脚本
//   └───────────────────────────────────────────────┘
//
// window label = `browser-<n>`；两个子 webview label 分别加 `-frame` / `-content` 后缀。
// 远程页面的安全边界（http/https 白名单、incognito、禁新窗、禁下载、无 Tauri IPC 桥）
// 原样保留，只是从「window 级」下沉到「content webview 级」。
//
// ⚠️ 本模块的窗口**不是** WebviewWindow：window label 下没有任何同名 webview。
// tauri 2.11.5 的 `get_webview_window(label)` 先按 label 找 webview（lib.rs:576），
// 找不到直接返回 None —— 因此**取不到** WindowBuilder 建的窗口，关窗必须走
// `app.get_window(label)`（已核对 tauri-2.11.5 源码，非假设）。
// 同理 `app.webview_windows()` 也列不出它（`is_webview_window` 要求全部子 webview
// 与 window 同 label），列窗口只能用 `app.windows()`。
//
// 壳 ↔ content 跨源通信硬约束：frame 是 tauri:// 本地页、content 是远程页，
// 不同源不能 postMessage，一切走 invoke → Rust → emit（见架构文档第三节）。
//
// 死锁纪律（wry#583，勿破）：`browser_open` 保持 `pub async fn`，建窗走
// spawn_blocking + run_on_main_thread + channel。`Window::add_child` 内部同样
// 走 run_on_main_thread，但 tauri-runtime-wry 的 `send_user_message` 检测到
// 已在主线程时**内联执行**闭包（不会自锁），故主线程内连续 add_child 安全
// （已核对 tauri-runtime-wry-2.11.4 源码，非假设）。
//
// 已知限制（勿误认为已实现防护）：Tauri 2.11.5 **没有** `on_permission_request`
// 这一 API（`webview/mod.rs` 中不存在，权限枚举也不存在），因此相机 / 麦克风 /
// 通知等权限请求无法在应用层逐项 Deny，只能由 WebView2 默认策略处理。
// 本模块不声称「权限全拒」——那会给出虚假的安全保证。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent},
    AppHandle, Emitter, Manager, Url, WebviewUrl,
};

/// 事件名：主窗口 `listen('browser://event')` 订阅。
/// 事件名允许字母数字与 `-` / `/` / `:` / `_`（tauri 2.11.5 `event::EventName`）。
pub const BROWSER_EVENT: &str = "browser://event";

/// 窗口 label 前缀。实际 label = `browser-<递增序号>`（见 `next_label`）。
const LABEL_PREFIX: &str = "browser-";
/// 壳 webview label 后缀：`browser-<n>-frame`。
const FRAME_SUFFIX: &str = "-frame";
/// 内容 webview label 后缀：`browser-<n>-content`。
const CONTENT_SUFFIX: &str = "-content";
/// 壳 webview 固定高度（逻辑像素）：地址栏 + 导航按钮 + 录制/标注开关 + tab 列表。
const FRAME_HEIGHT: f64 = 44.0;
/// 壳页面在 frontendDist 下的入口（前端任务负责创建，Rust 只认这个 URL）。
const FRAME_PAGE: &str = "browser-frame.html";

/// 新窗口默认尺寸（逻辑像素）。
const DEFAULT_WIDTH: f64 = 1100.0;
const DEFAULT_HEIGHT: f64 = 800.0;
/// 最小尺寸：低于此值页面操作区会挤到不可用。
const MIN_WIDTH: f64 = 480.0;
const MIN_HEIGHT: f64 = 360.0;
/// 初始标题；页面 `on_document_title_changed` 到达后被真实标题覆盖。
const INITIAL_TITLE: &str = "Nuphus 浏览器";

/// URL 长度上限（对齐 external.rs 的 2048），防超长串撑爆事件负载。
const MAX_URL_LEN: usize = 2048;
/// 历史栈上限：单窗口长期浏览会无界增长，超限截断**头部**而非尾部
/// （保住最近的历史 —— 那才是用户真正要退回的几页）。
const MAX_HISTORY: usize = 100;

/// content webview 的初始化脚本（document_start 注入，主框架）。
///
/// 现阶段直接复用 preview:// 标注链路的同一份脚本常量，**不手抄**
/// （后续 `sync-browser-scripts.mjs` 把标注 + 录制合一后换成本模块常量）。
/// 脚本非编辑态零干扰（P4 铁律），当前壳页面未接线前始终保持惰性：
/// 它的下行指令只认 `window.parent` 且必须先收到 `nuphus:annotate-mode`，
/// 而 content webview 里 `window.parent === window`，无人下发即永不激活。
const BROWSER_OVERLAY_SCRIPT: &str = crate::preview_protocol::ANNOTATION_OVERLAY_SCRIPT;

// ── 事件负载 ────────────────────────────────────────────────────────────────

/// 事件 `type` 取值。`kebab-case` 后与前端 `BrowserEvent` 判别联合逐字对齐。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserEventKind {
    /// 页面开始加载（on_page_load Started）。
    Loading,
    /// 页面加载完成（on_page_load Finished）。
    Loaded,
    /// 文档标题变化。
    TitleChanged,
    /// 导航完成 —— 前进/后退按钮可用性随此事件刷新。
    Navigated,
    /// 窗口已关闭（含用户手点关闭）。
    Closed,
    /// 导航被拒 / 跳转失败。
    Error,
}

/// 发往主窗口的事件负载。字段 camelCase 与前端契约一致。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserEvent {
    #[serde(rename = "type")]
    pub kind: BrowserEventKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_go_back: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_go_forward: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BrowserEvent {
    fn new(kind: BrowserEventKind, label: &str) -> Self {
        Self {
            kind,
            label: label.to_string(),
            url: None,
            title: None,
            can_go_back: None,
            can_go_forward: None,
            message: None,
        }
    }

    fn with_url(mut self, url: &str) -> Self {
        self.url = Some(url.to_string());
        self
    }

    fn loading(label: &str, url: &str) -> Self {
        Self::new(BrowserEventKind::Loading, label).with_url(url)
    }

    fn loaded(label: &str, url: &str) -> Self {
        Self::new(BrowserEventKind::Loaded, label).with_url(url)
    }

    fn title_changed(label: &str, title: &str) -> Self {
        let mut event = Self::new(BrowserEventKind::TitleChanged, label);
        event.title = Some(title.to_string());
        event
    }

    fn navigated(label: &str, url: &str, can_go_back: bool, can_go_forward: bool) -> Self {
        let mut event = Self::new(BrowserEventKind::Navigated, label).with_url(url);
        event.can_go_back = Some(can_go_back);
        event.can_go_forward = Some(can_go_forward);
        event
    }

    fn closed(label: &str) -> Self {
        Self::new(BrowserEventKind::Closed, label)
    }

    fn error(label: &str, message: &str) -> Self {
        let mut event = Self::new(BrowserEventKind::Error, label);
        event.message = Some(message.to_string());
        event
    }
}

/// `browser_get_state` 返回值
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// `browser_list_windows` 返回值：一个活跃浏览窗口一行的快照（壳恢复 tab 用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserWindowInfo {
    pub label: String,
    pub title: String,
    pub url: String,
}

// ── 会话记账 ────────────────────────────────────────────────────────────────

/// 一次历史转移的方向（后退/前进），用于失败时把光标回滚原位。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Back,
    Forward,
}

/// 单窗口的 URL 历史 + 导航中间态 + 展示态。
///
/// 模拟 `history[cursor]`，`history[cursor]` 即当前页。
/// `can_go_back = cursor > 0`，`can_go_forward = cursor + 1 < history.len()`。
/// `history` 为空表示**未曾导航**过（无加载状态；此时两者皆不可用）。
#[derive(Default)]
struct Session {
    history: Vec<String>,
    /// 当前页下标。`history` 为空时无效。
    cursor: usize,
    /// 由**我们**发起的一次转移（后退/前进），预登记期望到达的 URL。
    ///
    /// 因为 `on_page_load(Started)` 不知道这次导航是**前进/后退**还是用户点了
    /// 新链接，若不做登记，前进/后退也会被当成新访问压进历史栈，栈越点越乱。
    ///
    /// 用 `URL 相等` 而非 bool，是因为一次用户点击可能产生与期望 URL 不符的
    /// 跳转（重定向/拦截），此时自动降级为普通导航，不残留脏标记。
    pending: Option<(Step, String)>,
    loading: bool,
    /// 最近一次 `on_document_title_changed` 的标题。
    /// WebviewWindow 没了直接的 `title()` 取值路径（新架构是 Window + 子 webview），
    /// `browser_get_state` / `browser_list_windows` 从这里读。
    title: String,
    /// 录制动作流（`browser_record_action` 压入，壳页面取走）。
    /// 真录制脚本接入前先占位为空流，命令契约先行。
    recording: Vec<serde_json::Value>,
}

impl Session {
    fn can_go_back(&self) -> bool {
        self.cursor > 0 && !self.history.is_empty()
    }

    fn can_go_forward(&self) -> bool {
        !self.history.is_empty() && self.cursor + 1 < self.history.len()
    }

    /// 记一页（新导航 / 重定向 / 重复点击）。
    fn visit(&mut self, url: &str) {
        // 前进之后的历史全废，新分支不支持前进
        self.history.truncate(self.cursor + 1);
        if self.history.last().map(String::as_str) == Some(url) {
            // 同一页重复加载，不制造重复条目
            return;
        }
        self.history.push(url.to_string());
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.cursor = self.history.len() - 1;
    }

    /// 后退一条，返回目标 URL；栈底时返回 None。
    fn step_back(&mut self) -> Option<String> {
        if !self.can_go_back() {
            return None;
        }
        self.cursor -= 1;
        let target = self.history[self.cursor].clone();
        self.pending = Some((Step::Back, target.clone()));
        Some(target)
    }

    /// 前进一条，返回目标 URL；栈顶时返回 None。
    fn step_forward(&mut self) -> Option<String> {
        if !self.can_go_forward() {
            return None;
        }
        self.cursor += 1;
        let target = self.history[self.cursor].clone();
        self.pending = Some((Step::Forward, target.clone()));
        Some(target)
    }

    /// 撤销一次 `step_back/step_forward` 的光标移动——那一跳最终没发生时压回**原位**。
    ///
    /// 为什么不是简单地把 cursor 减 1：后退失败要回到「后面一页」、前进失败要回到
    /// 「前面一页」，方向相反。若写同一个加减，某一次跳转失败后历史导航就会停在一个
    /// 并没有真正到达的页上；之后每次前进/后退都从错误位置起跳。
    fn rollback(&mut self) {
        let Some((step, _)) = self.pending.take() else {
            return;
        };
        match step {
            // 后退失败 → 回到后面一页
            Step::Back => self.cursor = (self.cursor + 1).min(self.history.len() - 1),
            // 前进失败 → 回到前面一页
            Step::Forward => self.cursor = self.cursor.saturating_sub(1),
        }
    }

    /// 检查这页是「我们发起的前进/后退」刚刚落地，还是一次真实的新导航。
    ///
    /// 返回 `true` 表示该跳转是 step_* 发起、`pending` 标记已消费，调用方**不要**
    /// 追加历史；`false` 表示是用户实锚的新导航（点链接 / 地址栏回车），需要 `visit`
    /// 追加一页。
    fn take_pending(&mut self, url: &str) -> bool {
        match &self.pending {
            Some((_, expected)) if expected == url => {
                self.pending = None;
                true
            }
            // URL 对不上，或根本没有待消费的 pending（重定向等）→ 按新导航处理，
            // 标记同样作废，不残留脏状态
            _ => {
                self.pending = None;
                false
            }
        }
    }
}

/// 全局记账表：label → Session。窗口关闭时移除（见 `forget`）。
fn sessions() -> &'static Mutex<Vec<(String, Session)>> {
    static SESSIONS: OnceLock<Mutex<Vec<(String, Session)>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(Vec::new()))
}

/// label 分配器。
///
/// 计数是**进程级**的，且单调递增：关闭窗口后 label 不复用。Tauri 的窗口/ webview
/// 注册表按 label 索引，同一 label 循环复用会让尚未注销的旧条目顶掉新窗口。
/// 用「进程级计数 + 关闭即忘」换取稳定唯一，代价只是 label 数字单调变大。
fn next_label() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed) + 1;
    format!("{LABEL_PREFIX}{n}")
}

/// 带副作用地取 label 对应 session，不存在则就地新建空 session。
///
/// 为何用 `into_inner()` 恢复中毒锁：Mutex 中毒只意味着**某个**持锁者 panic 了，
/// 但我们的临界区全是内存里的 Vec/String 操作、不涉及跨线程不变式，
/// 数据仍是一致的。直接 unwrap 会把一次无关 panic 级联成整个浏览器功能瘫痪
/// （与 `hud.rs` 的 `HUD_USER_POS` 同一取舍）。
fn with_session<R>(label: &str, f: impl FnOnce(&mut Session) -> R) -> R {
    let mut guard = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let slot = match guard.iter().position(|(l, _)| l == label) {
        Some(i) => i,
        None => {
            guard.push((label.to_string(), Session::default()));
            guard.len() - 1
        }
    };
    f(&mut guard[slot].1)
}

/// 读取**已存在**的 session，不就地新建；label 无记录时返回 None。
///
/// get/clear 录音这类只读/清理入口必须安静返回空：一次对不存在窗口的调用
/// 就在全局表里留一份永不 `forget` 的空记账（僵尸条目）。
fn with_existing_session<R>(label: &str, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    let mut guard = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let slot = guard.iter().position(|(l, _)| l == label)?;
    Some(f(&mut guard[slot].1))
}

/// 注销某窗口的记账，窗口销毁时调用。
fn forget(label: &str) {
    let mut guard = sessions().lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|(l, _)| l != label);
}

// ── 事件广播 ────────────────────────────────────────────────────────────────

/// 广播一条浏览事件到主窗口。失败只记日志，不炸调用方（主窗口可能尚未就绪）。
fn emit(app: &AppHandle, event: BrowserEvent) {
    if let Err(e) = app.emit_to("main", BROWSER_EVENT, event) {
        tracing::warn!("[Browser] 事件广播失败（{BROWSER_EVENT}）: {e}");
    }
}

// ── 校验 ────────────────────────────────────────────────────────────────────

/// 是否允许的远程目标。
///
/// 复用 external.rs 的策略：只放行 http/https。`file:` 会让远程页读到本地磁盘，
/// `javascript:` 是代码执行入口，二者都不放行。缺 host 的（如 `https://`）同样拒绝。
fn is_allowed_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty())
}

/// 解析并校验远程 URL（纯函数，便于单测）。
///
/// 拒绝：空串、超长、含控制字符、无法解析、非 http/https、缺 host。
/// `Err` 带可读原因，前端直接展示即可。
pub fn parse_remote_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL 为空".to_string());
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(format!("URL 过长（上限 {MAX_URL_LEN} 字节）"));
    }
    if trimmed.chars().any(char::is_control) {
        return Err("URL 含控制字符".to_string());
    }
    let url = Url::parse(trimmed).map_err(|e| format!("URL 无法解析：{e}"))?;
    if !is_allowed_url(&url) {
        return Err("仅允许 http/https 链接".to_string());
    }
    Ok(url)
}

// ── label 工具 ──────────────────────────────────────────────────────────────

/// 壳 webview 的 label（window label + `-frame`）。
fn frame_label(label: &str) -> String {
    format!("{label}{FRAME_SUFFIX}")
}

/// 内容 webview 的 label（window label + `-content`）。
fn content_label(label: &str) -> String {
    format!("{label}{CONTENT_SUFFIX}")
}

/// 取窗口 label 对应的**内容** webview（所有驱动命令的落点）。
fn content_webview<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &str,
) -> Result<tauri::Webview<R>, String> {
    app.get_webview(&content_label(label))
        .ok_or_else(|| format!("浏览窗口不存在：{label}"))
}

/// 是否是浏览窗口的 window label。
///
/// `app.windows()` 只含窗口、永不含 webview label，但 `-frame` / `-content`
/// 后缀判断保留：label 前缀相同的一屏过滤器，壳/content 混进来就是脏数据，
/// 且给未来的多窗口/tab 检索留一道显式防线（不靠"现在恰好没有"）。
fn is_browser_window_label(label: &str) -> bool {
    label.strip_prefix(LABEL_PREFIX).is_some_and(|rest| {
        !rest.is_empty() && !rest.ends_with(FRAME_SUFFIX) && !rest.ends_with(CONTENT_SUFFIX)
    })
}

// ── 布局 ────────────────────────────────────────────────────────────────────

/// 由「左上角 + 逻辑尺寸」构造 webview 边界。
fn logical_rect(x: f64, y: f64, width: f64, height: f64) -> tauri::Rect {
    tauri::Rect {
        position: tauri::Position::Logical(tauri::LogicalPosition::new(x, y)),
        // 边界钳位：负尺寸会让 set_bounds 直接报错，把这一行炸成 resize 事件异常。
        // min_inner_size 已保证 height >= MIN_HEIGHT，这里仍是显式防御而非假设。
        size: tauri::Size::Logical(tauri::LogicalSize::new(width.max(0.0), height.max(0.0))),
    }
}

/// 按窗口当前客户区重算两个子 webview 的边界：壳固定 44px，内容填剩余。
///
/// `WebviewBuilder::auto_resize` 只能**等比缩放**，做不到「A 固定高、B 填剩余」，
/// 所以刻意不启用 auto_resize，布局完全由本函数 + resize 事件驱动。
fn relayout(window: &tauri::Window, frame: &tauri::Webview, content: &tauri::Webview) {
    // inner_size 是物理像素；窗口可能跨 DPI 不同的屏幕，必须用当前 scale_factor
    // 换算回逻辑像素，否则 150% 缩放下壳会被拉到 66px、内容区整体偏移。
    let Ok(inner) = window.inner_size() else {
        tracing::warn!("[Browser] 读取窗口客户区尺寸失败，跳过本轮布局");
        return;
    };
    let scale = match window.scale_factor() {
        Ok(s) if s > 0.0 => s,
        _ => {
            tracing::warn!("[Browser] 读取窗口缩放因子失败，跳过本轮布局");
            return;
        }
    };
    let width = inner.width as f64 / scale;
    let height = inner.height as f64 / scale;
    // 失败只警告不中断：resize 事件是高频的，一次 set_bounds 失败（极端尺寸）
    // 不该让整个事件处理器panic——但布局会停在旧值，靠下一次 resize 自愈。
    if let Err(e) = frame.set_bounds(logical_rect(0.0, 0.0, width, FRAME_HEIGHT)) {
        tracing::warn!("[Browser] 壳 webview 布局失败: {e}");
    }
    if let Err(e) = content.set_bounds(logical_rect(
        0.0,
        FRAME_HEIGHT,
        width,
        height - FRAME_HEIGHT,
    )) {
        tracing::warn!("[Browser] 内容 webview 布局失败: {e}");
    }
}

// ── 命令 ────────────────────────────────────────────────────────────────────

/// 打开一个应用内浏览窗口，返回其 label。
///
/// 远程页面与主窗口完全隔离：无 Tauri IPC 桥、无本地资源访问。
/// 历史不由本函数预置：首条历史由首次 `on_page_load(Started)` 落地，
/// 这样窗口标题栏显示的 URL 与实际（含重定向后）一致。
#[tauri::command]
/// ⚠️ 必须是 async：Tauri 文档明确警告（`WebviewWindowBuilder::new` 的 Known issues 段，
/// 对应 wry#583）——**Windows 上在同步命令或事件处理器里调 `WebviewWindowBuilder::build()`
/// 会死锁**。2026-10-08 实机事故：browser_open 写成同步命令时，主窗口 IPC 调用被
/// WebView2 窗口创建卡住，表现为「窗口弹出但无页面 / 无法关闭 / 主对话窗口完全冻结，
/// 只能结束进程」。改成 async 后建窗在 async runtime 线程执行，不再触碰死锁路径。
pub async fn browser_open(app: AppHandle, url: String) -> Result<String, String> {
    let target = parse_remote_url(&url)?;
    let label = next_label();
    let target_for_build = target.clone();

    // 建窗走 run_on_main_thread：Tauri 文档明确警告（`WebviewWindowBuilder::new`
    // 的 Known issues 段，对应 wry#583）——Windows 上在同步命令或事件处理器里调
    // `build()` 会死锁。2026-10-08 实机事故：写成同步命令时主窗口 IPC 调用被
    // WebView2 建窗卡住，表现为「窗口弹出但无页面 / 无法关闭 / 主对话窗口完全冻结，
    // 只能结束进程」。这里双保险：命令本身是 async + 建窗显式绕回主线程。
    let app_for_build = app.clone();
    let label_for_build = label.clone();
    let window = tauri::async_runtime::spawn_blocking(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let app_for_main = app_for_build.clone();
        let label_for_main = label_for_build.clone();
        app_for_build
            .run_on_main_thread(move || {
                let result = build_browser_window(&app_for_main, &label_for_main, target_for_build);
                let _ = tx.send(result);
            })
            .expect("[Browser] run_on_main_thread 派发失败");
        rx.recv().expect("[Browser] 建窗结果通道已断开")
    })
    .await
    .map_err(|e| format!("建窗任务异常：{e}"))??;

    // 建窗后补挂窗口事件：用户手点关闭时通知主窗口并回收记账。
    // （Destroyed 之后 window 句柄已失效，故只能在此处用 label 记账。）
    let app_closed = app.clone();
    let label_for_events = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            forget(&label_for_events);
            emit(&app_closed, BrowserEvent::closed(&label_for_events));
        }
    });

    Ok(label)
}

/// 真正创建浏览窗口（在主线程执行，见 browser_open 的死锁注释）。
///
/// 返回 window 句柄供调用方挂窗口事件；两个子 webview 由本函数内的 resize
/// 处理器持有（bounds 随窗口客户区重算），随窗口生命周期存亡。
fn build_browser_window(
    app: &AppHandle,
    label: &str,
    target: Url,
) -> Result<tauri::Window, String> {
    // ① 纯窗口（不带 webview）：装饰/标题栏完全交给系统，
    // 关闭/最小化/最大化/拖拽/resize 全部是原生行为，无需自绘。
    let window = tauri::window::WindowBuilder::new(app, label)
        .title(INITIAL_TITLE)
        .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .decorations(true)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| format!("创建浏览窗口失败：{e}"))?;

    // ② 壳 webview（控制器）：App URL，顶部固定 44px。
    //    页面本体由前端任务创建（frontend/browser-frame.html），此处只挂 URL。
    let frame = window
        .add_child(
            tauri::webview::WebviewBuilder::new(
                frame_label(label),
                WebviewUrl::App(FRAME_PAGE.into()),
            ),
            tauri::LogicalPosition::new(0.0, 0.0),
            tauri::LogicalSize::new(DEFAULT_WIDTH, FRAME_HEIGHT),
        )
        .map_err(|e| format!("创建浏览器壳页面失败：{e}"))?;

    // ③ 内容 webview（远程页）：External URL，注入标注 overlay 脚本。
    //    所有页面级回调全部挂在**这个** webview 上（不是 window）——
    //    on_page_load / 标题变化 / 导航 / 新窗 / 下载都是 content 的事件。
    //    每个闭包各自持有一份 label / AppHandle 克隆：`move` 会把变量交出去，
    //    多个闭包共用同一个 `label` 会 move 一次就耗尽，后面几个直接编译不过。
    let app_nav = app.clone();
    let label_nav = label.to_string();
    // on_new_window 专用：它需要同时持有 AppHandle（取 content webview）与 label
    let app_nav_blank = app.clone();
    let label_nav_blank = label.to_string();
    let app_title = app.clone();
    let label_title = label.to_string();
    let app_load = app.clone();
    let label_load = label.to_string();
    let label_download = label.to_string();

    let content = window
        .add_child(
            tauri::webview::WebviewBuilder::new(content_label(label), WebviewUrl::External(target))
                // 隐私模式：远程页面的 cookie/localStorage 不落用户主 profile。
                .incognito(true)
                // 标注 overlay（document_start、主框架）。录制脚本合并注入是下一步，
                // 当前常量即 preview:// 链路那份，惰性零干扰（见常量注释）。
                .initialization_script(BROWSER_OVERLAY_SCRIPT)
                // 只放行 http/https：页内 file:/javascript: 跳转一律拒绝并回报主窗口。
                // 注意：这里放行一切 http(s)，所以「页面内链接点了没反应」不是它拦的
                // （2026-10-08 一度误判，特此留注）。
                .on_navigation(move |url| {
                    if is_allowed_url(url) {
                        true
                    } else {
                        tracing::warn!("[Browser] 拒绝导航到非 http(s) 目标: {url}（{label_nav}）");
                        emit(
                            &app_nav,
                            BrowserEvent::error(&label_nav, "仅允许 http/https 导航，已阻止"),
                        );
                        false
                    }
                })
                // `target=_blank` / `window.open`：浏览窗口**一律不新开**。
                // 但 Deny 会被 WebView2 吞掉点击（页面无响应、无反馈），所以这里走
                // 「先 Deny，再把 URL 交给当前 content webview navigate」——既兑现
                // 「不凭空开窗」，又不吃掉用户点击。URL 又过一遍 is_allowed_url（双保险）。
                .on_new_window(move |url, _features| {
                    tracing::warn!("[Browser] 新窗口请求转当前页导航: {url}（{label_nav_blank}）");
                    if is_allowed_url(&url) {
                        if let Ok(w) = content_webview(&app_nav_blank, &label_nav_blank) {
                            if let Err(e) = w.navigate(url.clone()) {
                                tracing::warn!(
                                    "[Browser] 新窗口转导航失败: {url}（{label_nav_blank}）: {e}"
                                );
                                emit(
                                    &app_nav_blank,
                                    BrowserEvent::error(
                                        &label_nav_blank,
                                        &format!("打开失败：{e}"),
                                    ),
                                );
                            }
                        }
                    } else {
                        emit(
                            &app_nav_blank,
                            BrowserEvent::error(&label_nav_blank, "仅允许 http/https 链接，已阻止"),
                        );
                    }
                    NewWindowResponse::Deny
                })
                .on_document_title_changed(move |webview, title| {
                    // 同步窗口标题栏，多窗口并存时用户能分辨
                    if let Err(e) = webview.window().set_title(&title) {
                        tracing::warn!("[Browser] 设置窗口标题失败: {e}");
                    }
                    // 标题同时进 session：Webview 没有 title() 取值路径，
                    // browser_get_state / browser_list_windows 从这里读。
                    with_session(&label_title, |s| s.title = title.clone());
                    emit(
                        &app_title,
                        BrowserEvent::title_changed(&label_title, &title),
                    );
                })
                .on_page_load(move |_webview, payload| {
                    let url = payload.url().as_str().to_string();
                    // 先取出事件（PageLoadEvent 是 Copy），再进闭包记账：
                    // 避免 payload 的借用跨越 with_session 的锁作用域
                    let event = payload.event();
                    // 用**窗口 label** 记账（session 的 key 是 window label）。
                    // 回调收到的是 content webview（label 带 -content 后缀），
                    // 拿它的 label() 去查表会永远建出一份新空 session。
                    let (suppress, back, forward) = with_session(&label_load, |s| {
                        let suppress = s.take_pending(&url);
                        match event {
                            PageLoadEvent::Started => {
                                s.loading = true;
                                // 前进/后退是我们自己发起的 → 不重复记账（否则后退一次
                                // 又把自己压回栈里，栈越点越乱）
                                if !suppress {
                                    s.visit(&url);
                                }
                            }
                            PageLoadEvent::Finished => s.loading = false,
                        }
                        (suppress, s.can_go_back(), s.can_go_forward())
                    });
                    match event {
                        PageLoadEvent::Started => {
                            emit(&app_load, BrowserEvent::loading(&label_load, &url))
                        }
                        PageLoadEvent::Finished => {
                            emit(&app_load, BrowserEvent::loaded(&label_load, &url))
                        }
                    }
                    // 导航完成的权威信号：前进/后退可用性以此为准。
                    // 一次 Started 最多发一条 navigated，不重复刷同一状态。
                    if event == PageLoadEvent::Finished || !suppress {
                        emit(
                            &app_load,
                            BrowserEvent::navigated(&label_load, &url, back, forward),
                        );
                    }
                })
                // 隐私窗口下拒绝下载落盘，避免远程页面把文件写进用户目录。
                // DownloadEvent 是 #[non_exhaustive]，必须留通配分支；tauri 未来新增的
                // 变体同样代表「有下载动作」，按拒绝处理（fail-closed），
                // 否则新变体会静默变成放行。
                .on_download(move |_webview, event| match event {
                    tauri::webview::DownloadEvent::Requested { url, .. } => {
                        tracing::warn!("[Browser] 已拒绝下载: {url}（{label_download}）");
                        false
                    }
                    // 返回值对 Finished 不生效（tauri-runtime-wry 丢弃它），true 只是表意
                    tauri::webview::DownloadEvent::Finished { .. } => true,
                    _ => {
                        tracing::warn!("[Browser] 已拒绝未知下载事件（{label_download}）");
                        false
                    }
                }),
            tauri::LogicalPosition::new(0.0, FRAME_HEIGHT),
            tauri::LogicalSize::new(DEFAULT_WIDTH, (DEFAULT_HEIGHT - FRAME_HEIGHT).max(0.0)),
        )
        .map_err(|e| format!("创建浏览器内容页面失败：{e}"))?;

    // ④ resize / DPI 变化 → 重算两个 webview 的 bounds。
    //    add_child 给的初始 bounds 是逻辑常量，实际客户区以 inner_size() 为准
    //    （不同平台装饰边距、DPI 都会有偏差），所以建窗后先强制对齐一次首帧。
    relayout(&window, &frame, &content);
    let window_for_resize = window.clone();
    let frame_for_resize = frame.clone();
    let content_for_resize = content.clone();
    window.on_window_event(move |event| {
        // Resized 覆盖拖拽边缘 / 最大化 / 还原；ScaleFactorChanged 覆盖跨 DPI
        // 屏幕拖动与系统缩放变更（该类事件必然伴随新 inner_size，一并重算）。
        if matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
        ) {
            relayout(&window_for_resize, &frame_for_resize, &content_for_resize);
        }
    });

    Ok(window)
}

/// 读取某窗口的当前状态。窗口不存在时返回错误（前端据此清空地址栏）。
#[tauri::command]
pub fn browser_get_state(app: AppHandle, label: String) -> Result<BrowserState, String> {
    let content = content_webview(&app, &label)?;
    let url = content.url().map(|u| u.to_string()).unwrap_or_default();
    let (title, loading, can_go_back, can_go_forward) = with_session(&label, |s| {
        (
            if s.title.is_empty() {
                INITIAL_TITLE.to_string()
            } else {
                s.title.clone()
            },
            s.loading,
            s.can_go_back(),
            s.can_go_forward(),
        )
    });
    Ok(BrowserState {
        url,
        title,
        loading,
        can_go_back,
        can_go_forward,
    })
}

/// 关闭指定浏览窗口。
///
/// ⚠️ 不能用 `app.get_webview_window(label)`：新架构下 window label 下没有同名
/// webview（子 webview 叫 `<label>-frame` / `<label>-content`），tauri 2.11.5 的
/// `get_webview_window` 按 label 找 webview 找不到即 None（已核对源码）。
/// 关**整个 Window**，两个子 webview 随窗口一并销毁，Destroyed 事件触发记账回收。
///
/// 窗口已不存在视为成功（幂等）：前端关闭按钮与用户手点 × 可能重复触发。
#[tauri::command]
pub fn browser_close(app: AppHandle, label: String) -> Result<(), String> {
    match app.get_window(&label) {
        Some(window) => window
            .close()
            .map_err(|e| format!("关闭浏览窗口失败：{e}"))?,
        None => forget(&label),
    }
    Ok(())
}

/// 在已有窗口内跳转到新 URL（地址栏回车）。
#[tauri::command]
pub fn browser_navigate(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let target = parse_remote_url(&url)?;
    let content = content_webview(&app, &label)?;
    // 刻意**不**登记 pending_url：手动导航是一次真实的新访问，必须进历史栈
    // （否则地址栏输入的页面永远进不了历史，前进/后退行为与用户预期不符）。
    // 历史由随后的 on_page_load(Started) 落地；跳转失败时历史自然不动。
    content
        .navigate(target)
        .map_err(|e| format!("跳转失败：{e}"))
}

/// 刷新当前页。不记历史（reload 不是新导航）。
#[tauri::command]
pub fn browser_reload(app: AppHandle, label: String) -> Result<(), String> {
    let content = content_webview(&app, &label)?;
    content.reload().map_err(|e| format!("刷新失败：{e}"))
}

/// 后退：栈底时返回错误（前端据此置灰按钮）。
#[tauri::command]
pub fn browser_go_back(app: AppHandle, label: String) -> Result<(), String> {
    let Some(target) = with_session(&label, |s| s.step_back()) else {
        return Err("没有可后退的历史".to_string());
    };
    navigate_in_history(&app, &label, target)
}

/// 前进：栈顶时返回错误。
#[tauri::command]
pub fn browser_go_forward(app: AppHandle, label: String) -> Result<(), String> {
    let Some(target) = with_session(&label, |s| s.step_forward()) else {
        return Err("没有可前进的历史".to_string());
    };
    navigate_in_history(&app, &label, target)
}

/// 前进/后退的公共尾部：解析 → 跳转。
///
/// 失败时把光标**回滚**：`step_back/step_forward` 已经移动了光标，若此时
/// navigate 报错（窗口句柄失效 / 目标不可解析）而不回滚，账本就会停在一个
/// 并没有真正到达的页上 —— 下一次前进/后退将从错误位置起跳。
///
/// 按钮可用性不在这里单独 emit：`on_page_load(Finished)` 会带上最新的
/// `canGoBack` / `canGoForward` 发 `navigated`，避免同一状态发两次、彼此打架。
fn navigate_in_history(app: &AppHandle, label: &str, target: String) -> Result<(), String> {
    let Ok(target) = parse_remote_url(&target) else {
        with_session(label, |s| s.rollback());
        return Err("目标 URL 不可用".to_string());
    };
    // 先解析再取窗口：URL 不可用时立即回滚光标，不必先取一次窗口句柄；
    // 窗口已关（content webview 不在注册表里）时同样回滚。两条失败路径
    // 都把光标送回原位，不留在并未真正到达的页上。
    let Ok(content) = content_webview(app, label) else {
        with_session(label, |s| s.rollback());
        return Err(format!("浏览窗口不存在：{label}"));
    };
    content.navigate(target).map_err(|e| {
        with_session(label, |s| s.rollback());
        format!("跳转失败：{e}")
    })
}

/// 列出所有活跃浏览窗口（壳页面恢复 tab 用）。
///
/// 遍历 `app.windows()`：新架构的窗口是 WindowBuilder 建的纯 Window，
/// `app.webview_windows()` 按 `is_webview_window`（全部子 webview 与 window
/// 同 label）过滤，**列不出**我们的窗口（已核对 tauri 2.11.5 源码）。
/// 排序输出：HashMap 迭代顺序随机，不排序前端 tab 列表会每次闪烁。
#[tauri::command]
pub fn browser_list_windows(app: AppHandle) -> Vec<BrowserWindowInfo> {
    let mut list: Vec<BrowserWindowInfo> = app
        .windows()
        .into_values()
        .filter(|window| is_browser_window_label(window.label()))
        .map(|window| {
            let label = window.label().to_string();
            // URL 以 content webview 的实时位置为准；首帧未加载完时回落到
            // session 历史栈（至少给出用户请求过的目标）。
            let url = app
                .get_webview(&content_label(&label))
                .and_then(|content| content.url().ok())
                .map(|u| u.to_string())
                .or_else(|| with_session(&label, |s| s.history.get(s.cursor).cloned()))
                .unwrap_or_default();
            let title = with_session(&label, |s| {
                if s.title.is_empty() {
                    INITIAL_TITLE.to_string()
                } else {
                    s.title.clone()
                }
            });
            BrowserWindowInfo { label, title, url }
        })
        .collect();
    list.sort_by(|a, b| a.label.cmp(&b.label));
    list
}

/// 录制事件回传（占位）：content 脚本 invoke 此命令 → 压入 session 的 recording 流，
/// 壳页面用 `browser_get_recording` 取走。真录制脚本（browser-recorder.src.js）
/// 尚未接入，命令契约先就位。
#[tauri::command]
pub fn browser_record_action(
    app: AppHandle,
    label: String,
    action: serde_json::Value,
) -> Result<(), String> {
    // 先验窗再记账：content 脚本只在窗口存活期间能 invoke，窗口已关时拒绝，
    // 而不是用 with_session 就地建一份永不被 forget 回收的僵尸 session。
    content_webview(&app, &label)?;
    with_session(&label, |s| s.recording.push(action));
    Ok(())
}

/// 取回某窗口已录制的动作流（无记录返回空 Vec，不就地建 session）。
#[tauri::command]
pub fn browser_get_recording(label: String) -> Vec<serde_json::Value> {
    with_existing_session(&label, |s| s.recording.clone()).unwrap_or_default()
}

/// 清空某窗口的录制流（幂等：窗口/记录不存在时静默成功）。
#[tauri::command]
pub fn browser_clear_recording(label: String) -> Result<(), String> {
    let _ = with_existing_session(&label, |s| s.recording.clear());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(parse_remote_url("https://example.com/a?b=1#c").is_ok());
        assert!(parse_remote_url("http://example.com").is_ok());
    }

    #[test]
    fn rejects_dangerous_and_malformed() {
        // 远程页不得借 file:/javascript: 读本地或执行代码
        assert!(parse_remote_url("file:///C:/Windows").is_err());
        assert!(parse_remote_url("javascript:alert(1)").is_err());
        assert!(parse_remote_url("data:text/html,<h1>x</h1>").is_err());
        assert!(parse_remote_url("").is_err());
        assert!(parse_remote_url("https://").is_err());
        assert!(parse_remote_url("not a url").is_err());
    }

    #[test]
    fn rejects_control_chars_and_overlong() {
        assert!(parse_remote_url("https://example.com/\nnext").is_err());
        assert!(parse_remote_url("https://example.com/\u{0}").is_err());
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(parse_remote_url(&long).is_err());
    }

    #[test]
    fn empty_session_cannot_move() {
        let mut s = Session::default();
        assert!(!s.can_go_back() && !s.can_go_forward());
        assert!(s.step_back().is_none());
        assert!(s.step_forward().is_none());
    }

    #[test]
    fn visits_build_forward_only_history() {
        let mut s = Session::default();
        s.visit("https://a.test");
        assert!(!s.can_go_back() && !s.can_go_forward());
        s.visit("https://b.test");
        s.visit("https://c.test");
        assert!(s.can_go_back() && !s.can_go_forward());
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn back_then_forward_round_trips() {
        let mut s = Session::default();
        s.visit("https://a.test");
        s.visit("https://b.test");
        s.visit("https://c.test");

        assert_eq!(s.step_back().as_deref(), Some("https://b.test"));
        assert!(s.can_go_back() && s.can_go_forward());
        assert_eq!(s.step_back().as_deref(), Some("https://a.test"));
        assert!(!s.can_go_back() && s.can_go_forward());

        assert_eq!(s.step_forward().as_deref(), Some("https://b.test"));
        assert_eq!(s.step_forward().as_deref(), Some("https://c.test"));
        assert!(s.can_go_back() && !s.can_go_forward());
    }

    #[test]
    fn back_does_not_repush_entry_onto_stack() {
        // 回归：on_page_load(Started) 对「前进/后退自己发起的跳转」必须抑制记账，
        // 否则后退一次就把同一页又压回栈里，栈越点越乱。
        let mut s = Session::default();
        s.visit("https://a.test");
        s.visit("https://b.test");

        let target = s.step_back().unwrap();
        // 模拟 on_page_load(Started)：URL 与 pending 一致 → 不记账
        assert!(s.take_pending(&target), "pending 应被消费并抑制记账");
        let len = s.history.len();
        assert_eq!(len, 2, "后退不得新增历史条目");
        assert_eq!(s.history, vec!["https://a.test", "https://b.test"]);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn manual_navigation_is_recorded_not_suppressed() {
        // 回归：地址栏手动跳转 / 点链接是**真实新访问**，必须进历史栈。
        // （曾经错误地把它也标成 pending，导致手动导航永远不进历史。）
        let mut s = Session::default();
        s.visit("https://a.test");
        let url = "https://b.test".to_string();
        let suppress = s.take_pending(&url);
        if !suppress {
            s.visit(&url);
        }
        assert!(!suppress);
        assert_eq!(s.history, vec!["https://a.test", "https://b.test"]);
        assert!(s.can_go_back());
    }

    #[test]
    fn stale_pending_self_heals() {
        // pending 那一跳最终没发生 → 下一个真实导航 URL 对不上，按新导航处理，
        // 标记作废不残留脏状态
        let mut s = Session::default();
        s.visit("https://a.test");
        s.step_back();
        assert!(!s.take_pending("https://never-happened.test"));
        s.visit("https://c.test");
        assert_eq!(s.history, vec!["https://a.test", "https://c.test"]);
        assert_eq!(s.cursor, 1);
        // 标记已消费，下一次不会误抑制
        assert!(!s.take_pending("https://c.test"));
    }

    #[test]
    fn rollback_restores_cursor_in_both_directions() {
        let mut s = Session::default();
        s.visit("https://a.test");
        s.visit("https://b.test");
        s.visit("https://c.test");
        // 起点在 c
        s.step_back();
        assert_eq!(s.cursor, 1);
        s.rollback(); // 后退失败 → 必须回到 c（cursor 2），不能冲过头
        assert_eq!(s.cursor, 2, "后退失败必须退回原页");

        // 从 c 前进是不可行的一侧，先退到 a 再前进，验证前进失败回滚
        s.step_back();
        s.step_back();
        assert_eq!(s.cursor, 0);
        s.step_forward();
        assert_eq!(s.cursor, 1);
        s.rollback(); // 前进失败 → 必须回到 a（cursor 0）
        assert_eq!(s.cursor, 0, "前进失败必须退回原页");
    }

    #[test]
    fn rollback_without_pending_is_noop() {
        let mut s = Session::default();
        s.visit("https://a.test");
        s.rollback();
        assert_eq!(s.cursor, 0);
        assert_eq!(s.history, vec!["https://a.test"]);
    }

    #[test]
    fn new_branch_truncates_forward_history() {
        let mut s = Session::default();
        s.visit("https://a.test");
        s.visit("https://b.test");
        s.step_back();
        assert!(s.can_go_forward());
        s.visit("https://c.test");
        assert!(!s.can_go_forward(), "新导航后前进分支必须作废");
        assert_eq!(s.history, vec!["https://a.test", "https://c.test"]);
    }

    #[test]
    fn redirect_to_same_url_does_not_duplicate() {
        let mut s = Session::default();
        s.visit("https://a.test");
        s.visit("https://a.test");
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn history_is_bounded_keeping_recent_entries() {
        let mut s = Session::default();
        for i in 0..(MAX_HISTORY + 10) {
            s.visit(&format!("https://e{i}.test"));
        }
        assert_eq!(s.history.len(), MAX_HISTORY);
        assert_eq!(
            s.history.last().map(String::as_str),
            Some(format!("https://e{}.test", MAX_HISTORY + 9).as_str())
        );
        // 截断头部后 cursor 必须同步，否则 can_go_back 会算错
        assert_eq!(s.cursor, MAX_HISTORY - 1);
        assert!(s.can_go_back() && !s.can_go_forward());
    }

    // ── 新架构：label 派生 / 窗口过滤 / 录制流 ──────────────────────────────

    #[test]
    fn child_labels_are_derived_from_window_label() {
        assert_eq!(frame_label("browser-1"), "browser-1-frame");
        assert_eq!(content_label("browser-1"), "browser-1-content");
        // 序号不受位数影响，两个子 label 必须可区分
        assert_ne!(frame_label("browser-12"), content_label("browser-12"));
    }

    #[test]
    fn browser_window_label_filter() {
        // 真窗口 label：前缀 + 非空序号
        assert!(is_browser_window_label("browser-1"));
        assert!(is_browser_window_label("browser-42"));
        // 非浏览窗口（主窗口 / splash / HUD 一个都不能漏出来）
        assert!(!is_browser_window_label("main"));
        assert!(!is_browser_window_label("splash"));
        assert!(!is_browser_window_label("hud"));
        // 子 webview label 不是窗口：-frame / -content 必须被排除
        assert!(!is_browser_window_label("browser-1-frame"));
        assert!(!is_browser_window_label("browser-1-content"));
        // 前缀粘住但序号为空
        assert!(!is_browser_window_label("browser-"));
        // 前缀都不算（无分隔符）
        assert!(!is_browser_window_label("browserframe"));
    }

    #[test]
    fn recording_stream_round_trips_and_clears() {
        let mut s = Session::default();
        let action = serde_json::json!({ "action": "click", "selector": "#go" });
        s.recording.push(action.clone());
        s.recording.push(serde_json::json!({ "action": "input" }));
        assert_eq!(s.recording.len(), 2);
        assert_eq!(s.recording[0], action);
        s.recording.clear();
        assert!(s.recording.is_empty());
    }

    #[test]
    fn title_and_recording_default_to_empty() {
        // 回归：Session 新增 title / recording 字段后，默认构造仍可用
        // （on_page_load 前的窗口没有任何标题/录制记录）
        let s = Session::default();
        assert!(s.title.is_empty());
        assert!(s.recording.is_empty());
        assert!(!s.loading);
    }
}
