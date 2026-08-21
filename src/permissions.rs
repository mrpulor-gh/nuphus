//! Permission system
//!
//! Two-layer separation: tool category switches + security protection:
//! - Upper layer: three groups of independent switches (file read/write / web search / system automation)
//! - Lower layer: security protection (popup confirmation, path detection, etc.), handled by security.rs
//!
//! Tool categories:
//!   FileAccess        — file read/write (file_read/write/edit/delete/copy etc., including search, diff)
//!   WebSearch         — web search (search_web, web_extract)
//!   SystemAutomation  — system automation (system_shell, process_kill, desktop control, scheduled tasks, etc.)
//!   Uncategorized tools (think, get_system_info etc. read-only tools) are always allowed

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tool category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    /// Core tools (think, get_system_info etc.) — not affected by switches, always allowed
    #[serde(rename = "core")]
    Core,
    /// File read/write — controlled by file_access switch
    #[serde(rename = "file")]
    FileAccess,
    /// Web search — controlled by web_search switch
    #[serde(rename = "web")]
    WebSearch,
    /// System automation — controlled by system_automation switch
    #[serde(rename = "system")]
    SystemAutomation,
}

/// Tool permission configuration — independent switches
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub file_access: bool,
    pub web_search: bool,
    pub system_automation: bool,
}

impl ToolPermissions {
    /// All off
    pub fn none() -> Self {
        Self {
            file_access: false,
            web_search: false,
            system_automation: false,
        }
    }

    /// All on
    pub fn all() -> Self {
        Self {
            file_access: true,
            web_search: true,
            system_automation: true,
        }
    }

    pub fn as_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

impl Default for ToolPermissions {
    fn default() -> Self {
        Self {
            file_access: true,
            web_search: true,
            system_automation: false,
        }
    }
}

/// Permission check result
#[derive(Debug, Clone)]
pub enum PermissionOutcome {
    Allow,
    Deny { reason: String },
}

impl PermissionOutcome {
    pub fn allow() -> Self {
        Self::Allow
    }
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Permission policy
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    permissions: ToolPermissions,
    tool_categories: HashMap<String, ToolCategory>,
}

impl PermissionPolicy {
    pub fn new(permissions: ToolPermissions) -> Self {
        Self {
            permissions,
            tool_categories: HashMap::new(),
        }
    }

    /// Batch register tool categories
    pub fn with_categories<I: IntoIterator<Item = (String, ToolCategory)>>(
        mut self,
        iter: I,
    ) -> Self {
        for (name, cat) in iter {
            self.tool_categories.insert(name, cat);
        }
        self
    }

    /// Get current permission configuration
    pub fn permissions(&self) -> ToolPermissions {
        self.permissions
    }

    /// Update permission flags while preserving registered tool categories.
    /// Enables runtime permission changes to take effect in the next tool call.
    pub fn update_permissions(&mut self, permissions: ToolPermissions) {
        self.permissions = permissions;
    }

    /// Check tool permission by registered category (all tools registered dynamically at startup)
    pub fn authorize(&self, tool_name: &str) -> PermissionOutcome {
        match self.tool_categories.get(tool_name).copied() {
            Some(ToolCategory::Core) => PermissionOutcome::Allow,
            Some(ToolCategory::FileAccess) if !self.permissions.file_access => {
                PermissionOutcome::deny(format!(
                    "tool '{}' requires file access permission, which is disabled",
                    tool_name
                ))
            }
            Some(ToolCategory::WebSearch) if !self.permissions.web_search => {
                PermissionOutcome::deny(format!(
                    "tool '{}' requires web search permission, which is disabled",
                    tool_name
                ))
            }
            Some(ToolCategory::SystemAutomation) if !self.permissions.system_automation => {
                PermissionOutcome::deny(format!(
                    "tool '{}' requires system automation permission, which is disabled",
                    tool_name
                ))
            }
            _ => PermissionOutcome::Allow,
        }
    }

