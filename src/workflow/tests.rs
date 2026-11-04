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
    use crate::workflow::store::WorkflowStore;
    use crate::workflow::types::{
        Action, Condition, ForEachDef, IfDef, InputKind, InputSpec, LoopDef, ScriptDef, Step,
        VarRef, Workflow,
    };

    fn make_input(name: &str, required: bool, default: Option<serde_json::Value>) -> InputSpec {
        InputSpec {
            name: name.into(),
            kind: InputKind::String,
            required,
            default,
            description: None,
            sensitive: false,
            options: None,
        }
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

    #[tokio::test]
    async fn validate_calls_checks_target_shape_and_required_mappings() {
        let root = std::env::temp_dir().join(format!(
            "nuphus_validate_calls_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = WorkflowStore::with_root(root.clone());
        let mut child = Workflow::new("child");
        child.inputs = vec![make_input("topic", true, None)];
        child.steps = vec![make_tool(
            "child-step",
            "child",
            "Read",
            serde_json::json!({}),
        )];
        store.save(&child).await.unwrap();

        let mut parent = Workflow::new("parent");
        parent.steps = vec![Step {
            id: "call-child".into(),
            name: "call child".into(),
            action: Action::Call {
                call: child.id.clone(),
                with: serde_json::json!({"inputs": []}),
            },
            ..Default::default()
        }];
        let errors = Compiler::validate_calls(&parent, &store).await;
        assert!(
            errors
                .iter()
                .any(|error| error.contains("with.inputs 必须是对象")),
            "{errors:?}"
        );

        if let Action::Call { with, .. } = &mut parent.steps[0].action {
            *with = serde_json::json!({"inputs": {}});
        }
        let errors = Compiler::validate_calls(&parent, &store).await;
        assert!(
            errors
                .iter()
                .any(|error| error.contains("缺少必填输入映射 'topic'")),
            "{errors:?}"
        );

        if let Action::Call { with, .. } = &mut parent.steps[0].action {
            *with = serde_json::json!({"inputs": {"topic": "ok"}, "outputs": []});
        }
        let errors = Compiler::validate_calls(&parent, &store).await;
        assert!(
            errors
                .iter()
                .any(|error| error.contains("with.outputs 必须是对象")),
            "{errors:?}"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn validate_calls_rejects_missing_target_and_literal_type_error() {
        let root = std::env::temp_dir().join(format!(
            "nuphus_validate_call_types_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = WorkflowStore::with_root(root.clone());
        let mut child = Workflow::new("child");
        child.inputs = vec![InputSpec {
            name: "count".into(),
            kind: InputKind::Number,
            required: true,
            default: None,
            description: None,
            sensitive: false,
            options: None,
        }];
        store.save(&child).await.unwrap();

        let mut parent = Workflow::new("parent");
        parent.steps = vec![
            Step {
                id: "bad-type".into(),
                name: "bad type".into(),
                action: Action::Call {
                    call: child.id.clone(),
                    with: serde_json::json!({"inputs": {"count": "seven"}}),
                },
                ..Default::default()
            },
            Step {
                id: "missing".into(),
                name: "missing target".into(),
                action: Action::Call {
                    call: "does-not-exist".into(),
                    with: serde_json::json!({}),
                },
                ..Default::default()
            },
        ];
        let errors = Compiler::validate_calls(&parent, &store).await;
        assert!(
            errors.iter().any(|error| error.contains("类型不符合")),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|error| error.contains("不存在")),
            "{errors:?}"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
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

    // ── B15：repeat 编译期上限钳制 ──

    /// B15 回归：repeat 超过上限（此前 u32 全域可过 schema 与反序列化，无 tick
    /// 预算即静默空转）必须在编译期拒绝，错误信息含上限值与实际值。
    #[test]
    fn validate_loop_repeat_over_limit_fails() {
        let mut wf = Workflow::new("repeat 超限");
        wf.steps = vec![Step {
            id: "loop1".into(),
            name: "巨额循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    repeat: Some(4_000_000_000), // 40 亿：报告论断的原始场景
                    for_each: None,
                    until: None,
                    max: 100,
                    steps: vec![make_tool("l1", "体", "Read", serde_json::json!({}))],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(
            !report.passed,
            "repeat 超限必须编译期拒绝: {:?}",
            report.errors
        );
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("repeat") && e.contains("上限")),
            "错误信息应说明 repeat 超限: {:?}",
            report.errors
        );
    }

    /// B15 回归：上限内的合法 repeat 不受影响（钳制不得误伤正常批量场景）。
    #[test]
    fn validate_loop_repeat_within_limit_passes() {
        let mut wf = Workflow::new("repeat 合法");
        wf.steps = vec![Step {
            id: "loop1".into(),
            name: "批量循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    repeat: Some(100_000), // 恰好等于上限：合法
                    for_each: None,
                    until: None,
                    max: 100,
                    steps: vec![make_tool("l1", "体", "Read", serde_json::json!({}))],
                },
            },
            ..Default::default()
        }];
        let report = Compiler::validate_workflow(&wf);
        assert!(
            report.passed,
            "上限内的 repeat 必须放行: {:?}",
            report.errors
        );
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
    fn validate_script_capture_is_available_to_following_steps() {
        let mut wf = Workflow::new("脚本捕获");
        wf.steps = vec![
            Step {
                id: "timestamp".into(),
                name: "生成时间戳".into(),
                action: Action::Script {
                    script: ScriptDef {
                        runtime: "pwsh".into(),
                        code: "Get-Date".into(),
                        cwd: None,
                    },
                },
                capture: Some("ts".into()),
                ..Default::default()
            },
            Step {
                id: "append".into(),
                name: "追加记录".into(),
                action: Action::Tool {
                    tool: "Append".into(),
                    with: serde_json::json!({"content": "[{{ts}}] message"}),
                },
                ..Default::default()
            },
        ];

        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed);
        assert!(!report.warnings.iter().any(|warning| warning.contains("ts")));
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

    // ── 外部输入声明 ↔ 引用一致性 ──

    /// 引用 {{inputs.x}} 但未声明 → error（阻断执行）
    #[test]
    fn validate_undeclared_input_ref_fails() {
        let mut wf = Workflow::new("未声明输入");
        wf.steps = vec![make_tool(
            "s1",
            "写文件",
            "Write",
            serde_json::json!({"path": "{{inputs.missing}}"}),
        )];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed, "未声明引用应报 error");
        assert!(
            report
                .errors
                .iter()
                .any(|e| e == "未声明的输入引用 {{inputs.missing}}（请在 workflow.inputs 声明）"),
            "errors: {:?}",
            report.errors
        );
    }

    /// 声明后引用 → 通过，且不产生「未被引用」warning
    #[test]
    fn validate_declared_input_ref_passes() {
        let mut wf = Workflow::new("声明输入");
        wf.inputs = vec![make_input("topic", true, None)];
        wf.steps = vec![make_tool(
            "s1",
            "写文件",
            "Write",
            serde_json::json!({"path": "{{inputs.topic}}"}),
        )];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "errors: {:?}", report.errors);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.contains("已声明但未被任何步骤引用")),
            "warnings: {:?}",
            report.warnings
        );
    }

    /// 声明但全流程未被引用 → warning（不阻断执行）
    #[test]
    fn validate_declared_input_unused_warns() {
        let mut wf = Workflow::new("声明未用");
        wf.inputs = vec![make_input(
            "unused_one",
            false,
            Some(serde_json::json!("x")),
        )];
        wf.steps = vec![make_tool(
            "s1",
            "读文件",
            "Read",
            serde_json::json!({"path": "test.txt"}),
        )];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "errors: {:?}", report.errors);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w == "输入 unused_one 已声明但未被任何步骤引用"),
            "warnings: {:?}",
            report.warnings
        );
    }

    /// 嵌套子步骤（seq → loop.do → if.then → wait.auto）与管道写法的引用均被识别
    #[test]
    fn validate_nested_and_pipe_input_refs() {
        let nested = Step {
            id: "w1".into(),
            name: "等待".into(),
            action: Action::Wait {
                wait: "等待 {{inputs.c}}".into(),
                auto: vec![make_tool(
                    "w1c1",
                    "子工具",
                    "Read",
                    serde_json::json!({"path": "{{inputs.d}}"}),
                )],
            },
            ..Default::default()
        };
        let if_step = Step {
            id: "i1".into(),
            name: "条件".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::Always { always: true },
                    then: vec![nested],
                    else_branch: vec![],
                },
            },
            ..Default::default()
        };
        let loop_step = Step {
            id: "l1".into(),
            name: "循环".into(),
            action: Action::Loop {
                def: LoopDef {
                    for_each: None,
                    repeat: Some(1),
                    until: None,
                    max: 3,
                    steps: vec![if_step],
                },
            },
            ..Default::default()
        };
        let mut wf = Workflow::new("嵌套引用");
        wf.inputs = vec![
            make_input("a", false, None),
            make_input("b", false, None),
            make_input("c", false, None),
            make_input("d", false, None),
        ];
        wf.steps = vec![
            Step {
                id: "s1".into(),
                name: "顺序".into(),
                action: Action::Seq {
                    seq: vec![loop_step],
                },
                ..Default::default()
            },
            make_tool(
                "t1",
                "工具",
                "Read",
                serde_json::json!({"path": "a/{{inputs.a}}/{{inputs.b | default \"x\"}}"}),
            ),
        ];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "errors: {:?}", report.errors);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.contains("已声明但未被任何步骤引用")),
            "嵌套/管道引用应计入「已引用」: {:?}",
            report.warnings
        );
    }

    /// 条件 VarRef 引用输入：未声明 → error；声明 → 通过且计入「已引用」
    #[test]
    fn validate_condition_input_ref() {
        let mut wf = Workflow::new("条件输入");
        wf.steps = vec![Step {
            id: "i1".into(),
            name: "条件".into(),
            action: Action::If {
                def: IfDef {
                    condition: Condition::NotEmpty {
                        not_empty: VarRef::Var {
                            var: "inputs.flag".into(),
                        },
                    },
                    then: vec![],
                    else_branch: vec![],
                },
            },
            ..Default::default()
        }];

        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("未声明的输入引用 {{inputs.flag}}")),
            "errors: {:?}",
            report.errors
        );

        wf.inputs = vec![make_input("flag", false, None)];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "errors: {:?}", report.errors);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.contains("已声明但未被任何步骤引用")),
            "warnings: {:?}",
            report.warnings
        );
    }

    /// 既有校验规则不回归：新增 inputs 规则不覆盖/不干扰其它 error 判定
    #[test]
    fn validate_existing_rules_unaffected_by_inputs() {
        let mut wf = Workflow::new("回归");
        wf.inputs = vec![make_input("topic", false, None)];
        wf.steps = vec![
            make_tool(
                "s1",
                "工具",
                "Read",
                serde_json::json!({"path": "{{inputs.topic}}"}),
            ),
            Step {
                id: String::new(),
                name: "无ID".into(),
                action: Action::Break { _break: true },
                ..Default::default()
            },
        ];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(report.errors.iter().any(|e| e.contains("id 为空")));
        assert!(report
            .errors
            .iter()
            .any(|e| e.contains("break/continue 只能在 loop 内部使用")));
        assert!(
            !report.errors.iter().any(|e| e.contains("未声明的输入引用")),
            "errors: {:?}",
            report.errors
        );
    }
}

