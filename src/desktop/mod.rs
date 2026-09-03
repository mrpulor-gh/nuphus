//! Desktop module - Rust native desktop control
//!
//! Desktop automation based on desktop-api crate (Win32 + xcap).
//! All features implemented natively in Rust.

pub mod client;
pub mod dict_ocr;
pub mod linux_window;
pub mod paddle_ocr;
pub mod rec_hook;
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
