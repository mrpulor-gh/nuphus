//! Skills — 检索式知识包模块
//!
//! 每个 skill = skill.json + SKILL.md + data/ 的结构化知识包，
//! 通过 skill_query/skill_read 等工具按需检索，不注入 LLM prompt。

pub mod registry;
pub mod types;

pub use registry::plugin_skills_dir;
pub use registry::SkillRegistry;
pub use types::*;
