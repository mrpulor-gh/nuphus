pub mod config;
pub mod schema_fix;
pub mod transport;

pub use config::ChatCompletionsConfig;
pub use transport::ChatCompletionsTransport;
// `sanitize_moonshot_tools` and `is_moonshot_model` have moved to
// `crate::config::providers::kimi`. The per-Provider quirks are now
// accessed via `Provider::quirks()`.
// `sanitize_tool_name` is pub(crate) — accessible within the crate,
// used by transport.rs in this module and schema_fix.rs for the core logic.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transports::StreamEvent;

    // ── Auth header construction ──

    #[test]
    fn test_endpoint_for_deepseek() {
        let config = ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        };
        assert_eq!(
            config.endpoint(),
            "https://api.deepseek.com/chat/completions"
        );
    }

    #[test]
    fn test_endpoint_strips_trailing_slash() {
        let config = ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com/".into(),
            model: "deepseek-v4-pro".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        };
        assert_eq!(
            config.endpoint(),
            "https://api.deepseek.com/chat/completions"
        );
    }

    #[test]
    fn test_build_request_body_no_reasoning() {
        let config = ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks {
                requires_reasoning_echo: true,
                ..Default::default()
            },
            reasoning_effort: None,
        };
        let transport = ChatCompletionsTransport::new(config);
        let request = crate::api::MessageRequest::new(
            "deepseek-v4-pro",
            vec![
                serde_json::json!({"role": "user", "content": "Hello"}),
                serde_json::json!({"role": "assistant", "content": "Hi there!"}),
                serde_json::json!({"role": "user", "content": "How are you?"}),
            ],
        );
        let body = transport.build_request_body(&request);
        let msgs = body["messages"].as_array().unwrap();
        // reasoning_content padding is driven by quirks.requires_reasoning_echo.
        // The test enables it explicitly to verify the padding logic.
        let asst = msgs.iter().find(|m| m["role"] == "assistant").unwrap();
        assert!(
            asst.get("reasoning_content").is_some(),
            "requires_reasoning_echo: true should trigger reasoning_content padding"
        );
    }

    #[test]
    fn test_build_request_body_reasoning_effort_sent() {
        for effort in ["low", "high", "max"] {
            let config = ChatCompletionsConfig {
                name: "deepseek".into(),
                api_key: "sk-test".into(),
                base_url: "https://api.deepseek.com".into(),
                model: "deepseek-v4-flash".into(),
                timeout_secs: 30,
                auth_header: "authorization".into(),
                auth_prefix: "Bearer ".into(),
                provider_kind: Some(crate::api::ProviderKind::DeepSeek),
                quirks: crate::config::provider::ProviderQuirks {
                    requires_reasoning_echo: true,
                    supports_reasoning_effort: true,
                    ..Default::default()
                },
                reasoning_effort: Some(effort.to_string()),
            };
            let transport = ChatCompletionsTransport::new(config);
            let request = crate::api::MessageRequest::new("deepseek-v4-flash", vec![]);
            let body = transport.build_request_body(&request);
            assert_eq!(
                body["reasoning_effort"],
                serde_json::json!(effort),
                "configured effort {} should be sent as reasoning_effort",
                effort
            );
            assert_eq!(
                body["thinking"],
                serde_json::json!({"type": "enabled"}),
                "thinking mode should stay enabled alongside reasoning_effort"
            );
        }
    }

    #[test]
    fn test_build_request_body_reasoning_effort_not_configured() {
        // No reasoning_effort in config → parameter must be absent (provider default).
        let config = ChatCompletionsConfig {
            name: "deepseek".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: Some(crate::api::ProviderKind::DeepSeek),
            quirks: crate::config::provider::ProviderQuirks {
                requires_reasoning_echo: true,
                supports_reasoning_effort: true,
                ..Default::default()
            },
            reasoning_effort: None,
        };
        let transport = ChatCompletionsTransport::new(config);
        let request = crate::api::MessageRequest::new("deepseek-v4-flash", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("reasoning_effort").is_none(),
            "unconfigured effort must not appear in the request body"
        );
    }

    #[test]
    fn test_build_request_body_no_max_tokens_when_unconfigured() {
        // Regression pin: when neither the request nor providers.toml sets
        // max_tokens, the field must be OMITTED so the provider's official
        // default (≫ 8K) applies. e22542c broke this by forcing 8192, which
        // truncated long thinking streams for reasoning models.
        let config = ChatCompletionsConfig {
            name: "deepseek".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: Some(crate::api::ProviderKind::DeepSeek),
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        };
        let transport = ChatCompletionsTransport::new(config);
        let request = crate::api::MessageRequest::new("deepseek-v4-flash", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("max_tokens").is_none(),
            "unconfigured max_tokens must be omitted from request body (got {:?})",
            body.get("max_tokens")
        );

        // Explicit request max_tokens → field present
        let request =
            crate::api::MessageRequest::new("deepseek-v4-flash", vec![]).with_max_tokens(16_384);
        let body = transport.build_request_body(&request);
        assert_eq!(body["max_tokens"], serde_json::json!(16_384));
    }

    #[test]
    fn test_build_request_body_reasoning_effort_ignored_for_other_provider() {
        // Default quirks (supports_reasoning_effort=false) → even a configured
        // value must NOT leak into other providers' request bodies.
        let config = ChatCompletionsConfig {
            name: "openai".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.openai.com".into(),
            model: "gpt-4o".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: Some(crate::api::ProviderKind::OpenAI),
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: Some("high".to_string()),
        };
        let transport = ChatCompletionsTransport::new(config);
        let request = crate::api::MessageRequest::new("gpt-4o", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("reasoning_effort").is_none(),
            "providers without supports_reasoning_effort must not receive the field"
        );
        assert!(
            body.get("thinking").is_none(),
            "providers without requires_reasoning_echo must not receive the thinking toggle"
        );
    }

    #[test]
    fn test_build_request_body_reasoning_effort_skipped_with_tools() {
        // Same tool gate as the thinking toggle: tool-calling requests must stay
        // byte-identical to current behavior (no reasoning_effort).
        let config = ChatCompletionsConfig {
            name: "deepseek".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: Some(crate::api::ProviderKind::DeepSeek),
            quirks: crate::config::provider::ProviderQuirks {
                requires_reasoning_echo: true,
                supports_reasoning_effort: true,
                effort_excludes_tools: true,
                ..Default::default()
            },
            reasoning_effort: Some("max".to_string()),
        };
        let transport = ChatCompletionsTransport::new(config);
        let request =
            crate::api::MessageRequest::new("deepseek-v4-flash", vec![]).with_tools(vec![
                crate::api::ToolDefinition::new("get_weather", serde_json::json!({})),
            ]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("reasoning_effort").is_none(),
            "reasoning_effort must not be sent when tools are present (thinking gate)"
        );
        assert!(
            body.get("thinking").is_none(),
            "thinking toggle must not be sent when tools are present"
        );
    }

    #[test]
    fn test_with_model_preserves_other_config() {
        let config = ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-original".into(),
            base_url: "https://api.openai.com".into(),
            model: "gpt-4o".into(),
            timeout_secs: 60,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        };
        let new_config = config.with_model("gpt-4o-mini");
        assert_eq!(new_config.model, "gpt-4o-mini");
        assert_eq!(new_config.api_key, "sk-original");
        assert_eq!(new_config.base_url, "https://api.openai.com");
        assert_eq!(new_config.timeout_secs, 60);
    }

    #[test]
    fn test_with_provider_kind_sets_explicit_kind() {
        let config = ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.minimax.com/v1".into(),
            model: "MiniMax-M1".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        };
        let new_config = config.with_provider_kind(crate::api::ProviderKind::DeepSeek);
        assert_eq!(
            new_config.provider_kind,
            Some(crate::api::ProviderKind::DeepSeek)
        );
        assert_eq!(new_config.model, "MiniMax-M1");
    }

    // ── SSE parsing ──

    #[test]
    fn test_parse_sse_single_delta() {
        let body = r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"}}]}

data: [DONE]
"#;
        let events = ChatCompletionsTransport::parse_sse(body, "").unwrap();
        let text: String = events
            .iter()
            .filter_map(|e| {
                if let StreamEvent::TextDelta(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, "Hello");
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done)));
    }

    #[test]
    fn test_parse_sse_multiple_deltas() {
        let body = r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"}}]}

data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":" world"}}]}

data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"!"}}]}

