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

    /// issue #9 RC1：会话内 System（提炼摘要、安全警告、分裂锚点、ExecPool 压缩）是注进
    /// 上下文的**内容**，必须以 user 角色下发。原样发 system 会让报文成为
    /// [system(主提示), system(摘要), user…]，llama.cpp 的 Jinja 模板硬拒 → 确定性 HTTP 500
    /// （云端模板静默合并所以只在本地后端暴露）；Anthropic 适配器还会把这段内容换成占位串。
    #[test]
    fn test_to_api_messages_serializes_internal_system_as_user() {
        let mut session = Session::new();
        session.push_user("earlier question".to_string());
        session.replace_with_distill("SUMMARY-OF-EARLIER-TURNS");

        let api_msgs = session.to_api_messages(true);
        assert_eq!(api_msgs.len(), 1, "提炼后会话仅剩摘要一条");
        assert_eq!(
            api_msgs[0]["role"], "user",
            "会话内 System 必须以 user 下发，否则报文里会出现 index≥1 的 system"
        );
        // 内容必须完整保留（形状无关断言：Anthropic 侧不再被占位串替换掉）
        let serialized = serde_json::to_string(&api_msgs[0]).unwrap();
        assert!(
            serialized.contains("SUMMARY-OF-EARLIER-TURNS"),
            "摘要内容必须完整下发: {serialized}"
        );

        // push_system 安全警告走同一条路径，同样不得以 system 下发
        session.push_system("SAFETY-NOTE".to_string());
        let api_msgs = session.to_api_messages(true);
        assert!(
            api_msgs.iter().all(|m| m["role"] != "system"),
            "任何会话内消息都不得以 system 角色出现在 messages 数组里"
        );
    }

    /// issue #9「附带发现」：提炼后必须清零 api_input_tokens。触发判据
    /// （`agent/distill.rs`）在 api_input_tokens > 0 时直接取它，不清零则旧峰值
    /// （如 112498）会让 force refine 每轮都再次触发——即使请求能发出去也是内存态死循环。
    #[test]
    fn test_distill_resets_api_input_tokens() {
        let mut session = Session::new();
        session.push_user("x".repeat(4000));
        session.update_api_input_tokens(112_498);
        assert_eq!(session.api_input_tokens, 112_498);

        session.replace_with_distill("short summary");
        assert_eq!(session.api_input_tokens, 0, "replace_with_distill 必须清零");
        assert!(
            session.estimate_token_usage() < 102_400,
            "清零后不得再越过 force_limit，否则下一轮必然重炼"
        );

        // 累积路径（同一 session 二次提炼）同样必须清零
        session.update_api_input_tokens(200_000);
        session.accumulate_distill("second summary");
        assert_eq!(session.api_input_tokens, 0, "accumulate_distill 必须清零");
        assert!(session.estimate_token_usage() < 102_400);
    }

    /// 界面上的 ctx 占用必须来自官方 usage（input + output），不掺字符估算。
    /// 缺陷背景：轮次结束曾用 `estimate_token_usage()`，其 `max(api_input, 字符/4)`
    /// 会让字符估算压过 API 真实读数 ——「结束后的 ctx 已用值 ≠ 最后一次上报的
    /// input_tokens」。现在界面一律走 `context_occupancy()`。
    #[test]
    fn test_context_occupancy_uses_official_input_plus_output() {
        let mut session = Session::new();
        // 一次典型末轮：prompt 130_801，产出 199（含 reasoning 段）
        session.update_api_input_tokens(130_801);
        session.update_api_output_tokens(199);
        // = 下一次请求的提示词规模，全部官方
        assert_eq!(session.context_occupancy(), 131_000);

        // 多轮：后者覆盖前者（不是累加——占用是「当前规模」不是「累计消耗」）
        session.update_api_input_tokens(132_500);
        session.update_api_output_tokens(240);
        assert_eq!(session.context_occupancy(), 132_740);

        // 字符数再大也不能压过官方读数（estimate 仍可被内部启发式用，但不进界面）
        session.push_user("x".repeat(2_000_000));
        assert_eq!(session.context_occupancy(), 132_740);
        assert!(
            session.estimate_token_usage() > 132_740,
            "估算仅供内部启发式"
        );
    }

    /// 提炼后占用读数随之作废（0 = 未知 → 界面显示 "--" 或调用方退回估算），
    /// 不得残留提炼前的峰值。
    #[test]
    fn test_context_occupancy_reset_after_distill() {
        let mut session = Session::new();
        session.update_api_input_tokens(112_498);
        session.update_api_output_tokens(900);
        session.replace_with_distill("short summary");
        assert_eq!(session.context_occupancy(), 0, "提炼后占用必须归零");

        session.update_api_input_tokens(50_000);
        session.update_api_output_tokens(120);
        session.accumulate_distill("second summary");
        assert_eq!(session.context_occupancy(), 0, "accumulate 同样归零");
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
    // seal_interrupted_tools：中断后封口未配对 ToolUse（2026-09-29 大王决策）
    // 原则：ToolUse 已发出的副作用不会随中断消失，必须把「被用户强制中断、
    // 该调用无结果」写进历史，否则下一轮模型在失真的世界状态上继续推理。
    // ════════════════════════════════════════════════════════════════

    /// 封口必须补 ToolResult，且 ToolUse 本身不能被删
    #[test]
    fn test_seal_interrupted_tools_keeps_use_and_adds_result() {
        let mut session = Session::new();
        session.push_user("删掉 /tmp/a".to_string());
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "call-write-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"cmd": "rm /tmp/a"}),
        }]);

        session.seal_interrupted_tools();

        assert_eq!(
            session.len(),
            3,
            "user + assistant(tool_use) + sealed tool_result"
        );
        // ToolUse 仍在原地
        match &session.messages()[1].content[0] {
            ContentBlock::ToolUse { id, .. } => assert_eq!(id, "call-write-1"),
            other => panic!("ToolUse 应保留，得到 {other:?}"),
        }
        // 紧跟其后的 tool result 指向同一个 id，且声明中断
        assert_eq!(session.messages()[2].role, MessageRole::Tool);
        match &session.messages()[2].content[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "call-write-1");
                assert!(is_error, "无结果的调用应标记为 error");
                assert!(
                    content.contains("用户强制中断"),
                    "必须写明中断事实: {content}"
                );
            }
            other => panic!("应补 ToolResult，得到 {other:?}"),
        }
    }

    /// 封口后 API 消息必须配对合法（这是封口存在的主要理由）
    #[test]
    fn test_seal_interrupted_tools_makes_api_messages_paired() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
        }]);
        session.seal_interrupted_tools();

        let api = session.to_api_messages(true);
        assert_eq!(api.len(), 3);
        assert_eq!(api[1]["role"], "assistant");
        assert!(api[1]["tool_calls"].is_array());
        assert_eq!(api[2]["role"], "tool");
        assert_eq!(api[2]["tool_call_id"], "call-1");
    }

    /// 已配对的 ToolUse 不得被重复封口（幂等）
    #[test]
    fn test_seal_interrupted_tools_is_idempotent() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![ContentBlock::ToolUse {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
        }]);
        session.push_tool_result("call-1".to_string(), "real output".to_string(), false);

        session.seal_interrupted_tools();
        let len_after_first = session.len();
        session.seal_interrupted_tools();

        assert_eq!(
            session.len(),
            len_after_first,
            "已配对的 ToolUse 不应被重复封口"
        );
        // 原始 tool result 内容必须保持不被覆盖
        match &session.messages()[2].content[0] {
            ContentBlock::ToolResult { content, .. } => {
                assert_eq!(content, "real output", "不得覆盖已有真实结果");
            }
            other => panic!("得到 {other:?}"),
        }
    }

    /// 干净会话封口应为 no-op
    #[test]
    fn test_seal_interrupted_tools_noop_on_clean_session() {
        let mut session = Session::new();
        session.push_user("task".to_string());
        session.push_assistant(vec![ContentBlock::Text {
            text: "done".to_string(),
            reasoning: None,
        }]);
        let before = session.len();
        session.seal_interrupted_tools();
        assert_eq!(session.len(), before);
    }

    /// 封口与 strip 的分工：strip 仍用于错误场景（删除脏数据），
    /// seal 用于中断场景（保留痕迹）。两者对同一会话结果不同。
    #[test]
    fn test_seal_vs_strip_division_of_labour() {
        let build = || {
            let mut s = Session::new();
            s.push_user("task".to_string());
            s.push_assistant(vec![ContentBlock::ToolUse {
                id: "call-1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"cmd": "ls"}),
            }]);
            s
        };

        let mut stripped = build();
        stripped.strip_incomplete_tools();
        assert_eq!(stripped.len(), 1, "strip 删除痕迹：只剩 user");

        let mut sealed = build();
        sealed.seal_interrupted_tools();
        assert_eq!(sealed.len(), 3, "seal 保留痕迹并补中断说明");
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
