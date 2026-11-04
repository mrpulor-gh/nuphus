//! references — 聊天引用（ChatReference）解析与注入文本组装
//!
//! 主会话（桌面端 `commands::process::resolve_references`）与 Side Chat
//! （`commands::side_chat`）共用同一份解析：按 `ref_type` 取注入内容
//! （读 plugin 资源 / 取 content 或 label），拼成注入用户消息的文本。
//! capture（截图路径提示）是主会话特例，不在此通用分支内——Side Chat
//! 无视觉工具，capture 落入未知类型提示。

use serde::Deserialize;
use std::path::Path;

/// 前端传递的资源引用（主会话 ChatMessage.references / Side Chat references）
///
/// `type` 是 Rust 关键字，用 `#[serde(rename)]` 反序列化。
#[derive(Debug, Clone, Deserialize)]
pub struct ChatReference {
    #[serde(rename = "type")]
    pub ref_type: String, // "skill" | "knowledge" | "workflow" | "capture" | "quote" | "sidechat"
    pub id: String,
    pub label: String,
    /// 注入内容（每个分支优先取它）：显示与注入分离——quote 无 content 时
    /// 回落 label（现有行为不变），sidechat 等新类型直接承载注入全文。
    /// `#[serde(default)]` 兼容旧前端（缺省 None）。
    #[serde(default)]
    pub content: Option<String>,
}

