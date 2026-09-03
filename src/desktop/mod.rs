//! Desktop module - Rust native desktop control
//!
//! Desktop automation based on desktop-api crate (Win32 + xcap).
//! All features implemented natively in Rust.

pub mod client;
pub mod dict_ocr;
pub mod linux_window;
pub mod paddle_ocr;
// 桌面操作录制（低层 hook 捕获）仅 Windows 平台实现（WH_MOUSE_LL/KEYBOARD_LL）。
// 其余平台提供同名 stub：命令层 rec.rs 引用 rec_hook:: 符号时无需逐处 cfg，
// 运行期 capture_once 返回明确错误（2026-09-03 CI clippy -D warnings 修复）。
#[cfg(windows)]
pub mod rec_hook;

#[cfg(not(windows))]
pub mod rec_hook {
    // ── 非 Windows stub ──
    #[derive(Debug, Clone, serde::Serialize)]
    pub struct CaptureEvent {
        pub kind: String,
        pub button: Option<String>,
        pub x: i32,
        pub y: i32,
        pub wheel_delta: Option<i32>,
        pub keys: Vec<String>,
        pub window_title: String,
        pub hwnd: isize,
        pub pid: u32,
        pub process_name: Option<String>,
        pub ts_ms: u64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CaptureKind {
        Click,
        Scroll,
        Hotkey,
        Any,
    }

    impl CaptureKind {
        // 固有字符串解析（非 FromStr trait：调用点用 CaptureKind::from_str 语义更清晰）
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(s: &str) -> Result<Self, String> {
            match s.to_ascii_lowercase().as_str() {
                "click" => Ok(Self::Click),
                "scroll" => Ok(Self::Scroll),
                "hotkey" => Ok(Self::Hotkey),
                "any" => Ok(Self::Any),
                _ => Err(format!("未知捕获类型: {s}")),
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

    pub fn rec_cancel_current() {}

    pub fn capture_once(
        _kind: CaptureKind,
        _timeout_secs: u64,
        _ignore_self_window: bool,
    ) -> Result<CaptureEvent, String> {
        Err("桌面操作录制当前仅支持 Windows 平台".to_string())
    }
}
pub mod ui_perception;
pub mod vision;
pub mod vision_ocr;

// YoloDetector 已收编至 desktop-api（vision::yolo，同一二进制唯一检测路径，
// 消除双实现漂移——审计 P1）。此 re-export 保持既有调用路径兼容。
pub use desktop_api::vision::yolo::YoloDetector;

pub use client::captures_dir_path;
pub use client::DesktopClient;

use std::path::PathBuf;

/// 解析模型目录 — paddle_ocr 的入口（yolo 已收编至 desktop-api，用其自身 vision::models::resolve_models_dir）
///
/// 优先级：
/// 1. 环境变量 NUPHUS_MODELS_DIR
/// 2. 用户数据目录 (data_dir/Nuphus/models/)
/// 3. exe 相对路径候选
/// 4. cwd 相对路径候选（开发/测试环境）
/// 5. CARGO_MANIFEST_DIR 开发路径
pub fn resolve_models_dir() -> Option<PathBuf> {
    // 1. 环境变量 NUPHUS_MODELS_DIR
    if let Ok(dir) = std::env::var("NUPHUS_MODELS_DIR") {
        let p = PathBuf::from(&dir);
        if p.exists() {
            tracing::debug!("[models] 从 NUPHUS_MODELS_DIR 加载: {}", p.display());
            return Some(p);
        }
    }

    // 2. 用户数据目录
    if let Some(data_dir) = dirs::data_dir() {
        let p = data_dir.join("Nuphus").join("models");
        if p.exists() {
            tracing::debug!("[models] 从 data_dir 加载: {}", p.display());
            return Some(p);
        }
    }

    // 3. exe 相对路径
    if let Ok(exe) = std::env::current_exe() {
        let exe_candidates: Vec<PathBuf> = (0..=2)
            .filter_map(|n| {
                let mut p = exe.parent()?;
                for _ in 0..n {
                    p = p.parent()?;
                }
                Some(p.join("desktop").join("models"))
            })
            .collect();
        for c in &exe_candidates {
            if c.exists() {
                tracing::debug!("[models] 从 exe 路径加载: {}", c.display());
                return Some(c.clone());
            }
        }
    }

    // 4. cwd 候选（开发/测试环境）
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_candidates = vec![
            cwd.join("src-tauri").join("desktop").join("models"),
            cwd.join("../src-tauri/desktop/models"),
        ];
        // 也尝试上一级
        if let Some(parent) = cwd.parent() {
            let parent_candidates = vec![
                parent.join("src-tauri").join("desktop").join("models"),
                parent.join("../src-tauri/desktop/models"),
            ];
            for c in parent_candidates {
                if c.exists() {
                    tracing::debug!("[models] 从 cwd 父路径加载: {}", c.display());
                    return Some(c);
                }
            }
        }
        for c in &cwd_candidates {
            if c.exists() {
                tracing::debug!("[models] 从 cwd 路径加载: {}", c.display());
                return Some(c.clone());
            }
        }
    }

    // 5. CARGO_MANIFEST_DIR 开发路径
    let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/desktop/models");
    if dev_path.exists() {
        tracing::debug!(
            "[models] 从 CARGO_MANIFEST_DIR 加载: {}",
            dev_path.display()
        );
        return Some(dev_path);
    }

    tracing::warn!("[models] 未找到模型目录");
    None
}
