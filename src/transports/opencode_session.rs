//! OpenCode Go 实例级稳定会话 ID
//!
//! OpenCode Go 网关（opencode.ai/zen/go）要求每个对话携带稳定的
//! `x-opencode-session` 请求头（官方文档 "Send a stable session ID in
//! x-opencode-session"），缺失时网关返回 400 MissingSessionID。Nuphus 没有
//! per-conversation id 的下传链路（`MessageRequest` 无会话字段），因此采用
//! **持久化的实例级稳定 ID**：写入 `{nuphus_data_dir()}/opencode-session.txt`，
//! 进程内 `OnceLock` 缓存。同一 Nuphus 实例的所有对话共享该 ID，供网关
//! 路由与 prompt 缓存优化使用。
//!
//! # 安全约束
//! 该 ID 仅供请求头注入。**绝不**写入日志 / 错误信息 / 请求 body，
//! 也不参与任何 serde 序列化输出。

use std::path::Path;
use std::sync::OnceLock;

/// 会话 ID 文件名（位于 Nuphus 数据目录下）。
const SESSION_ID_FILE: &str = "opencode-session.txt";

static SESSION_ID: OnceLock<String> = OnceLock::new();

/// 返回 OpenCode Go 请求所需的实例级稳定会话 ID。
///
/// 首次调用读取 `opencode-session.txt`（trim 后非空即用）；文件缺失或为空则
/// 生成 uuid v4 并尝试写回（先 `create_dir_all` 父目录，写失败忽略——进程内
/// 仍返回内存 ID，保证本进程内稳定）。后续调用命中 `OnceLock` 缓存。
pub fn opencode_session_id() -> String {
    SESSION_ID
        .get_or_init(|| {
            let path = crate::utils::nuphus_data_dir().join(SESSION_ID_FILE);
            load_or_create(&path)
        })
        .clone()
}

/// 从文件读取既有 ID；不可读 / 空内容时生成 uuid v4 并尝试持久化。
fn load_or_create(path: &Path) -> String {
    if let Ok(raw) = std::fs::read_to_string(path) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 连续两次调用返回相同非空串（OnceLock 缓存 + 文件读取路径稳定）。
    #[test]
    fn session_id_stable_across_calls() {
        let first = opencode_session_id();
        assert!(!first.is_empty());
        let second = opencode_session_id();
        assert_eq!(first, second);
    }
}