/// 解析整组引用 → 注入文本（`\n\n` 连接；空组 / 全空内容 → 空串）。
///
/// 生产入口：plugin 根取 [`crate::utils::workspace_root`]。
pub async fn resolve_references(refs: &[ChatReference]) -> String {
    let root = crate::utils::workspace_root();
    let mut parts: Vec<String> = Vec::new();
    for r in refs {
        if let Some(text) = resolve_reference_text(&root, r).await {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

/// 单条引用 → 注入文本（已含通用格式）；`None` = 内容为空，调用方跳过。
///
/// `root` = plugin 的父目录（[`crate::utils::workspace_root`]），由调用方
/// 传入以便测试隔离，不自己解析全局根。
///
/// 取数顺序：`content` 字段优先（带 content 的引用直接注入它——quote 回落
/// label、sidechat 等新类型无需特例分支）；否则按类型读 plugin 资源；
/// 未登记类型且无 content → `[Unknown reference type: x]`。
pub async fn resolve_reference_text(root: &Path, r: &ChatReference) -> Option<String> {
    let content = match r.content.as_deref() {
        Some(c) => c.to_string(),
        None => match r.ref_type.as_str() {
            "skill" => {
                let entry = root
                    .join("plugin")
                    .join("skills")
                    .join(&r.id)
                    .join("SKILL.md");
                read_or_missing(&entry, &format!("[Skill not found: {}]", r.id)).await
            }
            "knowledge" => {
                let entry = root
                    .join("plugin")
                    .join("knowledge")
                    .join(format!("{}.md", r.id));
                read_or_missing(&entry, &format!("[Knowledge not found: {}]", r.id)).await
            }
            "workflow" => {
                let entry = root
                    .join("plugin")
                    .join("workflows")
                    .join(&r.id)
                    .join("workflow.json");
                read_or_missing(&entry, &format!("[Workflow not found: {}]", r.id)).await
            }
            // 聊天区选中文字追问：原文由 label 承载（前端 chip 只做显示截断）。
            // 不落盘、不查文件——引用的就是这段文本本身，注入后即可针对它追问。
            "quote" => r.label.clone(),
            _ => format!("[Unknown reference type: {}]", r.ref_type),
        },
    };
    if content.is_empty() {
        return None;
    }
    // quote 的 label 即原文全文（该类型没有独立显示名）：若套用下面
    // 「[类型] label:」的通用格式，会把同一段文本注入两次
    // （[quote] 原文:\n原文）。该类型只发类型标记 + 原文。
    if r.ref_type == "quote" {
        Some(format!("[quote]\n{}", content))
    } else {
        Some(format!("[{}] {}:\n{}", r.ref_type, r.label, content))
    }
}

/// 读插件资源文件；不存在时返回 missing 提示，读失败按空串（调用方跳过）。
async fn read_or_missing(entry: &Path, missing: &str) -> String {
    if entry.exists() {
        tokio::fs::read_to_string(entry).await.unwrap_or_default()
    } else {
        missing.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_of(ref_type: &str, id: &str, label: &str) -> ChatReference {
        ChatReference {
            ref_type: ref_type.into(),
            id: id.into(),
            label: label.into(),
            content: None,
        }
    }

    /// 缺 content 字段的旧前端 payload 必须能反序列化（content = None）。
    #[test]
    fn deserializes_without_content_field_as_none() {
        let json = r#"{"type":"quote","id":"q1","label":"原文"}"#;
        let r: ChatReference = serde_json::from_str(json).expect("旧前端 payload 必须兼容");
        assert!(r.content.is_none());
        assert_eq!(r.label, "原文");
    }

    /// content 优先于 label：带 content 的 quote 注入 content 而非 label。
    #[tokio::test]
    async fn content_wins_over_label_for_quote() {
        let root = std::env::temp_dir();
        let mut r = ref_of("quote", "q1", "显示用短名");
        r.content = Some("注入用全文".into());
        let out = resolve_reference_text(&root, &r)
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[quote]\n注入用全文");
    }

    /// 无 content 的 quote 回落 label，且只注入一次（不套通用格式）。
    #[tokio::test]
    async fn quote_without_content_falls_back_to_label() {
        let root = std::env::temp_dir();
        let out = resolve_reference_text(&root, &ref_of("quote", "q1", "选中原文"))
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[quote]\n选中原文");
    }

    /// sidechat 新类型无需特例：走通用格式 `[type] label:\ncontent`。
    #[tokio::test]
    async fn sidechat_type_uses_generic_format_without_special_case() {
        let root = std::env::temp_dir();
        let mut r = ref_of("sidechat", "s1", "来自临时对话");
        r.content = Some("助手回复全文".into());
        let out = resolve_reference_text(&root, &r)
            .await
            .expect("有内容不该跳过");
        assert_eq!(out, "[sidechat] 来自临时对话:\n助手回复全文");
    }

    /// 未登记类型且无 content → 未知提示（行为不变）。
    #[tokio::test]
    async fn unknown_type_without_content_reports_unknown() {
        let root = std::env::temp_dir();
        let out = resolve_reference_text(&root, &ref_of("mystery", "x", "L"))
            .await
            .expect("有提示文本不该跳过");
        assert_eq!(out, "[mystery] L:\n[Unknown reference type: mystery]");
    }

    /// 空内容跳过：空 label 的 quote 不产出注入文本。
    #[tokio::test]
    async fn empty_content_is_skipped() {
        let root = std::env::temp_dir();
        assert!(resolve_reference_text(&root, &ref_of("quote", "q1", ""))
            .await
            .is_none());
    }

    /// skill：读 `{root}/plugin/skills/{id}/SKILL.md` 并按通用格式拼接。
    #[tokio::test]
    async fn skill_reads_plugin_file() {
        let root = std::env::temp_dir().join(format!("nuphus-refs-test-{}", std::process::id()));
        let entry = root
            .join("plugin")
            .join("skills")
            .join("demo")
            .join("SKILL.md");
        std::fs::create_dir_all(entry.parent().expect("有父目录")).expect("建临时 skill 目录");
        std::fs::write(&entry, "# demo skill").expect("写临时 SKILL.md");
        let out = resolve_reference_text(&root, &ref_of("skill", "demo", "演示技能"))
            .await
            .expect("读到文件不该跳过");
        assert_eq!(out, "[skill] 演示技能:\n# demo skill");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// skill 不存在 → `[Skill not found: id]`（与旧行为逐字一致）。
    #[tokio::test]
    async fn missing_skill_reports_not_found() {
        let root = std::env::temp_dir().join(format!("nuphus-refs-miss-{}", std::process::id()));
        let out = resolve_reference_text(&root, &ref_of("skill", "nope", "技能"))
            .await
            .expect("有提示文本不该跳过");
        assert_eq!(out, "[skill] 技能:\n[Skill not found: nope]");
    }

    /// knowledge：读 `{root}/plugin/knowledge/{id}.md` 并按通用格式拼接。
    #[tokio::test]
    async fn knowledge_reads_plugin_file() {
        let root = std::env::temp_dir().join(format!("nuphus-refs-kd-{}", std::process::id()));
        let entry = root.join("plugin").join("knowledge").join("demo.md");
        std::fs::create_dir_all(entry.parent().expect("有父目录")).expect("建临时 knowledge 目录");
        std::fs::write(&entry, "知识内容").expect("写临时知识文件");
        let out = resolve_reference_text(&root, &ref_of("knowledge", "demo", "知识点"))
            .await
            .expect("读到文件不该跳过");
        assert_eq!(out, "[knowledge] 知识点:\n知识内容");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// workflow：读 `{root}/plugin/workflows/{id}/workflow.json`。
    #[tokio::test]
    async fn workflow_reads_plugin_file() {
        let root = std::env::temp_dir().join(format!("nuphus-refs-wf-{}", std::process::id()));
        let entry = root
            .join("plugin")
            .join("workflows")
            .join("demo")
            .join("workflow.json");
        std::fs::create_dir_all(entry.parent().expect("有父目录")).expect("建临时 workflow 目录");
        std::fs::write(&entry, "{}").expect("写临时 workflow.json");
        let out = resolve_reference_text(&root, &ref_of("workflow", "demo", "工作流"))
            .await
            .expect("读到文件不该跳过");
        assert_eq!(out, "[workflow] 工作流:\n{}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