data: [DONE]
"#;
        let events = ChatCompletionsTransport::parse_sse(body, "").unwrap();
        let text: String = events
            .iter()
            .filter_map(|e| {
                if let StreamEvent::TextDelta(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, "Hello world!");
    }

    #[test]
    fn test_parse_sse_usage_then_done() {
        let body = r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hi"}}]}

data: {"id":"1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}

data: [DONE]
"#;
        let events = ChatCompletionsTransport::parse_sse(body, "").unwrap();
        let has_usage = events
            .iter()
            .any(|e| matches!(e, StreamEvent::Usage { .. }));
        assert!(has_usage, "Should contain Usage event");
    }

    #[test]
    fn test_parse_sse_empty_body() {
        let events = ChatCompletionsTransport::parse_sse("", "").unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Done));
    }

    #[test]
    fn test_parse_sse_only_done() {
        let events = ChatCompletionsTransport::parse_sse("data: [DONE]", "").unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Done));
    }

    // ── Non-streaming parsing ──

    #[test]
    fn test_parse_non_streaming_text() {
        let json = r#"{
            "id": "1",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello! How can I help you today?"
                }
            }]
        }"#;
        let events = ChatCompletionsTransport::parse_non_streaming(json, "").unwrap();
        let text: String = events
            .iter()
            .filter_map(|e| {
                if let StreamEvent::TextDelta(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, "Hello! How can I help you today?");
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done)));
    }

    #[test]
    fn test_parse_non_streaming_api_error() {
        let json = r#"{
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "code": "invalid_api_key"
            }
        }"#;
        let result = ChatCompletionsTransport::parse_non_streaming(json, "");
        assert!(result.is_err(), "API error should be propagated");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid API key"),
            "Error should contain the API message"
        );
    }
}
