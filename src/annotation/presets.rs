//! AnnotationPresets — 发行版内置预置关系标注。
//! 内置标注 `builtin: true`，用户不可删除但可覆盖。
//! 添加 annotation 的约束见 annotation_add 工具描述。

use super::types::Annotation;

/// 获取所有内置预置标注
pub fn get_builtins() -> Vec<Annotation> {
    // ── 外部 Agent 平台交互 ──
    // 用户说出平台名 → 注入关键规则 + 指向 skill
    let rule = "外部 Agent ≠ task_dispatch。用 desktop_input（终端类）或 browser（Web 类）交互。\
               外部 Agent 无 Checker，Leader 自行验证产出。\
               交互协议、窗口识别、并行调度、失败回退详见 skill: agent-orchestration。";

    let platforms = [
        "Claude",      // Anthropic
        "Codex",       // OpenAI Codex CLI
        "Copilot",     // GitHub Copilot
        "Cursor",      // 独立桌面 IDE
        "Gemini",      // Google Gemini
        "Hermes",      // Nous Research 桌面 Agent
        "OpenClaw",    // 开源自主 AI 助理
        "OpenCode",    // 开源终端/桌面/IDE Agent
        "vibe coding", // AI vibe coding 范式
        "Windsurf",    // 独立桌面 IDE
        "WorkBuddy",   // 腾讯 AI 工作台
        "ZCode",       // 智谱 ADE
    ];

    let mut ann = Annotation::new(
        "外部 Agent 平台".into(),
        rule.into(),
        vec![],
        vec!["external-agent".into(), "protocol".into()],
        "system".into(),
        true,
        100,
    );
    ann.keywords = platforms.iter().map(|s| s.to_string()).collect();
    vec![ann]
}