#[cfg(test)]
mod serialization_tests {
    use crate::workflow::types::{
        Action, ChatOpts, Condition, IfDef, InputKind, InputSpec, LoopDef, OnError, Step, VarRef,
        Workflow,
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

    // ── 外部输入声明（workflow.inputs）──

    /// 旧 JSON（无 inputs 字段）反序列化兼容：inputs 缺省为空 vec
    #[test]
    fn workflow_without_inputs_field_deserializes_empty() {
        let json = serde_json::json!({
            "id": "w1",
            "name": "旧工作流",
            "status": "Draft",
            "steps": [],
            "doc": null,
            "schedule": null
        });
        let wf: Workflow = serde_json::from_value(json).expect("旧 JSON 反序列化失败");
        assert_eq!(wf.name, "旧工作流");
        assert!(wf.inputs.is_empty(), "缺省应为空 vec: {:?}", wf.inputs);
    }

    /// 空 inputs 不序列化：旧文件形状保持不变（不新增字段噪声）
    #[test]
    fn workflow_empty_inputs_not_serialized() {
        let wf = Workflow::new("无输入");
        let json = serde_json::to_string(&wf).unwrap();
        assert!(
            !json.contains("\"inputs\""),
            "空 inputs 不应被序列化: {json}"
        );
    }

    /// InputSpec 往返：`type` 为 serde 别名（Rust 侧字段名 kind）
    #[test]
    fn input_spec_serde_round_trip() {
        let spec = InputSpec {
            name: "token".into(),
            kind: InputKind::Path,
            required: true,
            default: Some(serde_json::json!("D:/tmp")),
            description: Some("输入目录".into()),
            sensitive: true,
            options: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains(r#""type":"path""#), "type 别名未生效: {json}");

        let round: InputSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(round.name, "token");
        assert_eq!(round.kind, InputKind::Path);
        assert!(round.required);
        assert_eq!(round.default, Some(serde_json::json!("D:/tmp")));
        assert_eq!(round.description.as_deref(), Some("输入目录"));
        assert!(round.sensitive);
    }

    /// enum 类型序列化与 options 携带：旧数据（无 options）反序列化不受影响
    #[test]
    fn input_spec_enum_round_trip() {
        let spec = InputSpec {
            name: "mode".into(),
            kind: InputKind::Enum,
            required: false,
            default: Some(serde_json::json!("普通")),
            description: None,
            sensitive: false,
            options: Some(vec!["普通".into(), "后台".into()]),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains(r#""type":"enum""#), "enum 别名未生效: {json}");
        assert!(json.contains(r#""options""#), "options 未序列化: {json}");

        let round: InputSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(round.kind, InputKind::Enum);
        assert_eq!(
            round.options.as_deref(),
            Some(&["普通".to_string(), "后台".to_string()][..])
        );

        // 旧数据：无 options 字段 → None，非 enum 类型零影响
        let legacy: InputSpec =
            serde_json::from_str(r#"{"name":"topic","type":"string"}"#).unwrap();
        assert_eq!(legacy.kind, InputKind::String);
        assert!(legacy.options.is_none());
    }

    /// 声明项省略可选字段时取默认：type=string / required=false / sensitive=false
    #[test]
    fn input_spec_defaults() {
        let spec: InputSpec = serde_json::from_str(r#"{"name":"topic"}"#).unwrap();
        assert_eq!(spec.kind, InputKind::String);
        assert!(!spec.required);
        assert!(spec.default.is_none());
        assert!(spec.description.is_none());
        assert!(!spec.sensitive);
    }
}

// ── 调度域回归（批次四：B16/B17/B18/B10）──

#[cfg(test)]
mod scheduler_domain_tests {
    use crate::workflow::scheduler::{
        human_blocker_in_step, human_blocker_in_steps, SchedulerEngine,
    };
    use crate::workflow::store::WorkflowStore;
    use crate::workflow::types::{Action, ChatOpts, Condition, IfDef, Step, Workflow};

    fn config(cron: &str) -> crate::workflow::types::ScheduleConfig {
        crate::workflow::types::ScheduleConfig {
            cron: cron.into(),
            timezone: "UTC".into(),
            enabled: true,
            label: None,
            interval_minutes: None,
        }
    }

    fn chat_step(name: &str) -> Step {
        Step {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            action: Action::Chat {
                chat: "做点什么".into(),
                with: ChatOpts::default(),
            },
            ..Default::default()
        }
    }

    fn call_step(name: &str, target: &str) -> Step {
        Step {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            action: Action::Call {
                call: target.into(),
                with: serde_json::json!({}),
            },
            ..Default::default()
        }
    }

    fn tool_step(tool: &str) -> Step {
        Step {
            id: uuid::Uuid::new_v4().to_string(),
            name: tool.into(),
            action: Action::Tool {
                tool: tool.into(),
                with: serde_json::json!({}),
            },
            ..Default::default()
        }
    }

    /// `prompt` 即 wait 步骤的 `wait` 字段：空 = 不暂停，非空 = 暂停等人点继续。
    /// auto 里放一个界面步骤：非空 prompt 时报告必须是 wait 自身（说明先判自身再下钻），
    /// 空 prompt 时因为 auto 不含阻塞点而放行。
    fn wait_step(prompt: &str) -> Step {
        Step {
            id: uuid::Uuid::new_v4().to_string(),
            name: prompt.into(),
            action: Action::Wait {
                wait: prompt.into(),
                auto: vec![tool_step("browser_navigate")],
            },
            ..Default::default()
        }
    }

    /// B16：chat 步骤的 Agent 白名单含 request_user_input 等会等人的工具，一律按需人在场——
    /// 直接步骤与 Seq/Loop/If/Wait 容器内嵌套均要被判为阻塞点。
    #[test]
    fn chat_step_blocks_everywhere() {
        // 直接 chat
        let wf = Workflow {
            steps: vec![chat_step("chat1")],
            ..Workflow::new("b16_chat_direct")
        };
        assert!(
            human_blocker_in_steps(&wf.steps).is_some(),
            "顶层 chat 应判为阻塞点"
        );

        // Seq 内嵌套
        let wf = Workflow {
            steps: vec![Step {
                id: uuid::Uuid::new_v4().to_string(),
                name: "seq".into(),
                action: Action::Seq {
                    seq: vec![chat_step("chat_nested")],
                },
                ..Default::default()
            }],
            ..Workflow::new("b16_chat_in_seq")
        };
        assert!(
            human_blocker_in_steps(&wf.steps).is_some(),
            "Seq 内 chat 应判为阻塞点"
        );

        // If 分支内
        let wf = Workflow {
            steps: vec![Step {
                id: uuid::Uuid::new_v4().to_string(),
                name: "if".into(),
                action: Action::If {
                    def: IfDef {
                        condition: Condition::Always { always: true },
                        then: vec![chat_step("chat_in_then")],
                        else_branch: vec![],
                    },
                },
                ..Default::default()
            }],
            ..Workflow::new("b16_chat_in_if")
        };
        assert!(
            human_blocker_in_steps(&wf.steps).is_some(),
            "If 内 chat 应判为阻塞点"
        );
    }

    /// 判据换核：界面自动化步骤（browser_/desktop_）**不再**是不合格理由。
    ///
    /// 回归的是 issue #106：读取邮件 → 企业微信文档填表 → 提交这种全流程无人值守的
    /// 自动化，此前只因出现 `browser_navigate` 就无法建定时。选择权在"人 + Agent 一同
    /// 编译工作流"时已经行使，定时只是执行那份已决定的人工件。
    #[test]
    fn gui_automation_steps_do_not_block_scheduling() {
        for tool in [
            "browser_navigate",
            "browser_exec",
            "browser_click",
            "browser_type",
            "desktop_mouse",
            "desktop_find_color",
            "desktop_window_activate",
            "Read",
            "web_extract",
            "system_shell",
        ] {
            let wf = Workflow {
                steps: vec![tool_step(tool)],
                ..Workflow::new("gui_not_blocking")
            };
            assert_eq!(
                human_blocker_in_steps(&wf.steps),
                None,
                "{tool} 只是界面自动化，不该拦住定时"
            );
        }
    }

    /// 真正会让人无人值守跑不下去的三种构造：request_user_input、非空 wait、审批门。
    #[test]
    fn human_blocking_constructs_are_rejected_with_reasons() {
        let user_input = human_blocker_in_step(&tool_step("request_user_input"))
            .expect("request_user_input 必须判为阻塞点");
        assert!(user_input.contains("request_user_input"), "{user_input}");

        let approval = human_blocker_in_step(&tool_step("desktop_semantic_execute"))
            .expect("desktop_semantic_execute 的审批门必须判为阻塞点");
        assert!(approval.contains("desktop_semantic_execute"), "{approval}");

        let wait =
            human_blocker_in_step(&wait_step("等用户确认")).expect("非空 wait 必须判为阻塞点");
        assert!(wait.contains("wait"), "{wait}");
        assert!(
            !wait.contains("browser_navigate"),
            "报告必须指向 wait 自身: {wait}"
        );

        // 空 wait = 不暂停（step_wait 的既有语义）：不是阻塞点
        assert_eq!(
            human_blocker_in_step(&wait_step("")),
            None,
            "空 prompt 的 wait 步骤不会暂停，不该拦住定时"
        );
        // 纯空白的 prompt 同样不暂停
        assert_eq!(human_blocker_in_step(&wait_step("   \n ")), None);
    }

    /// B16：Call 指向含 chat 步骤的子工作流必须被拦（异步递归解析）；
    /// 纯工具子工作流不误伤——包括只含 browser_/desktop_ 的那种。
    #[tokio::test]
    async fn call_into_blocking_subworkflow_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("nuphus_b16_call_{}", uuid::Uuid::new_v4().simple()));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));

        // 子工作流 A：含 chat → Call 它必须被拒
        let mut frontend_sub = Workflow::new("sub_frontend");
        frontend_sub.steps = vec![chat_step("sub_chat")];
        store.save(&frontend_sub).await.unwrap();

        // 子工作流 B：纯工具 → Call 它放行
        let mut pure_sub = Workflow::new("sub_pure");
        pure_sub.steps = vec![Step {
            id: uuid::Uuid::new_v4().to_string(),
            name: "t".into(),
            action: Action::Tool {
                tool: "mock_echo".into(),
                with: serde_json::json!({}),
            },
            ..Default::default()
        }];
        store.save(&pure_sub).await.unwrap();

        // 调用方 C：Call A
        let mut caller = Workflow::new("caller");
        caller.steps = vec![call_step("c1", &frontend_sub.id)];
        store.save(&caller).await.unwrap();
        let schedule_result = scheduler
            .set_schedule(
                &caller.id,
                config("* * * * *"),
                std::collections::HashMap::new(),
                &store,
                |_| async {},
            )
            .await;
        let err = match schedule_result {
            Ok(()) => panic!("Call 指向 chat 子工作流的调度必须被拒"),
            Err(error) => error,
        };
        assert!(
            err.to_string().contains("无法定时执行"),
            "应报人类阻塞点错误: {err}"
        );

        // 调用方 D：Call B（纯工具）→ 放行
        let mut caller_pure = Workflow::new("caller_pure");
        caller_pure.steps = vec![call_step("d1", &pure_sub.id)];
        store.save(&caller_pure).await.unwrap();
        scheduler
            .set_schedule(
                &caller_pure.id,
                config("* * * * *"),
                std::collections::HashMap::new(),
                &store,
                |_| async {},
            )
            .await
            .expect("Call 纯工具子工作流不应被拦");

        scheduler.remove_schedule(&caller_pure.id).await;
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// issue #106 的正面场景：全流程都是界面自动化的工作流必须能建定时。
    /// 读邮件 → 企业微信文档填表 → 提交 → 校验，没有任何需要人判断的节点。
    #[tokio::test]
    async fn gui_only_workflow_can_be_scheduled() {
        let root =
            std::env::temp_dir().join(format!("nuphus_106_gui_{}", uuid::Uuid::new_v4().simple()));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));

        let mut gui = Workflow::new("daily_report");
        gui.steps = vec![
            tool_step("Read"),
            tool_step("browser_navigate"),
            tool_step("browser_exec"),
            tool_step("browser_click"),
            tool_step("browser_type"),
            tool_step("browser_extract"),
        ];
        store.save(&gui).await.unwrap();

        scheduler
            .set_schedule(
                &gui.id,
                config("0 9 * * 1-5"),
                std::collections::HashMap::new(),
                &store,
                |_| async {},
            )
            .await
            .expect("纯界面自动化工作流必须允许定时（issue #106）");

        scheduler.remove_schedule(&gui.id).await;
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// 保存期判过不等于触发时成立：Call 的目标当时缺失（放行）之后被补成含 chat 的
    /// 子工作流，运行期复检必须拦住——否则无人时白等到各自的超时。
    #[tokio::test]
    async fn schedule_survives_save_but_run_time_recheck_rejects() {
        let root =
            std::env::temp_dir().join(format!("nuphus_106_late_{}", uuid::Uuid::new_v4().simple()));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));

        // 目标先不落盘：此刻对 store 不可见
        let mut late = Workflow::new("sub_added_later");
        let late_id = late.id.clone();
        late.steps = vec![chat_step("late_chat")];

        let mut caller = Workflow::new("caller_late");
        caller.steps = vec![call_step("c1", &late_id)];
        store.save(&caller).await.unwrap();

        // 目标此刻不存在 → 保存期放行
        scheduler
            .set_schedule(
                &caller.id,
                config("* * * * *"),
                std::collections::HashMap::new(),
                &store,
                |_| async {},
            )
            .await
            .expect("Call 目标缺失时保存期放行");

        // 之后目标被补齐，且是会等人的那种
        store.save(&late).await.unwrap();

        let reason = crate::workflow::scheduler::human_blocker_with_calls(&caller.steps, &store, 0)
            .await
            .expect("补齐目标后运行期复检必须判出阻塞点");
        assert!(reason.contains("Chat"), "{reason}");

        scheduler.remove_schedule(&caller.id).await;
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// B18：工作流文件损坏 → 调度条目必须保留（persist 落盘不删）；
    /// 工作流目录不存在 → 孤儿调度照旧删除（既有行为保持）。
    #[tokio::test]
    async fn restore_keeps_schedule_when_workflow_file_corrupted() {
        let root = std::env::temp_dir().join(format!(
            "nuphus_b18_restore_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));

        // wf_broken：先正常保存再写坏 workflow.json（目录仍在）
        let mut broken = Workflow::new("broken");
        broken.steps = vec![];
        store.save(&broken).await.unwrap();
        let broken_dir = root.join("workflows").join(&broken.id);
        std::fs::write(broken_dir.join("workflow.json"), "{ not valid json").unwrap();

        // wf_missing：注册过调度但目录被整删（孤儿）
        let missing = Workflow::new("missing");
        store.save(&missing).await.unwrap();
        let missing_id = missing.id.clone();
        tokio::fs::remove_dir_all(root.join("workflows").join(&missing_id))
            .await
            .unwrap();

        // 直接写 schedules.json 模拟「上次运行的持久化状态」
        let persisted = format!(
            r#"{{"schedules":{{"{}":{{"cron":"* * * * *","timezone":"UTC","enabled":true}},"{}":{{"cron":"* * * * *","timezone":"UTC","enabled":true}}}}}}"#,
            broken.id, missing_id
        );
        std::fs::write(root.join("schedules.json"), persisted).unwrap();

        // 模拟 restore 主循环的核心判定（用干净 store 实例绕过 save 写入的缓存，
        // 强制读盘——与启动时 restore_schedules 的运行态一致）：
        // broken → get None + validation_load_error Some → 保留
        let cold_store = WorkflowStore::with_root(root.join("workflows"));
        assert!(
            cold_store.get(&broken.id).await.is_none(),
            "损坏的工作流必须读不出来（get → None）"
        );
        assert!(
            cold_store.validation_load_error(&broken.id).await.is_some(),
            "损坏文件必须被 validation_load_error 探测到（区分于不存在）"
        );
        // missing → get None + validation_load_error None → 删除
        assert!(cold_store.get(&missing_id).await.is_none());
        assert!(
            cold_store
                .validation_load_error(&missing_id)
                .await
                .is_none(),
            "目录不存在时 validation_load_error 必须返回 None（真孤儿）"
        );

        // persist_current_with_retained：restore 主循环只把 broken 放 retained，
        // 孤儿 missing 不放 → 落盘后 broken 在、missing 消失。
        let persisted_before = scheduler.load_current();
        assert_eq!(persisted_before.schedules.len(), 2);
        let mut retained = std::collections::HashMap::new();
        retained.insert(
            broken.id.clone(),
            persisted_before.schedules[&broken.id].clone(),
        );
        scheduler
            .persist_current_with_retained(&retained)
            .await
            .unwrap();
        let after = scheduler.load_current();
        assert!(
            after.schedules.contains_key(&broken.id),
            "损坏工作流的调度条目必须保留"
        );
        assert!(
            !after.schedules.contains_key(&missing_id),
            "孤儿调度（目录不存在）必须被物理删除（既有行为保持）"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// B10：并发 save 同一工作流不产生交错损坏——每次 save 的 tmp 名唯一，
    /// 最终 workflow.json 恰为某一次完整序列化。
    #[tokio::test]
    async fn concurrent_saves_produce_valid_workflow_json() {
        let root =
            std::env::temp_dir().join(format!("nuphus_b10_tmp_{}", uuid::Uuid::new_v4().simple()));
        let store = WorkflowStore::with_root(root.join("workflows"));

        let wf = Workflow::new("b10_concurrent");
        store.save(&wf).await.unwrap();

        // 20 个并发 save（节流保存 vs 暂停快照并发的放大模拟）
        let mut handles = Vec::new();
        for i in 0..20 {
            let store = WorkflowStore::with_root(root.join("workflows"));
            let mut wf = wf.clone();
            wf.name = format!("b10_concurrent_v{i}");
            handles.push(tokio::spawn(async move { store.save(&wf).await }));
        }
        for handle in handles {
            handle.await.unwrap().expect("并发 save 不应失败");
        }

        // 最终文件必须是合法 JSON（旧实现下并发交错可能写出半成品）
        let final_text =
            tokio::fs::read_to_string(root.join("workflows").join(&wf.id).join("workflow.json"))
                .await
                .unwrap();
        let parsed: Workflow = serde_json::from_str(&final_text)
            .expect("并发 save 后 workflow.json 必须是完整合法 JSON");
        assert!(parsed.name.starts_with("b10_concurrent"));

        // 目录中不应残留 tmp 文件
        let leftovers: Vec<_> = std::fs::read_dir(root.join("workflows").join(&wf.id))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "rename 成功后不得残留 tmp 文件: {:?}",
            leftovers.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// B17：tool_schemas=None 时 registry 兜底必须生效（定时源 chat 步骤拿工具），
    /// Some(vec![]) 会挡掉兜底（修复前的错误形态，测试固化其区别）。
    /// execute_workflow 的闸门/编译器全链路太重，这里直测 mod.rs:347 的
    /// 兜底表达式语义：None + registry 注入 → Some(非空)。
    #[tokio::test]
    async fn registry_fallback_applies_when_tool_schemas_none() {
        use crate::workflow::WorkflowEngine;

        let engine = WorkflowEngine::new();
        // 未注入 tools 时：None → None（无兜底，chat 步骤无工具——headless 预期）
        let schemas: Option<Vec<crate::api::ToolDefinition>> = None;
        let resolved = schemas
            .clone()
            .or_else(|| engine.tools().map(|t| t.get_schemas()));
        assert!(resolved.is_none(), "未注入 registry 时不得凭空产生 schemas");

        // 注入 registry 后：None → Some(非空)（B17 修复依赖的兜底路径）
        let registry = std::sync::Arc::new(crate::tools::ToolRegistry::new());
        let mut engine = engine;
        engine.set_tools(registry);
        let resolved = schemas.or_else(|| engine.tools().map(|t| t.get_schemas()));
        let schemas = resolved.expect("注入 registry 后 None 必须兜底为 Some");
        assert!(
            !schemas.is_empty(),
            "registry.get_schemas() 必须返回非空全量工具表"
        );
        // 对照：Some(vec![]) 挡掉兜底（B17 修复前的错误形态）
        let blocking: Option<Vec<crate::api::ToolDefinition>> = Some(vec![]);
        let resolved_blocked = blocking.or_else(|| engine.tools().map(|t| t.get_schemas()));
        assert!(
            resolved_blocked.map(|s| s.is_empty()).unwrap_or(false),
            "Some(vec![]) 必须挡掉 registry 兜底（这就是 B17 的 bug 形态）"
        );
    }
}

// ── Model 步骤（第 14 种 do：text / image / video） ──

#[cfg(test)]
mod model_step_tests {
    use crate::workflow::compiler::Compiler;
    use crate::workflow::types::{Action, ModelKind, ModelOpts, Step, Workflow};

    fn make_model_step(id: &str, prompt: &str, opts: ModelOpts) -> Step {
        Step {
            id: id.into(),
            name: format!("模型步骤-{id}"),
            action: Action::Model {
                model: prompt.to_string(),
                with: opts,
            },
            ..Default::default()
        }
    }

    /// 编译器接受三种 kind 的合法 model 步骤。
    #[test]
    fn compiler_accepts_model_steps_all_kinds() {
        for (id, kind) in [
            ("m-text", ModelKind::Text),
            ("m-image", ModelKind::Image),
            ("m-video", ModelKind::Video),
        ] {
            let mut wf = Workflow::new("model ok");
            wf.steps = vec![make_model_step(
                id,
                "生成一个产品图",
                ModelOpts {
                    kind,
                    ..Default::default()
                },
            )];
            let report = Compiler::validate_workflow(&wf);
            assert!(
                report.passed,
                "{kind:?} model step must pass validation: {:?}",
                report.errors
            );
        }
    }

    /// 缺省 with（无 kind）= text；缺省 with.model = 运行时 llm / 绑定能力。
    #[test]
    fn compiler_accepts_model_step_with_bare_with() {
        let mut wf = Workflow::new("model bare");
        wf.steps = vec![make_model_step("m1", "写一句文案", ModelOpts::default())];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "{:?}", report.errors);
        assert!(
            report.warnings.is_empty(),
            "裸 with 不应产生 warning: {:?}",
            report.warnings
        );
    }

    /// model 任务描述为空 → 编译期 error。
    #[test]
    fn compiler_rejects_empty_model_prompt() {
        let mut wf = Workflow::new("model empty");
        wf.steps = vec![make_model_step("m1", "", ModelOpts::default())];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed);
        assert!(
            report.errors.iter().any(|e| e.contains("model 不能为空")),
            "{:?}",
            report.errors
        );
    }

    /// 具名规格值非法（比例不在白名单 / 张数越界 / 时长非选项）→ 编译期 error：
    /// 生成调用计费，不能等 API 才报错。
    #[test]
    fn compiler_rejects_illegal_named_spec_values() {
        let mut wf = Workflow::new("model illegal spec");
        wf.steps = vec![
            make_model_step(
                "m-ratio",
                "画猫",
                ModelOpts {
                    kind: ModelKind::Image,
                    ratio: Some("1:2".to_string()),
                    ..Default::default()
                },
            ),
            make_model_step(
                "m-count",
                "画猫",
                ModelOpts {
                    kind: ModelKind::Image,
                    count: Some(5),
                    ..Default::default()
                },
            ),
            make_model_step(
                "m-duration",
                "拍猫",
                ModelOpts {
                    kind: ModelKind::Video,
                    duration: Some(7),
                    ..Default::default()
                },
            ),
        ];
        let report = Compiler::validate_workflow(&wf);
        assert!(!report.passed, "非法规格值必须阻断: {:?}", report.errors);
        assert!(
            report.errors.iter().any(|e| e.contains("画面比例 '1:2'")),
            "{:?}",
            report.errors
        );
        assert!(
            report.errors.iter().any(|e| e.contains("生成数量 5 越界")),
            "{:?}",
            report.errors
        );
        assert!(
            report.errors.iter().any(|e| e.contains("时长 7 秒")),
            "{:?}",
            report.errors
        );
    }

    /// 合法具名规格值通过编译，且不产生 warning（模态归属正确）。
    #[test]
    fn compiler_accepts_named_spec_values() {
        let mut wf = Workflow::new("model spec ok");
        wf.steps = vec![
            make_model_step(
                "m-image",
                "画猫",
                ModelOpts {
                    kind: ModelKind::Image,
                    ratio: Some("4:3".to_string()),
                    count: Some(2),
                    ..Default::default()
                },
            ),
            make_model_step(
                "m-video",
                "拍猫",
                ModelOpts {
                    kind: ModelKind::Video,
                    ratio: Some("16:9".to_string()),
                    duration: Some(6),
                    first_frame: Some("{{frame}}".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    /// 缺省值不回写：未设置的具名字段不序列化（不落进 workflow.json）。
    #[test]
    fn model_opts_skip_unset_named_fields_on_serialize() {
        let value = serde_json::to_value(ModelOpts {
            kind: ModelKind::Image,
            ..Default::default()
        })
        .expect("serialize");
        let object = value.as_object().expect("with object");
        for absent in ["ratio", "count", "duration", "first_frame", "params"] {
            assert!(
                !object.contains_key(absent),
                "未设置的 {absent} 不得落盘: {object:?}"
            );
        }
        assert_eq!(object.get("kind"), Some(&serde_json::json!("image")));

        let set = serde_json::to_value(ModelOpts {
            kind: ModelKind::Video,
            ratio: Some("9:16".to_string()),
            duration: Some(5),
            first_frame: Some(String::new()),
            ..Default::default()
        })
        .expect("serialize");
        assert_eq!(
            set,
            serde_json::json!({"kind":"video","ratio":"9:16","duration":5,"first_frame":""})
        );
    }

    /// 跨模态误用只 warning 不阻断：text 带 params、image 带 system。
    #[test]
    fn compiler_warns_on_modality_mismatch() {
        let mut wf = Workflow::new("model mismatch");
        wf.steps = vec![
            make_model_step(
                "m-text",
                "写文案",
                ModelOpts {
                    params: Some(serde_json::json!({"n": 1})),
                    ..Default::default()
                },
            ),
            make_model_step(
                "m-image",
                "画猫",
                ModelOpts {
                    kind: ModelKind::Image,
                    system: Some("你是画师".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let report = Compiler::validate_workflow(&wf);
        assert!(report.passed, "模态误用只警告不阻断: {:?}", report.errors);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("params") && w.contains("image/video")),
            "{:?}",
            report.warnings
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("system") && w.contains("text")),
            "{:?}",
            report.warnings
        );
    }

    /// kind 缺省 = text；显式 image/video 如实解析。
    #[test]
    fn model_kind_defaults_to_text_on_deserialize() {
        let action: Action =
            serde_json::from_value(serde_json::json!({"model": "写文案"})).expect("bare do.model");
        match action {
            Action::Model { model, with } => {
                assert_eq!(model, "写文案");
                assert_eq!(with.kind, ModelKind::Text, "kind 缺省必须是 text");
            }
            other => panic!("expected Action::Model, got {other:?}"),
        }

        let action: Action = serde_json::from_value(
            serde_json::json!({"model": "画一只猫", "with": {"kind": "image"}}),
        )
        .expect("image model step");
        match action {
            Action::Model { with, .. } => assert_eq!(with.kind, ModelKind::Image),
            other => panic!("expected Action::Model, got {other:?}"),
        }

        // 序列化形状（前端/存储契约）：do 下只有 model + with 两个键
        let value = serde_json::to_value(&Action::Model {
            model: "做视频".to_string(),
            with: ModelOpts {
                kind: ModelKind::Video,
                ..Default::default()
            },
        })
        .expect("serializable");
        let object = value.as_object().expect("do is an object");
        assert_eq!(object.len(), 2, "do 只允许 model + with: {object:?}");
        assert!(object.contains_key("model") && object.contains_key("with"));
    }

    /// 非法 kind / 缺 model 字段：不得被解析成合法 Model（untagged 回退 Custom，
    /// 执行期报 not yet supported——与其它 do 类型的畸形数据同一命运）。
    #[test]
    fn illegal_kind_or_missing_model_is_not_accepted_as_model() {
        let action: Action =
            serde_json::from_value(serde_json::json!({"model": "x", "with": {"kind": "audio"}}))
                .expect("untagged enum always yields some variant");
        assert!(
            !matches!(action, Action::Model { .. }),
            "非法 kind 不得解析为 Model: {action:?}"
        );

        let action: Action = serde_json::from_value(serde_json::json!({"with": {"kind": "text"}}))
            .expect("untagged enum always yields some variant");
        assert!(
            !matches!(action, Action::Model { .. }),
            "缺 model 字段不得解析为 Model: {action:?}"
        );
    }

    /// schema 漂移守卫：DoModel 已注册进 Action oneOf，且 DoCustom 的
    /// propertyNames 黑名单覆盖全部 14 个 do 名——漏一处 model 步骤就会被
    /// DoCustom 吞掉或直接被判未知属性。
    #[test]
    fn step_schema_registers_model_do_kind() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("workflow/step_schema.json");
        let text = std::fs::read_to_string(&path).expect("step_schema.json readable");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("valid json schema");
        let defs = &schema["$defs"];

        // DoModel 定义：required = ["model"]，kind enum 三态
        assert!(defs["DoModel"].is_object(), "$defs.DoModel must exist");
        assert_eq!(defs["DoModel"]["required"], serde_json::json!(["model"]));
        let kind_enum = defs["DoModel"]["properties"]["with"]["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind enum");
        assert_eq!(
            kind_enum,
            &vec![
                serde_json::json!("text"),
                serde_json::json!("image"),
                serde_json::json!("video")
            ]
        );

        // 具名规格字段：enum / 边界必须与 compiler.rs 的 MODEL_RATIOS /
        // MODEL_DURATIONS 白名单一致（前端选项控件同样只发这些值）
        let with_props = &defs["DoModel"]["properties"]["with"]["properties"];
        assert_eq!(
            with_props["ratio"]["enum"],
            serde_json::json!(["1:1", "4:3", "3:4", "3:2", "2:3", "16:9", "9:16"])
        );
        assert_eq!(
            with_props["count"],
            serde_json::json!({
                "type": "integer",
                "minimum": 1,
                "maximum": 4,
                "description": "image 一次生成张数"
            })
        );
        assert_eq!(
            with_props["duration"]["enum"],
            serde_json::json!([5, 6, 8, 10])
        );
        assert_eq!(
            with_props["first_frame"]["type"],
            serde_json::json!("string")
        );

        // Step/Action 的 oneOf 已注册 DoModel
        let one_of = defs["Action"]["oneOf"]
            .as_array()
            .expect("Action.oneOf array");
        assert!(
            one_of
                .iter()
                .any(|entry| entry["$ref"] == "#/$defs/DoModel"),
            "Action oneOf must register #/$defs/DoModel: {one_of:?}"
        );

        // DoCustom propertyNames 黑名单 = 全部 14 个 do 名（防止 model 被当自定义属性放行）
        let custom_names = defs["DoCustom"]["propertyNames"]["not"]["enum"]
            .as_array()
            .expect("DoCustom propertyNames enum");
        for name in [
            "tool", "seq", "loop", "if", "call", "wait", "chat", "model", "script", "assert",
            "mcp", "sleep", "break", "continue",
        ] {
            assert!(
                custom_names.iter().any(|value| value == name),
                "DoCustom propertyNames missing do kind '{name}': {custom_names:?}"
            );
        }
        assert_eq!(
            custom_names.len(),
            14,
            "propertyNames 黑名单必须恰好覆盖 14 种 do（新增类型必须同步此处）: {custom_names:?}"
        );
    }
}
