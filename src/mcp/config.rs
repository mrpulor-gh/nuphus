//! MCP server configuration loader.
//!
//! Reads `plugin/mcp/servers.yaml` at startup / first use.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 启动命令
    pub command: String,
    /// 启动参数
    #[serde(default)]
    pub args: Vec<String>,
    /// 环境变量
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 调用超时 ms，默认 30000
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Nuphus 启动时自动拉起？
    #[serde(default)]
    pub auto_start: bool,
}

fn default_timeout() -> u64 {
    30000
}

/// Top-level MCP configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub servers: HashMap<String, ServerConfig>,
}

/// Load MCP configuration from `plugin/mcp/servers.yaml`.
/// Returns empty config if the file does not exist.
pub fn load_config() -> Result<McpConfig, String> {
    let path = crate::utils::workspace_root()
        .join("plugin")
        .join("mcp")
        .join("servers.yaml");
    match std::fs::read_to_string(&path) {
        Ok(yaml) => serde_yaml::from_str(&yaml)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(
                "[mcp] No servers.yaml found at {}, using empty config",
                path.display()
            );
            Ok(McpConfig {
                servers: HashMap::new(),
            })
        }
        Err(e) => Err(format!("Failed to read {}: {}", path.display(), e)),
    }
}
