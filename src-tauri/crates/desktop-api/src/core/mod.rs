//! 核心类型系统 - Session, Target, Frame, Scope, AppKind

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[cfg(feature = "http-server")]
use std::sync::Arc;
#[cfg(feature = "http-server")]
use tokio::sync::RwLock;
use uuid::Uuid;

// ─────────────────────────────── 应用类型 ───────────────────────────────

#[cfg(feature = "http-server")]
/// 应用类型 - 决定交互策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppKind {
    /// Win32/UWP 桌面应用 - 需要窗口激活、SendInput
    Desktop,
    /// 浏览器页面 - Playwright/CDP 操作
    Browser,
    /// 终端/TUI 应用 - 特殊字符编码
    Tui,
}

// ─────────────────────────────── 目标规格 ───────────────────────────────

#[cfg(feature = "http-server")]
/// 创建会话时指定的目标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSpec {
    /// 通过窗口标题查找
    Title(String),
    /// 通过窗口句柄 (Windows)
    Hwnd(isize),
    /// 通过进程名
    Process(String),
    /// 浏览器 URL
    Url(String),
    /// Playwright 已连接的页面
    PlaywrightPage {
        ws_url: String,
        token: Option<String>,
    },
}

// ─────────────────────────────── 会话 ───────────────────────────────

#[cfg(feature = "http-server")]
/// 会话句柄 - 所有操作的入口
#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub id: Uuid,
    pub kind: AppKind,
    pub target: Arc<RwLock<Target>>,
    pub viewport: Arc<RwLock<Viewport>>,
    pub created_at: DateTime<Utc>,
    pub last_activity: Arc<RwLock<DateTime<Utc>>>,
}

#[cfg(feature = "http-server")]
impl SessionHandle {
    /// 更新活动时间
    pub async fn touch(&self) {
        let mut last = self.last_activity.write().await;
        *last = Utc::now();
    }
}

// ─────────────────────────────── 图形后端 ───────────────────────────────

/// 窗口的图形渲染后端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GfxBackend {
    Unknown,
    Gdi,
    DirectX,
    OpenGl,
    Vulkan,
}

// ─────────────────────────────── 目标 ───────────────────────────────

/// 运行时目标 - 包含平台句柄
#[derive(Debug)]
pub enum Target {
    /// Windows 窗口
    #[cfg(windows)]
    Window {
        hwnd: isize,
        title: String,
        verified: bool,
        gfx_backend: GfxBackend,
    },
    /// 浏览器页面
    Browser { page_id: String, url: String },
    /// 终端
    Tui { hwnd: isize, title: String },
}

impl Target {
    /// 获取窗口标题 (用于识别)
    pub fn title(&self) -> &str {
        match self {
            #[cfg(windows)]
            Target::Window { title, .. } => title,
            Target::Browser { url, .. } => url,
            Target::Tui { title, .. } => title,
        }
    }

    /// 是否已验证激活
    pub fn is_verified(&self) -> bool {
        match self {
            #[cfg(windows)]
            Target::Window { verified, .. } => *verified,
            Target::Browser { .. } => true, // 浏览器不需要前台
            Target::Tui { .. } => false,
        }
    }

    /// 标记为已验证
    pub fn verify(&mut self) {
        match self {
            #[cfg(windows)]
            Target::Window { verified, .. } => *verified = true,
            Target::Browser { .. } => {}
            Target::Tui { .. } => {}
        }
    }
}

// ─────────────────────────────── 视口 ───────────────────────────────

#[cfg(feature = "http-server")]
/// 当前可视区域
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

#[cfg(feature = "http-server")]
impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
        }
    }
}

// ─────────────────────────────── 帧 ───────────────────────────────

/// 屏幕帧 - 内存中，不落盘
#[derive(Debug)]
pub struct Frame {
    pub id: Uuid,
    pub pixels: Vec<u8>, // RGBA
    pub width: u32,
    pub height: u32,
    pub scope: Scope,
    pub timestamp: DateTime<Utc>,
    pub source: FrameSource,
}

/// 帧来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameSource {
    Screenshot,
    WindowCapture,
    BrowserCapture,
    RegionCrop,
}

impl Frame {
    /// 计算近似内存占用 (KB)
    pub fn memory_kb(&self) -> f64 {
        (self.pixels.len() as f64) / 1024.0
    }

    /// 估算 PNG 大小 (KB)
    pub fn estimated_png_kb(&self) -> f64 {
        // 粗略估算: 压缩率约 30-50%
        self.memory_kb() * 0.4
    }

