//! 图片处理策略（两态能力矩阵）与描述缓存
//!
//! 主模型不支持视觉时的统一策略（2026-08-08 起，按大王决策）：
//! - `Main` — 主模型原生支持视觉，image_url 直发主模型（零变化）
//! - `Fallback` — 主模型不支持：图片保存为临时 BMP + 路径注入 LLM 上下文，
//!   Agent 按需调用 desktop_vision(image_path=<路径>, prompt=<精准问题>) 定向查看。
//!   不再自动调用视觉模型生成泛化描述（VisionDescribe 已废弃）。
//! - `None` — 主模型不支持 + 未配置视觉模型 → 图片降级发送 + 前端 image_warning 弹窗
//!
//! 描述缓存 key 使用图片 data URL 的稳定 hash（data URL 在入 session 时已冻结为
//! PNG，不再变化）。

/// 图片处理策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageStrategy {
    /// 主模型原生支持视觉 → image_url 直发主模型
    Main,
    /// 主模型不支持 → 图片保存为临时 BMP + 路径注入，Agent 按需调 desktop_vision。
    /// （2026-08-08 起替代 VisionDescribe 自动描述模式）
    Fallback,
    /// 主模型不支持 + 未配置视觉模型 → 降级发送 + image_warning
    None,
}

/// 基于主模型原生支持度与已配视觉模型判定图片处理策略
///
/// `supports_vision` 来自 ModelEntry/ModelDef 的主模型原生支持；
/// `vision_model` 来自 resolve_vision_strategy()（Capability=独立视觉模型 /
/// Main=主模型自身 / None=未配置）。
pub fn resolve_image_strategy(supports_vision: bool, vision_model: Option<&str>) -> ImageStrategy {
    if supports_vision {
        ImageStrategy::Main
    } else if vision_model.is_some() {
        ImageStrategy::Fallback
    } else {
        ImageStrategy::None
    }
}

/// 图片 data URL 稳定 hash（描述缓存 key）
///
/// 与 save_base64_to_temp_bmp 的 hash 风格一致（DefaultHasher），
/// 对 URL 字符串做 hash——URL 已冻结，相同图片始终产生相同 key。
pub fn image_url_hash(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_image_strategy_main() {
        // 主模型原生支持 → Main（无论 vision_model 是什么）
        assert_eq!(resolve_image_strategy(true, None), ImageStrategy::Main);
        assert_eq!(
            resolve_image_strategy(true, Some("minimax-vl")),
            ImageStrategy::Main
        );
    }

    #[test]
    fn test_resolve_image_strategy_fallback() {
        // 主模型不支持 + 独立视觉模型 → Fallback（路径注入 + Agent 按需 desktop_vision）
        assert_eq!(
            resolve_image_strategy(false, Some("minimax-vl")),
            ImageStrategy::Fallback
        );
    }

    #[test]
    fn test_resolve_image_strategy_none() {
        // 都无 → None
        assert_eq!(resolve_image_strategy(false, None), ImageStrategy::None);
    }

    #[test]
    fn test_image_url_hash_stable() {
        let url = "data:image/png;base64,AAAA";
        assert_eq!(image_url_hash(url), image_url_hash(url));
        assert_ne!(
            image_url_hash(url),
            image_url_hash("data:image/png;base64,BBBB")
        );
    }
}
