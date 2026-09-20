//! User preference configuration — language, theme, and other persisted settings

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persisted identity of the user-picked external (fingerprint) browser.
///
/// The URL alone is not enough for reconnection: fingerprint browsers
/// (AdsPower & co.) typically launch with `--remote-debugging-port=0`, so a
/// reopened window listens on a NEW random port. With the exe path the running
/// process can be located and its actual debug port re-resolved (via cmdline
/// or `<user-data-dir>/DevToolsActivePort`) — see nuphus-browser's
/// `attach_external` self-healing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserIdentity {
    /// Human-readable platform name (e.g. "AdsPower") for UI/error display.
    pub name: String,
    /// Browser executable path — locates the running process.
    pub exe_path: String,
    /// `--user-data-dir` the window was launched with; fallback for
    /// DevToolsActivePort resolution when the process cmdline is unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<String>,
}

/// 项目书签：项目中心（输入框项目弹窗）维护的工作目录快捷入口。
///
/// 单一事实源落在这里（与 `project_dir` 同源），前端不再各自维护
/// localStorage 副本 —— 历史上前后端两套键（`nuphus_projects` /
/// `nuphus_project_bookmarks`）互不相通，是「Ctrl+K 加的书签在输入框看不到」
/// 的根因。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectBookmark {
    /// 展示名（默认取目录末段，可自定义）
    pub name: String,
    /// 绝对路径
    pub path: String,
    /// 归档标记：true = 该文件夹在会话工作台中隐藏（可从「已归档文件夹」恢复）。
    ///
    /// 只影响会话台分组展示——**不触碰** `project_dir`，因此记忆检索的项目过滤
    /// 行为不受影响。`serde(default)` 保证老配置（无此字段）平滑升级为未归档。
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    /// User language preference, default "zh-CN"
    pub language: String,
    /// User-set project directory path
    #[serde(default)]
    pub project_dir: String,
    /// 项目书签列表（项目中心维护）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_bookmarks: Vec<ProjectBookmark>,
    /// 会话分组折叠上限（全局单值，会话工作台「项目文件夹」每组默认折叠的会话数）。
    ///
    /// `serde(default)` 保证老配置平滑升级：缺字段即 6。0 视为未设置 → 读数回落默认值
    /// （见 [`UserPreferences::session_group_limit`]）。
    #[serde(default = "default_session_group_collapsed_limit")]
    pub session_group_collapsed_limit: u32,
    /// External browser CDP endpoint (tri-state):
    /// `None` = never configured (leave any servers.yaml env untouched);
    /// `Some("")` = user explicitly switched back to managed Chrome (strip the env);
    /// `Some(url)` = attach all browser tools to this endpoint (e.g. fingerprint browser).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_cdp_url: Option<String>,
    /// Identity of the picked external browser. Only meaningful together with
    /// `browser_cdp_url: Some(url)`; cleared when switching back to managed
    /// Chrome or when a URL is set without identity (legacy/manual path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_identity: Option<BrowserIdentity>,
}

/// 会话分组折叠上限默认值（全局单值）。
pub const DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT: u32 = 6;

/// serde 缺省函数：老配置无该字段时回落默认上限。
fn default_session_group_collapsed_limit() -> u32 {
    DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            project_dir: String::new(),
            project_bookmarks: Vec::new(),
            session_group_collapsed_limit: DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT,
            browser_cdp_url: None,
            browser_identity: None,
        }
    }
}

impl UserPreferences {
    /// 会话分组折叠上限读数：0（手改配置/异常值）视为未设置 → 默认值。
    ///
    /// 收敛在此，避免 0 让每个分组都折叠成空列表。
    pub fn session_group_limit(&self) -> u32 {
        if self.session_group_collapsed_limit == 0 {
            DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT
        } else {
            self.session_group_collapsed_limit
        }
    }

    pub fn load() -> Self {
        let path = Self::path();
        if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            let prefs = UserPreferences::default();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&prefs) {
                let _ = std::fs::write(&path, json);
            }
            prefs
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create prefs dir failed: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize prefs failed: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("write prefs failed: {}", e))?;
        Ok(())
    }

    fn path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".nuphus/preferences.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 老配置平滑升级：无 collapsed_limit / 无 archived 字段的 preferences.json
    /// 必须可反序列化并取默认值（迁移不报错、不丢字段）。
    #[test]
    fn legacy_preferences_json_deserializes_with_defaults() {
        let legacy = r#"{
            "language": "zh-CN",
            "project_dir": "E:\\work\\A",
            "project_bookmarks": [{"name": "A", "path": "E:\\work\\A"}],
            "browser_cdp_url": null
        }"#;
        let prefs: UserPreferences = serde_json::from_str(legacy).expect("老配置必须可反序列化");
        assert_eq!(
            prefs.session_group_collapsed_limit,
            DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT
        );
        assert_eq!(prefs.session_group_limit(), 6, "缺字段时折叠上限默认 6");
        assert_eq!(prefs.project_dir, "E:\\work\\A");
        assert!(!prefs.project_bookmarks[0].archived, "老书签默认未归档");
        assert_eq!(prefs.project_bookmarks[0].name, "A");
    }

    /// 0 = 未设置（手改配置/异常值）→ 读数回落默认值，不把每个组折叠成空列表。
    #[test]
    fn zero_collapsed_limit_falls_back_to_default() {
        let prefs = UserPreferences {
            session_group_collapsed_limit: 0,
            ..Default::default()
        };
        assert_eq!(
            prefs.session_group_limit(),
            DEFAULT_SESSION_GROUP_COLLAPSED_LIMIT
        );
    }

    /// 归档标记往返：序列化保留、反序列化还原（前端 Phase 2 依据该字段隐藏分组）。
    #[test]
    fn bookmark_archived_flag_roundtrip() {
        let bm = ProjectBookmark {
            name: "A".to_string(),
            path: "E:\\work\\A".to_string(),
            archived: true,
        };
        let json = serde_json::to_string(&bm).unwrap();
        assert!(json.contains("\"archived\":true"));
        let back: ProjectBookmark = serde_json::from_str(&json).unwrap();
        assert!(back.archived);
    }
}
