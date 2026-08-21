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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    /// User language preference, default "zh-CN"
    pub language: String,
    /// User-set project directory path
    #[serde(default)]
    pub project_dir: String,
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

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            project_dir: String::new(),
            browser_cdp_url: None,
            browser_identity: None,
        }
    }
}

impl UserPreferences {
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
