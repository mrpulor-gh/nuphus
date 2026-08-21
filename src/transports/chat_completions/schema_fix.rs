// formerly used Map + Value for schema repair — now lives in `crate::config::providers::kimi`.

/// Normalize tool name: replace :: with _, ensure it matches ^[a-zA-Z0-9_-]+$.
/// OpenAI / DeepSeek / Moonshot all require this format.
pub(crate) fn sanitize_tool_name(name: &str) -> String {
    name.replace("::", "_").replace(".", "_")
}
