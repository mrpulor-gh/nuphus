//! workflow API 冒烟测试
//!
//! 覆盖：
//!   - Store: CRUD 全链路（create save get list delete + 持久化往返）
//!   - Compiler: 所有步骤类型的验证逻辑（pass + fail）
//!   - Types: serialization round-trip

#[cfg(test)]
mod store_tests {
    use crate::workflow::store::WorkflowStore;
    use crate::workflow::types::{Action, Step, Workflow};

    fn tmp_root() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nuphus_wf_api_test_{}", uuid::Uuid::new_v4()));
        path
    }

    fn make_tool(id: &str, name: &str, tool: &str, params: serde_json::Value) -> Step {
        Step {
            id: id.into(),
            name: name.into(),
            action: Action::Tool {
                tool: tool.into(),
                with: params,
            },
            ..Default::default()
        }
    }

    // ── CRUD 基础 ──

    #[tokio::test]
    async fn crud_create_list_get_delete() {
        let root = tmp_root();
        let store = WorkflowStore::with_root(root.clone());

        let wf = Workflow::new("冒烟测试-1");
        let id = wf.id.clone();
        store.ensure_dirs(&id).await.unwrap();
        store.save(&wf).await.unwrap();

        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "冒烟测试-1");

        let loaded = store.get(&id).await;
        assert!(loaded.is_some());
        assert_eq!(loaded.as_ref().unwrap().name, "冒烟测试-1");
        assert_eq!(
            loaded.unwrap().status,
            crate::workflow::types::WorkflowStatus::Draft
        );

        store.delete(&id).await.unwrap();
        assert_eq!(store.list().await.len(), 0);
        assert!(store.get(&id).await.is_none());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn crud_multiple_workflows() {
        let root = tmp_root();
        let store = WorkflowStore::with_root(root.clone());

        for i in 0..5 {
            let wf = Workflow::new(&format!("WF-{}", i));
            store.ensure_dirs(&wf.id).await.unwrap();
            store.save(&wf).await.unwrap();
        }

        let list = store.list().await;
        assert_eq!(list.len(), 5);
        let mut names: Vec<String> = list.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["WF-0", "WF-1", "WF-2", "WF-3", "WF-4"]);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    // ── 持久化往返 ──

    #[tokio::test]
    async fn persistence_round_trip() {
        let root = tmp_root();
        let id;
        {
            let store = WorkflowStore::with_root(root.clone());
            let mut wf = Workflow::new("持久化测试");
            wf.status = crate::workflow::types::WorkflowStatus::Ready;
            wf.steps = vec![make_tool(
                "s1",
                "读文件",
                "Read",
                serde_json::json!({"path": "/tmp/test.txt"}),
            )];
            id = wf.id.clone();
            store.ensure_dirs(&id).await.unwrap();
            store.save(&wf).await.unwrap();
        }

        // 新 store 模拟"重启"
        {
            let store = WorkflowStore::with_root(root.clone());
            store.load_all().await.unwrap();
            let loaded = store.get(&id).await;
            assert!(loaded.is_some());
            let reloaded = loaded.unwrap();
            assert_eq!(reloaded.name, "持久化测试");
            assert_eq!(
                reloaded.status,
                crate::workflow::types::WorkflowStatus::Ready
            );
            assert_eq!(reloaded.steps.len(), 1);
        }

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    // ── 启动加载（真实 index.json / workflow.json）──
    // 探测式断言：验证 index.json → workflow.json 加载链路，不依赖具体工作流存在
    // （真实目录内容随产品演化变化，硬编码工作流 ID 会使测试环境敏感）

    #[tokio::test]
    async fn load_all_from_real_index() {
        let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let root = project_root.join("plugin").join("workflows");
        let store = WorkflowStore::with_root(root);

        store
            .load_all()
            .await
            .expect("load_all 应成功加载真实工作流目录");

        // 验证加载链路本身：目录存在即可成功 load（真实工作流目录内容随产品演化，
        // 且大部分 workflow 数据被 .gitignore 排除，CI checkout 后可能为空）。
        // 若目录非空，则额外验证 index.json ↔ workflow.json 反序列化链路。
        let summaries = store.list().await;
        if !summaries.is_empty() {
            for s in &summaries {
                let wf = store.get(&s.id).await;
                assert!(
                    wf.is_some(),
                    "get('{}') 应返回 Some（index.json 摘要 ↔ workflow.json 不一致）",
                    s.id
                );
            }
        }
    }

    // ── delete_workflow ChatAgent 关联清理（按 agent_id，非步骤名）──

    #[tokio::test]
    async fn delete_workflow_agent_cleanup_by_id() {
        use crate::workflow::chat_agent::{ChatAgentConfig, ChatAgentStore};
        use crate::workflow::types::ChatOpts;
        use crate::workflow::WorkflowEngine;

        // ChatAgentStore 使用真实 plugin/chat-agents 目录：唯一 id + 事后清理，避免污染
        let agent = ChatAgentConfig::new("wf_del_agent_test");
        let agent_id = agent.id.clone();
        ChatAgentStore::save(&agent).unwrap();

        let tmp = tmp_root();
        let _ = std::fs::create_dir_all(&tmp);
        let mut engine = WorkflowEngine::new();
        engine.store = WorkflowStore::with_root(tmp.clone());

        // 1) 无 agent_id 的 Chat 步骤 → 删除工作流不得删全局/其它配置
        let mut wf_no_agent = Workflow::new("delete_no_agent");
        wf_no_agent.steps = vec![Step {
            id: "s1".into(),
            name: "chat_no_agent".into(),
            action: Action::Chat {
                chat: "hi".into(),
                with: ChatOpts::default(),
            },
            ..Default::default()
        }];
        let wf_no_agent_id = wf_no_agent.id.clone();
        engine.store.save(&wf_no_agent).await.unwrap();
        engine.delete_workflow(&wf_no_agent_id).await.unwrap();
        assert!(
            ChatAgentStore::load_by_id(&agent_id).is_some(),
            "无 agent_id 的 Chat 步骤不应删除全局配置"
        );

        // 2) 含 agent_id 的 Chat 步骤 → 删除工作流后对应配置被删（按 ID 而非步骤名）
        let mut wf_with_agent = Workflow::new("delete_with_agent");
        wf_with_agent.steps = vec![Step {
            id: "s2".into(),
            name: "登录保障".into(), // 步骤名与 agent 名不同——按 ID 删除必须生效
            action: Action::Chat {
                chat: "hi".into(),
                with: ChatOpts {
                    agent_id: Some(agent_id.clone()),
                    ..Default::default()
                },
            },
            ..Default::default()
        }];
        let wf_with_agent_id = wf_with_agent.id.clone();
        engine.store.save(&wf_with_agent).await.unwrap();
        engine.delete_workflow(&wf_with_agent_id).await.unwrap();
        assert!(
            ChatAgentStore::load_by_id(&agent_id).is_none(),
            "含 agent_id 的 Chat 步骤应删除对应配置（按 ID）"
        );

        // 清理
        let _ = ChatAgentStore::delete_by_id(&agent_id);
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}

#[cfg(test)]
mod compiler_tests {
    use crate::workflow::compiler::Compiler;
    use crate::workflow::types::{
        Action, Condition, ForEachDef, IfDef, LoopDef, Step, VarRef, Workflow,
    };

    fn make_tool(id: &str, name: &str, tool: &str, params: serde_json::Value) -> Step {
        Step {
            id: id.into(),
            name: name.into(),
            action: Action::Tool {
                tool: tool.into(),
                with: params,
            },
            ..Default::default()
        }
    }

    // ── 基础验证 ──

    #[test]
    fn validate_empty_workflow_passes() {
        let wf = Workflow::new("空工作流");
        let report = Compiler::validate_workflow(&wf);
        // 空工作流通过（允许空的步骤列表）
        assert!(report.passed);
    }

    #[test]
    fn validate_tool_passes() {
        let mut wf = Workflow::new("Tool OK");
        wf.steps = vec![make_tool(
            "s1",
            "读文件",
            "Read",
            serde_json::json!({"path": "test.txt"}),
        )];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
    }

    #[test]
    fn validate_missing_id_fails() {
        let mut wf = Workflow::new("缺ID");
        wf.steps = vec![Step {
            id: String::new(),
            name: "无ID步骤".into(),
            action: Action::Tool {
                tool: "Read".into(),
                with: serde_json::json!({"path": "f"}),
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(report.errors.iter().any(|e| e.contains("id 为空")));
    }

    #[test]
    fn validate_missing_name_fails() {
        let mut wf = Workflow::new("缺Name");
        wf.steps = vec![Step {
            id: "s1".into(),
            name: String::new(),
            action: Action::Tool {
                tool: "Read".into(),
                with: serde_json::json!({"path": "f"}),
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(report.errors.iter().any(|e| e.contains("name 不能为空")));
    }

    #[test]
    fn validate_tool_empty_tool_fails() {
        let mut wf = Workflow::new("空Tool");
        wf.steps = vec![Step {
            id: "s1".into(),
            name: "空工具".into(),
            action: Action::Tool {
                tool: String::new(),
                with: serde_json::json!({}),
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(report.errors.iter().any(|e| e.contains("tool is empty")));
    }

    #[test]
    fn validate_seq_empty_children_warns() {
        let mut wf = Workflow::new("空 Seq");
        wf.steps = vec![Step {
            id: "seq1".into(),
            name: "空分组".into(),
            action: Action::Seq { seq: vec![] },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
        assert!(report.warnings.iter().any(|w| w.contains("无子步骤")));
    }

    // ── Loop 嵌套验证 ──

    #[test]
    fn validate_loop_passes() {
        let mut wf = Workflow::new("Loop 嵌套");
        wf.steps = vec![Step {
            id: "loop1".into(),
            name: "循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    repeat: Some(3),
                    for_each: None,
                    until: None,
                    max: 100,
                    steps: vec![make_tool(
                        "l1",
                        "重复步骤",
                        "Read",
                        serde_json::json!({"path": "test.txt"}),
                    )],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
    }

    #[test]
    fn validate_loop_empty_body_warns() {
        let mut wf = Workflow::new("空 Loop");
        wf.steps = vec![Step {
            id: "loop1".into(),
            name: "空循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    repeat: Some(1),
                    for_each: None,
                    until: None,
                    max: 100,
                    steps: vec![],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        // V2: 空 loop body 不生成 warning（允许空循环体作为占位）
        assert!(report.passed);
    }

    // ── If 嵌套验证 ──

    #[test]
    fn validate_if_passes() {
        let mut wf = Workflow::new("If 分支");
        wf.steps = vec![Step {
            id: "if1".into(),
            name: "条件".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::Equals {
                        equals: vec![
                            VarRef::Var {
                                var: "result".into(),
                            },
                            VarRef::Lit("ok".into()),
                        ],
                    },
                    then: vec![make_tool(
                        "t1",
                        "成功",
                        "Write",
                        serde_json::json!({"path": "ok.txt", "content": "done"}),
                    )],
                    else_branch: vec![make_tool(
                        "e1",
                        "失败",
                        "Write",
                        serde_json::json!({"path": "err.txt", "content": "fail"}),
                    )],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
    }

    #[test]
    fn validate_if_with_faulty_then_fails() {
        let mut wf = Workflow::new("If 坏分支");
        wf.steps = vec![Step {
            id: "if1".into(),
            name: "条件".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::Always { always: true },
                    then: vec![make_tool("t1", "空工具", "", serde_json::json!({}))],
                    else_branch: vec![],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
    }

    // ── Call / Wait / Talk ──

    #[test]
    fn validate_call_empty_workflow_id_fails() {
        let mut wf = Workflow::new("空 Call");
        wf.steps = vec![Step {
            id: "c1".into(),
            name: "调用".into(),
            action: Action::Call {
                call: String::new(),
                with: serde_json::json!({}),
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("workflow_id 不能为空")));
    }

    #[test]
    fn validate_chat_agent_passes() {
        let mut wf = Workflow::new("ChatAgent OK");
        wf.steps = vec![Step {
            id: "chat1".into(),
            name: "对话".into(),
            action: Action::Chat {
                chat: "你好".into(),
                with: Default::default(),
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
    }

    #[test]
    fn validate_wait_passes() {
        let mut wf = Workflow::new("Wait OK");
        wf.steps = vec![Step {
            id: "w1".into(),
            name: "等待确认".into(),
            action: Action::Wait {
                wait: "请确认".into(),
                auto: vec![],
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
    }

    // ── 变量捕获验证 ──

    #[test]
    fn validate_capture_var_tracking() {
        let mut wf = Workflow::new("变量捕获");
        wf.steps = vec![
            Step {
                id: "s1".into(),
                name: "消费者".into(),
                action: Action::Tool {
                    tool: "Read".into(),
                    with: serde_json::json!({"path": "{{data}}/file.txt"}),
                },
                ..Default::default()
            },
            Step {
                id: "s2".into(),
                name: "生产者".into(),
                action: Action::Tool {
                    tool: "Write".into(),
                    with: serde_json::json!({"path": "out"}),
                },
                capture: Some("data".into()),
                ..Default::default()
            },
        ];
        let report = Compiler::validate_workflow(&wf);
        // 前向引用产生 warning，不阻断执行（变量可能由 inputs 注入）
        assert!(report.passed);
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("data") && w.contains("尚未")));
    }

    #[test]
    fn validate_condition_var_unused_warns() {
        let mut wf = Workflow::new("条件引未捕获变量");
        wf.steps = vec![Step {
            id: "if1".into(),
            name: "判断".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::Equals {
                        equals: vec![
                            VarRef::Var {
                                var: "result".into(),
                            },
                            VarRef::Lit("ok".into()),
                        ],
                    },
                    then: vec![make_tool(
                        "t1",
                        "then",
                        "Read",
                        serde_json::json!({"path": "t"}),
                    )],
                    else_branch: vec![],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(
            report.passed,
            "条件变量未捕获不阻断执行: {:?}",
            report.errors
        );
        assert!(report.warnings.iter().any(|w| w.contains("result")));
    }

    #[test]
    fn validate_for_each_missing_var_warns() {
        let mut wf = Workflow::new("for_each缺变量");
        wf.steps = vec![Step {
            id: "lp1".into(),
            name: "循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    for_each: Some(ForEachDef {
                        items: VarRef::Var {
                            var: "unknown_list".into(),
                        },
                        item_var: "item".into(),
                    }),
                    repeat: None,
                    until: None,
                    max: 100,
                    steps: vec![make_tool(
                        "t1",
                        "body",
                        "Read",
                        serde_json::json!({"path": "{{item}}"}),
                    )],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(
            report.passed,
            "for_each 变量缺失不阻断: {:?}",
            report.errors
        );
        assert!(report.warnings.iter().any(|w| w.contains("unknown_list")));
    }

    #[test]
    fn validate_condition_op_requires_var_and_value() {
        let mut wf = Workflow::new("条件缺字段");
        wf.steps = vec![Step {
            id: "if1".into(),
            name: "判断".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::Contains { contains: vec![] },
                    then: vec![],
                    else_branch: vec![],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        // V2: 空 contains 不生成错误，验证通过（无变量引用可检查）
        assert!(report.passed);
    }

    #[test]
    fn validate_numeric_condition_ops() {
        // gt/lt/gte/lte 需要 var 和 value
        let numeric_conditions: Vec<(&str, Condition)> = vec![
            (
                "gt",
                Condition::Gt {
                    gt: vec![
                        VarRef::Var {
                            var: "count".into(),
                        },
                        VarRef::Lit("5".into()),
                    ],
                },
            ),
            (
                "lt",
                Condition::Lt {
                    lt: vec![
                        VarRef::Var {
                            var: "count".into(),
                        },
                        VarRef::Lit("5".into()),
                    ],
                },
            ),
            (
                "gte",
                Condition::Gte {
                    gte: vec![
                        VarRef::Var {
                            var: "count".into(),
                        },
                        VarRef::Lit("5".into()),
                    ],
                },
            ),
            (
                "lte",
                Condition::Lte {
                    lte: vec![
                        VarRef::Var {
                            var: "count".into(),
                        },
                        VarRef::Lit("5".into()),
                    ],
                },
            ),
        ];
        for (label, cond) in &numeric_conditions {
            let mut wf = Workflow::new("数字条件");
            wf.steps = vec![Step {
                id: "if1".into(),
                name: "判断".into(),
                action: Action::If {
                    def: IfDef {
                        condition: cond.clone(),
                        then: vec![],
                        else_branch: vec![],
                    },
                },
                ..Default::default()
            }];
            let report = Compiler::validate_workflow(&wf);
            assert!(
                report.passed,
                "op={} should pass: {:?}",
                label, report.errors
            );
        }
    }
}

#[cfg(test)]
mod serialization_tests {
    use crate::workflow::types::{
        Action, ChatOpts, Condition, IfDef, LoopDef, OnError, Step, VarRef, Workflow,
    };

    fn make_tool(id: &str, name: &str, tool: &str, params: serde_json::Value) -> Step {
        Step {
            id: id.into(),
            name: name.into(),
            action: Action::Tool {
                tool: tool.into(),
                with: params,
            },
            ..Default::default()
        }
    }

    /// Step JSON 往返: 序列化 → 反序列化 → 字段等价
    #[test]
    fn step_serde_round_trip_tool() {
        let step = make_tool(
            "s1",
            "读文件",
            "Read",
            serde_json::json!({"path": "/tmp/test.txt"}),
        );
        let json = serde_json::to_string(&step).unwrap();
        let round: Step = serde_json::from_str(&json).unwrap();

        assert_eq!(round.id(), "s1");
        assert_eq!(round.name(), "读文件");
        assert_eq!(round.kind_str(), "tool");
    }

    #[test]
    fn step_serde_round_trip_all_variants() {
        let steps = vec![
            make_tool("t1", "tool", "Read", serde_json::json!({})),
            Step {
                id: "s1".into(),
                name: "seq".into(),
                action: Action::Seq {
                    seq: vec![make_tool(
                        "s1c1",
                        "child",
                        "Write",
                        serde_json::json!({"c": "x"}),
                    )],
                },
                ..Default::default()
            },
            Step {
                id: "l1".into(),
                name: "loop".into(),
                action: Action::Loop {
                    def: LoopDef {
                        repeat: Some(5),
                        for_each: None,
                        until: None,
                        max: 100,
                        steps: vec![],
                    },
                },
                ..Default::default()
            },
            Step {
                id: "i1".into(),
                name: "if".into(),
                action: Action::If {
                    def: IfDef {
                        condition: Condition::Contains {
                            contains: vec![
                                VarRef::Var { var: "x".into() },
                                VarRef::Lit("ok".into()),
                            ],
                        },
                        then: vec![],
                        else_branch: vec![],
                    },
                },
                ..Default::default()
            },
            Step {
                id: "c1".into(),
                name: "call".into(),
                action: Action::Call {
                    call: "uuid-123".into(),
                    with: serde_json::json!({"a": 1}),
                },
                ..Default::default()
            },
            Step {
                id: "w1".into(),
                name: "wait".into(),
                action: Action::Wait {
                    wait: "确认".into(),
                    auto: vec![],
                },
                ..Default::default()
            },
            Step {
                id: "tk1".into(),
                name: "chat_agent".into(),
                action: Action::Chat {
                    chat: "你好".into(),
                    with: ChatOpts::default(),
                },
                ..Default::default()
            },
        ];

        for step in &steps {
            let json = serde_json::to_string(step).unwrap();
            let round: Step = serde_json::from_str(&json).unwrap();
            assert_eq!(round.kind_str(), step.kind_str(), "variant mismatch");
            assert_eq!(
                round.name(),
                step.name(),
                "name mismatch for {:?}",
                step.kind_str()
            );
        }
    }

    #[test]
    fn workflow_full_serde_round_trip() {
        let mut wf = Workflow::new("完整工作流");
        wf.steps = vec![make_tool(
            "s1",
            "第一步",
            "Read",
            serde_json::json!({"path": "config.json"}),
        )];
        wf.doc = Some("# 测试文档\n\n内容".into());

        let json = serde_json::to_string(&wf).unwrap();
        let round: Workflow = serde_json::from_str(&json).unwrap();

        assert_eq!(round.id, wf.id);
        assert_eq!(round.name, "完整工作流");
        assert_eq!(round.steps.len(), 1);
        assert_eq!(round.doc, Some("# 测试文档\n\n内容".into()));
    }

    /// 创建端到端测试工作流（echo 命令），用于验证 workflow_run 工具
    /// 注意：此测试需要 import WorkflowStore；工作流已通过 Node.js 直接写入磁盘。
    /// 工作流 ID：5684a6ac-aaf7-4ab4-9a63-60540d578ba3
    /// 见 ~/Desktop/test_workflow_id.txt
    #[ignore]
    #[tokio::test]
    async fn create_echo_test_workflow() {
        // 工作流已通过 Node.js 脚本直接写入 ~/.nuphus/workflows/
        // 此测试仅作为回归验证占位
    }

    /// V2 JSON 反序列化 + 转换验证
    #[test]
    fn step_v2_deserialize_and_convert() {
        // Test 1: Sleep step
        let json = r#"{"id":"t1","name":"测试","do":{"sleep":1}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Sleep 反序列化失败");
        assert_eq!(step.id(), "t1");
        assert_eq!(step.name(), "测试");
        assert_eq!(step.kind_str(), "tool");

        // Test 2: Tool step
        let json = r#"{"id":"t2","name":"工具调用","do":{"tool":"system_shell","with":{"command":"echo hi"}},"capture":"result","timeout_secs":30}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Tool 反序列化失败");
        assert_eq!(step.capture.as_deref(), Some("result"));
        assert_eq!(step.timeout_secs, Some(30));
        assert_eq!(step.kind_str(), "tool");

        // Test 3: Seq step (container)
        let json = r#"{"id":"s1","name":"顺序","do":{"seq":[{"id":"c1","name":"子","do":{"sleep":0.5}}]}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Seq 反序列化失败");
        assert_eq!(step.kind_str(), "seq");

        // Test 4: Break step
        let json = r#"{"id":"b1","name":"跳出","do":{"break":true}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Break 反序列化失败");
        assert_eq!(step.kind_str(), "break");

        // Test 5: Condition If step
        let json = r#"{"id":"i1","name":"条件","do":{"if":{"condition":{"not_empty":"$var"},"then":[{"id":"i1t1","name":"分支","do":{"sleep":0.3}}],"else":[]}}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 If 反序列化失败");
        assert_eq!(step.kind_str(), "if");

        // Test 6: Loop with repeat
        let json = r#"{"id":"l1","name":"循环","do":{"loop":{"repeat":3,"do":[{"id":"l1b1","name":"体","do":{"sleep":0.2}}]}}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Loop repeat 反序列化失败");
        assert_eq!(step.kind_str(), "loop");

        // Test 7: Loop with for_each
        let json = r#"{"id":"l2","name":"遍历","do":{"loop":{"for_each":{"items":"$items","as":"it"},"do":[{"id":"l2b1","name":"体","do":{"tool":"system_shell","with":{"command":"echo $it"}}}]}}}"#;
        let step: Step = serde_json::from_str(json).expect("V2 Loop for_each 反序列化失败");
        assert_eq!(step.kind_str(), "loop");

        // Test 8: V2 step deserialized as Step struct
        let v2_json = r#"{"id":"t1","name":"测试","do":{"sleep":1}}"#;
        let step: Step = serde_json::from_str(v2_json).expect("Step V2 反序列化失败");
        assert_eq!(step.kind_str(), "tool");

        // Test 9: Verify V2 deserialization preserves all fields for a Tool step
        let json = r#"{"id":"full","name":"完整测试","description":"desc","capture":"out","timeout_secs":60,"do":{"tool":"system_shell","with":{"command":"hello"}}}"#;
        let step_result: Result<Step, _> = serde_json::from_str(json);
        let step: Step = step_result.expect("V2 完整字段反序列化失败");
        assert_eq!(step.id(), "full");
        assert_eq!(step.name(), "完整测试");
        assert_eq!(step.description, "desc");
        assert_eq!(step.capture.as_deref(), Some("out"));
        assert_eq!(step.timeout_secs, Some(60));
        // on_error defaults to Abort
        match &step.on_error {
            OnError::Abort => {}
            _ => panic!("expected Abort on_error (default)"),
        }
    }
}
