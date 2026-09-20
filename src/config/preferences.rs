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
    /// 会话工作台排序 · 组序维度（"bookmark" = 书签顺序，默认；"recent" = 按组内最近会话倒序）。
    ///
    /// `serde(default)` 保证老配置平滑升级；手改配置写入的非法值由
    /// [`UserPreferences::session_group_order`] 归一为默认值，不影响启动。
    #[serde(default = "default_session_group_order")]
    pub session_group_order: String,
    /// 会话工作台排序 · 组内排序键（"updated" = 更新时间倒序，默认；"created" = 创建时间，早的在上）。
    #[serde(default = "default_session_sort_key")]
    pub session_sort_key: String,
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

/// 会话工作台排序 · 组序维度：按书签顺序（默认，= 分组引入前的行为）。
pub const SESSION_GROUP_ORDER_BOOKMARK: &str = "bookmark";

/// 会话工作台排序 · 组序维度：按组内最近一次会话时间倒序。
pub const SESSION_GROUP_ORDER_RECENT: &str = "recent";

/// 会话工作台排序 · 组内键：按更新时间倒序（默认，= 分组引入前的行为）。
pub const SESSION_SORT_KEY_UPDATED: &str = "updated";

/// 会话工作台排序 · 组内键：按创建时间升序（早的在上）。
pub const SESSION_SORT_KEY_CREATED: &str = "created";

/// serde 缺省函数：老配置无组序维度字段时回落「按项目」。
fn default_session_group_order() -> String {
    SESSION_GROUP_ORDER_BOOKMARK.to_string()
}

/// serde 缺省函数：老配置无组内键字段时回落「更新时间」。
fn default_session_sort_key() -> String {
    SESSION_SORT_KEY_UPDATED.to_string()
}

/// 组序维度归一：只认 `"recent"`（去空白 + 忽略大小写），其余一律回落默认「按项目」。
///
/// 归一收敛在此，`list_shelf_sessions` / 排序命令 / 前端三处读数语义一致。
pub fn normalize_session_group_order(raw: &str) -> &'static str {
    if raw.trim().eq_ignore_ascii_case(SESSION_GROUP_ORDER_RECENT) {
        SESSION_GROUP_ORDER_RECENT
    } else {
        SESSION_GROUP_ORDER_BOOKMARK
    }
}

/// 组内排序键归一：只认 `"created"`（去空白 + 忽略大小写），其余一律回落默认「更新时间」。
pub fn normalize_session_sort_key(raw: &str) -> &'static str {
    if raw.trim().eq_ignore_ascii_case(SESSION_SORT_KEY_CREATED) {
        SESSION_SORT_KEY_CREATED
    } else {
        SESSION_SORT_KEY_UPDATED
    }
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            project_dir: String::new(),
            project_bookmarks: Vec::new(),
            session_group_order: default_session_group_order(),
            session_sort_key: default_session_sort_key(),
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

    /// 组序维度读数（归一后）：手改配置的非法值一律回落「按项目」。
    pub fn session_group_order(&self) -> &'static str {
        normalize_session_group_order(&self.session_group_order)
    }

    /// 组内排序键读数（归一后）：手改配置的非法值一律回落「更新时间」。
    pub fn session_sort_key(&self) -> &'static str {
        normalize_session_sort_key(&self.session_sort_key)
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
        assert_eq!(prefs.session_group_order(), SESSION_GROUP_ORDER_BOOKMARK);
        assert_eq!(prefs.session_sort_key(), SESSION_SORT_KEY_UPDATED);
    }

    /// 排序偏好落盘往返：合法值序列化保留、反序列化还原（重启后排序不丢）。
    #[test]
    fn sort_prefs_roundtrip_through_json() {
        let prefs = UserPreferences {
            session_group_order: SESSION_GROUP_ORDER_RECENT.to_string(),
            session_sort_key: SESSION_SORT_KEY_CREATED.to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert!(json.contains("\"session_group_order\":\"recent\""));
        assert!(json.contains("\"session_sort_key\":\"created\""));

        let back: UserPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_group_order(), SESSION_GROUP_ORDER_RECENT);
        assert_eq!(back.session_sort_key(), SESSION_SORT_KEY_CREATED);
    }

    /// 非法取值归一：手改配置写进的怪值 / 空串 / 大小写变体 / 带空白，
    /// 读数一律回落默认，且**不**把非法原值当作有效偏好。
    #[test]
    fn illegal_sort_prefs_fall_back_to_defaults() {
        // 大小写 / 首尾空白容错（手改配置常见形态）
        assert_eq!(
            normalize_session_group_order(" Recent "),
            SESSION_GROUP_ORDER_RECENT
        );
        assert_eq!(
            normalize_session_sort_key("Created"),
            SESSION_SORT_KEY_CREATED
        );
        // 合法值原样通过
        assert_eq!(
            normalize_session_group_order("bookmark"),
            SESSION_GROUP_ORDER_BOOKMARK
        );
        assert_eq!(
            normalize_session_sort_key("updated"),
            SESSION_SORT_KEY_UPDATED
        );
        // 非法值 / 空串 → 默认（不把非法原值当有效偏好）
        for raw in ["", "  ", "newest", "recentt", "时间"] {
            assert_eq!(
                normalize_session_group_order(raw),
                SESSION_GROUP_ORDER_BOOKMARK,
                "组序维度归一错误: {raw:?}"
            );
        }
        for raw in ["", "  ", "create", "oldest", "updated_at"] {
            assert_eq!(
                normalize_session_sort_key(raw),
                SESSION_SORT_KEY_UPDATED,
                "组内键归一错误: {raw:?}"
            );
        }

        let prefs = UserPreferences {
            session_group_order: "newest".to_string(),
            session_sort_key: String::new(),
            ..Default::default()
        };
        assert_eq!(prefs.session_group_order(), SESSION_GROUP_ORDER_BOOKMARK);
        assert_eq!(prefs.session_sort_key(), SESSION_SORT_KEY_UPDATED);
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
