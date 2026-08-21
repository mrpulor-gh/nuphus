//! Session 模块测试

#[cfg(test)]
mod tests {
    use crate::session::session::Session;
    use crate::session::types::*;

    #[test]
    fn test_new_session() {
        let session = Session::new();
        assert!(session.is_empty());
        assert_eq!(session.len(), 0);
    }

    #[test]
    fn test_push_user() {
        let mut session = Session::new();
        session.push_user("Hello".to_string());
        assert_eq!(session.len(), 1);
        assert_eq!(session.messages()[0].role, MessageRole::User);
    }

    #[test]
    fn test_push_assistant() {
        let mut session = Session::new();
        session.push_assistant(vec![ContentBlock::Text {
            text: "Hi there".to_string(),
            reasoning: None,
        }]);
        assert_eq!(session.len(), 1);
        assert_eq!(session.messages()[0].role, MessageRole::Assistant);
    }

    #[test]
    fn test_push_tool_result() {
        let mut session = Session::new();
        session.push_tool_result("tool-1".to_string(), "result".to_string(), false);
        assert_eq!(session.len(), 1);
        assert_eq!(session.messages()[0].role, MessageRole::Tool);
    }

    #[test]
    fn test_to_api_messages() {
        let mut session = Session::new();
        session.push_user("Hello".to_string());
        session.push_assistant(vec![ContentBlock::Text {
            text: "Hi".to_string(),
            reasoning: None,
        }]);

        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 2);
        assert_eq!(api_msgs[0]["role"], "user");
        assert_eq!(api_msgs[1]["role"], "assistant");
    }

    #[test]
    fn test_to_api_messages_preserves_reasoning_content() {
        let mut session = Session::new();
        session.push_user("task".to_string());

        // Assistant message with reasoning (simulating DeepSeek thinking mode)
        session.push_assistant(vec![
            ContentBlock::Text {
                text: "result".to_string(),
                reasoning: Some("deep thinking process".to_string()),
            },
            ContentBlock::ToolUse {
                id: "call-1".to_string(),
                name: "file_read".to_string(),
                input: serde_json::json!({"path": "/tmp/test"}),
            },
        ]);
        session.push_tool_result("call-1".to_string(), "file contents".to_string(), false);

        // Second assistant (no reasoning)
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);

        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 4); // user, assistant(tool_calls), tool, assistant

        // The first assistant message must have reasoning_content
        let first_asst = &api_msgs[1];
        assert_eq!(first_asst["role"], "assistant");
        assert_eq!(
            first_asst["reasoning_content"], "deep thinking process",
            "first assistant must preserve reasoning_content for DeepSeek multi-turn"
        );

        // The second assistant should NOT have reasoning_content
        let second_asst = &api_msgs[3];
        assert_eq!(second_asst["role"], "assistant");
        assert!(
            second_asst.get("reasoning_content").is_none(),
            "second assistant should not have reasoning_content"
        );
    }

    #[test]
    fn test_to_api_messages_reasoning_before_content_order() {
        // Verify that reasoning_content appears BEFORE content in the JSON output.
        // The order matters because some models may interpret field order as a signal
        // for output ordering (thinking before text vs text before thinking).
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![ContentBlock::Text {
            text: "result text".to_string(),
            reasoning: Some("deep thinking".to_string()),
        }]);

        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 2);

        // Serialize to string to inspect key order
        let asst = &api_msgs[1];
        let json_str = serde_json::to_string(asst).unwrap();

        // reasoning_content must appear before content in the JSON string
        let rc_pos = json_str
            .find("\"reasoning_content\"")
            .expect("reasoning_content must exist");
        let content_pos = json_str.find("\"content\"").expect("content must exist");
        assert!(
            rc_pos < content_pos,
            "reasoning_content ({}) must appear BEFORE content ({}) in JSON, got: {}",
            rc_pos,
            content_pos,
            json_str
        );
    }

    #[test]
    fn test_to_api_messages_reasoning_without_text() {
        // Test: DeepSeek sometimes returns only reasoning + tool_calls, no text
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![
            ContentBlock::Text {
                text: String::new(),
                reasoning: Some("thinking about what tool to use".to_string()),
            },
            ContentBlock::ToolUse {
                id: "call-2".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"cmd": "ls"}),
            },
        ]);
        session.push_tool_result("call-2".to_string(), "output".to_string(), false);

        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 3);

        // Assistant with empty text but has reasoning and tool_calls
        let asst = &api_msgs[1];
        assert_eq!(asst["role"], "assistant");
        assert!(asst.get("tool_calls").is_some(), "should have tool_calls");
        assert_eq!(
            asst["reasoning_content"], "thinking about what tool to use",
            "must preserve reasoning_content even when text is empty"
        );
    }

    #[test]
    fn test_strip_incomplete_tools_noop_on_clean_session() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);
        let len_before = session.len();
        session.strip_incomplete_tools();
        assert_eq!(
            session.len(),
            len_before,
            "clean session should be unchanged"
        );
    }

    #[test]
    fn test_strip_incomplete_tools_removes_orphaned_tool_use() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        // Assistant with ToolUse but NO matching ToolResult
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "orphan-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
        }]);
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);
        session.strip_incomplete_tools();
        // The assistant message with orphaned ToolUse should now have no content → removed
        // Only user + "done" assistant should remain
        assert_eq!(
            session.len(),
            2,
            "orphaned tool_use message should be removed"
        );
        assert_eq!(session.messages()[1].content.len(), 1);
        assert!(matches!(
            session.messages()[1].content[0],
            ContentBlock::Text { .. }
        ));
    }

    #[test]
    fn test_strip_incomplete_tools_removes_orphaned_tool_result() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        // ToolResult without preceding Assistant ToolUse
        session.push_tool_result("orphan-tool".to_string(), "result".to_string(), false);
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);
        assert_eq!(session.len(), 3);
        session.strip_incomplete_tools();
        // Orphaned tool result should be removed
        assert_eq!(session.len(), 2, "orphaned tool_result should be removed");
    }

    #[test]
    fn test_strip_incomplete_tools_preserves_matched_pair() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![
            ContentBlock::Text {
                text: "let me check".to_string(),
                reasoning: None,
            },
            ContentBlock::ToolUse {
                id: "call-1".to_string(),
                name: "file_read".to_string(),
                input: serde_json::json!({"path": "/tmp/test"}),
            },
        ]);
        session.push_tool_result("call-1".to_string(), "file contents".to_string(), false);
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);
        let len_before = session.len();
        session.strip_incomplete_tools();
        assert_eq!(
            session.len(),
            len_before,
            "matched pair should be preserved"
        );
    }

    #[test]
    fn test_strip_incomplete_tools_mixed_matched_and_orphaned() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        // Matched pair
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "call-1".to_string(),
            name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/a"}),
        }]);
        session.push_tool_result("call-1".to_string(), "content A".to_string(), false);
        // Orphaned ToolUse (no matching ToolResult)
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "orphan-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"cmd": "rm -rf /"}),
        }]);
        // Orphaned ToolResult (no matching ToolUse)
        session.push_tool_result(
            "orphan-tool".to_string(),
            "orphan result".to_string(),
            false,
        );
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);

        session.strip_incomplete_tools();

        // After cleanup: user, assistant(call-1), tool(call-1), assistant("done")
        assert_eq!(
            session.len(),
            4,
            "should remove orphaned messages but keep matched pairs"
        );
        // Verify call-1 is still there
        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 4);
        // Second message should be assistant with tool_calls for call-1
        assert_eq!(api_msgs[1]["role"], "assistant");
        assert!(api_msgs[1]["tool_calls"].is_array());
        // Third message should be tool result for call-1
        assert_eq!(api_msgs[2]["role"], "tool");
        assert_eq!(api_msgs[2]["tool_call_id"], "call-1");
    }

    // ════════════════════════════════════════════════════════════════
    // 图片处理矩阵：to_api_messages 行为（2026-08-08 大王决策：去自动 vision）
    //  ① Main（supports_vision=true）→ image_url 直发主模型
    //  ② Fallback（supports_vision=false，无论是否配置视觉模型）→ 保存临时 BMP + 路径占位，
    //     Agent 按需调 desktop_vision(image_path=路径, prompt=精准问题) 查看（不自动描述）
    // ════════════════════════════════════════════════════════════════
    fn session_with_image() -> Session {
        let mut session = Session::new();
        session.messages.push(Message {
            role: MessageRole::User,
            content: vec![ContentBlock::Image {
                url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
                    .to_string(),
            }],
            internal: false,
            timestamp: Some(crate::session::types::now_ms()),
        });
        session
    }

    #[test]
    fn test_to_api_messages_image_main_direct_image_url() {
        // ① 主模型支持视觉：image_url 直发，不走临时文件
        let session = session_with_image();
        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 1);
        let content = api_msgs[0]["content"].as_array().unwrap();
        assert_eq!(
            content.len(),
            1,
            "should contain exactly one block (image_url)"
        );
        assert_eq!(content[0]["type"], "image_url");
        assert!(content[0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png"));
    }

    #[test]
    fn test_to_api_messages_image_fallback_saves_temp_file() {
        // ② 主模型不支持：统一保存临时 BMP + 路径占位（旧描述缓存不再影响 transform）
        let session = session_with_image();

        let api_msgs = session.to_api_messages(false);
        assert_eq!(api_msgs.len(), 1);
        let content = api_msgs[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        let text = content[0]["text"].as_str().unwrap();
        assert!(
            text.contains("已保存至"),
            "主模型不支持时图片应保存临时文件+路径（Agent 按需 vision），实际: {text}"
        );
    }

    #[test]
    fn test_to_api_messages_image_none_saves_temp_file() {
        // ② 主模型不支持：保存临时 BMP + 路径占位
        let session = session_with_image();
        let api_msgs = session.to_api_messages(false);
        assert_eq!(api_msgs.len(), 1);
        let content = api_msgs[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        let text = content[0]["text"].as_str().unwrap();
        assert!(
            text.contains("已保存至"),
            "主模型不支持时图片应保存临时文件+路径，实际: {text}"
        );
    }

    #[test]
    fn test_to_api_messages_image_fallback_with_text() {
        // ② 变体：用户文本 + 图片 → 文本保留 + 图片保存路径
        let mut session = Session::new();
        session.messages.push(Message {
            role: MessageRole::User,
            content: vec![
                ContentBlock::Text {
                    text: "看看这张图".to_string(),
                    reasoning: None,
                },
                ContentBlock::Image {
                    url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
                        .to_string(),
                },
            ],
            internal: false,
            timestamp: Some(crate::session::types::now_ms()),
        });

        let api_msgs = session.to_api_messages(false);
        let content = api_msgs[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "text + 图片占位");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "看看这张图");
        assert_eq!(content[1]["type"], "text");
        assert!(content[1]["text"].as_str().unwrap().contains("已保存至"));
    }
}