    /// authorize by category (bypasses tool_categories map)
    pub fn authorize_by_category(
        &self,
        tool_name: &str,
        category: ToolCategory,
    ) -> PermissionOutcome {
        match category {
            ToolCategory::Core => PermissionOutcome::Allow,
            ToolCategory::FileAccess if !self.permissions.file_access => PermissionOutcome::deny(
                format!("tool '{}' requires file_read permission", tool_name),
            ),
            ToolCategory::WebSearch if !self.permissions.web_search => PermissionOutcome::deny(
                format!("tool '{}' requires web_search permission", tool_name),
            ),
            ToolCategory::SystemAutomation if !self.permissions.system_automation => {
                PermissionOutcome::deny(format!(
                    "tool '{}' requires system_automation permission",
                    tool_name
                ))
            }
            _ => PermissionOutcome::Allow,
        }
    }

    /// Check permission and return (bool, message)
    pub fn authorize_with_message(&self, tool_name: &str) -> (bool, String) {
        match self.authorize(tool_name) {
            PermissionOutcome::Allow => (true, format!("tool '{}' authorized", tool_name)),
            PermissionOutcome::Deny { reason } => (false, reason),
        }
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::new(ToolPermissions::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_permissions() {
        let p = ToolPermissions::default();
        assert!(p.file_access);
        assert!(p.web_search);
        assert!(!p.system_automation);
    }

    #[test]
    fn test_authorize_allowed() {
        let mut cat_map = HashMap::new();
        cat_map.insert("Read".to_string(), ToolCategory::FileAccess);
        cat_map.insert("search_web".to_string(), ToolCategory::WebSearch);

        let policy = PermissionPolicy::new(ToolPermissions {
            file_access: true,
            web_search: false,
            system_automation: false,
        })
        .with_categories(cat_map);

        assert!(policy.authorize("Read").is_allowed());
        assert!(!policy.authorize("search_web").is_allowed());
    }

    #[test]
    fn test_unknown_tool_allowed() {
        let policy = PermissionPolicy::new(ToolPermissions::none());
        assert!(policy.authorize("think").is_allowed());
    }

    #[test]
    fn test_desktop_prefix_blocked() {
        let mut cat_map = HashMap::new();
        cat_map.insert("desktop_mouse".to_string(), ToolCategory::SystemAutomation);
        cat_map.insert(
            "desktop_screenshot".to_string(),
            ToolCategory::SystemAutomation,
        );
        let policy = PermissionPolicy::new(ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: false,
        })
        .with_categories(cat_map);
        assert!(!policy.authorize("desktop_mouse").is_allowed());
        assert!(!policy.authorize("desktop_screenshot").is_allowed());
    }

    #[test]
    fn test_browser_prefix_blocked() {
        let mut cat_map = HashMap::new();
        cat_map.insert("browser_navigate".to_string(), ToolCategory::WebSearch);
        let policy = PermissionPolicy::new(ToolPermissions {
            file_access: true,
            web_search: false,
            system_automation: false,
        })
        .with_categories(cat_map);
        assert!(!policy.authorize("browser_navigate").is_allowed());
    }

    #[test]
    fn test_browser_prefix_allowed() {
        let policy = PermissionPolicy::new(ToolPermissions {
            file_access: true,
            web_search: true,
            system_automation: false,
        });
        assert!(policy.authorize("browser_navigate").is_allowed());
    }

    #[test]
    fn test_deny_reason() {
        let mut cat_map = HashMap::new();
        cat_map.insert("Write".to_string(), ToolCategory::FileAccess);
        let policy = PermissionPolicy::new(ToolPermissions {
            file_access: false,
            web_search: false,
            system_automation: false,
        })
        .with_categories(cat_map);

        if let PermissionOutcome::Deny { reason } = policy.authorize("Write") {
            assert!(reason.contains("requires file access"));
        } else {
            panic!("expected deny");
        }
    }
}
