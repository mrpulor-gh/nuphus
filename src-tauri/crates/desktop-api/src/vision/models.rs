//! 本地模型路径解析与就绪检查（PaddleOCR + YOLO 共享）。
//!
//! 与主 crate `src/desktop/mod.rs::resolve_models_dir` 保持同一优先级，保证
//! nuphus-mcp / 主程序 / desktop-api 三条调用链命中同一个模型目录：
//! 1. 环境变量 `NUPHUS_MODELS_DIR`
//! 2. 用户数据目录 `data_dir/Nuphus/models`
//! 3. exe 相对路径候选
//! 4. cwd 相对路径候选（开发/测试环境）
//!
//! `models_dir_for_write` 是下载器的落盘目标（可创建）；`resolve_models_dir`
//! 只返回已存在的目录。

use std::path::{Path, PathBuf};

/// PaddleOCR 文本检测模型（~4.6 MB）
pub const PADDLE_DET_MODEL: &str = "ch_PP-OCRv4_det.onnx";
/// PaddleOCR 文本识别模型（~9.2 MB）
pub const PADDLE_REC_MODEL: &str = "ch_PP-OCRv4_rec.onnx";
/// PaddleOCR 字符字典（~93 KB，6623 类）
pub const PADDLE_DICT: &str = "ch_PP-OCR_keys_v1.txt";
/// YOLO UI 元素检测模型（icon_detect.onnx，~80 MB，可选增强）
pub const YOLO_MODEL: &str = "icon_detect.onnx";

/// PaddleOCR 必需的三个文件
pub const PADDLE_OCR_FILES: [&str; 3] = [PADDLE_DET_MODEL, PADDLE_REC_MODEL, PADDLE_DICT];

/// 解析已存在的模型目录（只读，不创建）。
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

    // 3. exe 相对路径候选（发布物旁置）
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

    tracing::warn!("[models] 未找到模型目录");
    None
}

/// 下载器的落盘目标：优先 `NUPHUS_MODELS_DIR`，否则用户数据目录，必要时创建。
pub fn models_dir_for_write() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("NUPHUS_MODELS_DIR") {
        let p = PathBuf::from(&dir);
        std::fs::create_dir_all(&p)
            .map_err(|e| format!("创建模型目录失败 {}: {e}", p.display()))?;
        return Ok(p);
    }
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "无法定位用户数据目录 (dirs::data_dir 返回 None)".to_string())?;
    let p = data_dir.join("Nuphus").join("models");
    std::fs::create_dir_all(&p).map_err(|e| format!("创建模型目录失败 {}: {e}", p.display()))?;
    Ok(p)
}

/// 校验 PaddleOCR 三件套是否齐备。返回 Err 时附带缺失文件路径（供下载指引）。
pub fn validate_ocr_models(dir: &Path) -> Result<(), String> {
    let missing: Vec<String> = PADDLE_OCR_FILES
        .iter()
        .filter(|f| !dir.join(f).exists())
        .map(|f| dir.join(f).display().to_string())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "PaddleOCR 模型文件缺失 ({}): {}",
            missing.len(),
            missing.join(", ")
        ))
    }
}

/// YOLO UI 元素检测模型是否存在（可选增强，缺失时仅 OCR 可用）。
pub fn yolo_model_available(dir: &Path) -> bool {
    dir.join(YOLO_MODEL).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ocr_models_reports_missing_files() {
        // 空临时目录 → 必须报出缺失文件且不 panic
        let tmp = std::env::temp_dir().join("nuphus_desktop_api_missing_models_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let err = validate_ocr_models(&tmp).expect_err("empty dir must fail");
        assert!(err.contains(PADDLE_DET_MODEL), "err should name det: {err}");
        assert!(err.contains(PADDLE_REC_MODEL), "err should name rec: {err}");
        assert!(err.contains(PADDLE_DICT), "err should name dict: {err}");
    }

    #[test]
    fn yolo_model_missing_is_not_fatal() {
        let tmp = std::env::temp_dir().join("nuphus_desktop_api_yolo_missing_test");
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(!yolo_model_available(&tmp));
        // OCR 校验与 YOLO 独立：OCR 缺失时报错，YOLO 缺失只是 optional
        assert!(validate_ocr_models(&tmp).is_err());
    }

    #[test]
    fn file_constants_non_empty() {
        assert!(PADDLE_OCR_FILES.iter().all(|f| !f.is_empty()));
        assert_eq!(PADDLE_OCR_FILES.len(), 3);
    }
}