    /// 裁剪区域
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Option<Self> {
        if x + w > self.width || y + h > self.height {
            return None;
        }

        let mut cropped = Vec::with_capacity((w * h * 4) as usize);
        for row in y..(y + h) {
            let start = ((row * self.width + x) * 4) as usize;
            let end = start + (w * 4) as usize;
            cropped.extend_from_slice(&self.pixels[start..end]);
        }

        Some(Self {
            id: Uuid::new_v4(),
            pixels: cropped,
            width: w,
            height: h,
            scope: Scope::Element {
                x: x as i32,
                y: y as i32,
                w,
                h,
            },
            timestamp: Utc::now(),
            source: FrameSource::RegionCrop,
        })
    }

    /// 获取像素颜色
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = ((y * self.width + x) * 4) as usize;
        Some(Color {
            r: self.pixels[idx],
            g: self.pixels[idx + 1],
            b: self.pixels[idx + 2],
            a: self.pixels[idx + 3],
        })
    }
}

// ─────────────────────────────── 截图范围 ───────────────────────────────

/// 截图范围 - 决定感知粒度
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Scope {
    /// 全盘 - 极少用，需显式指定
    Fullscreen,
    /// 当前窗口 - 默认
    #[default]
    Window,
    /// 窗口客户区 (去掉标题栏边框)
    ClientArea,
    /// 元素级 - 推荐
    Element { x: i32, y: i32, w: u32, h: u32 },
    /// 点周围区域 - 最省
    Point { x: i32, y: i32, radius: u32 },
}

// ─────────────────────────────── 颜色 ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// 色差 (0-441.7)
    pub fn distance(&self, other: &Color) -> f32 {
        let dr = (self.r as f32 - other.r as f32).abs();
        let dg = (self.g as f32 - other.g as f32).abs();
        let db = (self.b as f32 - other.b as f32).abs();
        (dr * dr + dg * dg + db * db).sqrt()
    }

    /// 是否在容差内
    pub fn matches(&self, target: &Color, tolerance: u8) -> bool {
        self.distance(target) <= (tolerance as f32 * 3.0_f32.sqrt())
    }
}

// ─────────────────────────────── 几何 ───────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn center(&self) -> Point {
        Point {
            x: self.x + (self.w as i32) / 2,
            y: self.y + (self.h as i32) / 2,
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x
            && p.x < self.x + self.w as i32
            && p.y >= self.y
            && p.y < self.y + self.h as i32
    }
}

// ─────────────────────────────── 会话管理 ───────────────────────────────

#[cfg(feature = "http-server")]
use std::collections::HashMap;

#[cfg(feature = "http-server")]
pub struct SessionManager {
    sessions: HashMap<Uuid, SessionHandle>,
}

#[cfg(feature = "http-server")]
impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub async fn create(&mut self, spec: TargetSpec) -> Result<SessionHandle> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        // TODO: 根据 spec 解析目标类型
        let kind = AppKind::Desktop;
        let target = Arc::new(RwLock::new(Target::Tui {
            hwnd: 0,
            title: "placeholder".to_string(),
        }));

        let handle = SessionHandle {
            id,
            kind,
            target,
            viewport: Arc::new(RwLock::new(Viewport::default())),
            created_at: now,
            last_activity: Arc::new(RwLock::new(now)),
        };

        self.sessions.insert(id, handle.clone());
        Ok(handle)
    }

    pub fn get(&self, id: Uuid) -> Option<SessionHandle> {
        self.sessions.get(&id).cloned()
    }

    pub async fn close(&mut self, id: Uuid) -> Result<()> {
        self.sessions.remove(&id);
        Ok(())
    }
}

// ─────────────────────────────── 错误 ───────────────────────────────

pub type Result<T> = std::result::Result<T, DesktopError>;

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("目标未找到: {0}")]
    TargetNotFound(String),
    #[error("窗口激活失败: {0}")]
    ActivationFailed(String),
    #[error("截图失败: {0}")]
    CaptureFailed(String),
    #[error("OCR 失败: {0}")]
    OcrFailed(String),
    #[error("输入发送失败: {0}")]
    InputFailed(String),
    #[error("查找失败: {0}")]
    LocateFailed(String),
    #[error("所有策略已耗尽")]
    AllStrategiesFailed,
    #[error("会话不存在: {0}")]
    SessionNotFound(Uuid),
    #[error("平台不支持")]
    PlatformNotSupported,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
