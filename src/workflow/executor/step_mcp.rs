//! MCP 步骤执行器
//!
//! 通过 MCP client pool 调用外部 MCP server 的工具。

use super::*;

impl Executor {
    /// 执行 MCP 工具调用步骤
    pub(super) async fn execute_mcp_step(
        &self,
        step: &Step,
        mcp: &McpDef,
        variables: &mut HashMap<String, serde_json::Value>,
    ) -> crate::Result<String> {
        // 1. 解析 params 中的变量引用
        let resolved_params = Self::resolve_vars(&mcp.with, variables);

        // 2. 读取超时配置
        let timeout_ms = step.timeout_secs.map(|secs| secs * 1000).unwrap_or(30000);

        // 3. 调用 MCP 工具（call_tool 已是 async，直接 await）
        let server = mcp.server.clone();
        let tool = mcp.tool.clone();
        let result = crate::mcp::client::call_tool(&server, &tool, resolved_params, timeout_ms)
            .await
            .map_err(|e| {
                crate::NuphusError::agent(format!(
                    "MCP step '{}' ({}::{}): {}",
                    step.name, mcp.server, mcp.tool, e
                ))
            })?;

        // 4. 从 MCP 响应中提取文本内容
        let output = extract_mcp_content(&result)?;

        // 5. 根据 capture 将输出写入变量池
        super::variables::capture_output(&step.capture, &output, variables)?;

        Ok(output)
    }
}

/// 从 MCP tools/call 响应中提取 content 文本
/// MCP 响应格式: {"result": {"content": [{"type": "text", "text": "..."}]}}
fn extract_mcp_content(response: &serde_json::Value) -> crate::Result<String> {
    let content_arr = response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| crate::NuphusError::agent("MCP 响应不含 content 数组".to_string()))?;

    let texts: Vec<String> = content_arr
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                item.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    if texts.is_empty() {
        Err(crate::NuphusError::agent("MCP 响应无文本内容".to_string()))
    } else {
        Ok(texts.join("\n"))
    }
}
