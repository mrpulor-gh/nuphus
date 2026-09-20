use super::execute::resolve_declared_inputs;
use super::*;
use crate::workflow::events::EventBus;
use crate::workflow::store::WorkflowStore;
use crate::workflow::types::Workflow;
use crate::workflow::types::{
    Action, ForEachDef, InputKind, InputSpec, LoopDef, RunStatus, Step, StepRunStatus, VarRef,
};
use std::result::Result as StdResult;

fn make_tool_step(name: &str, tool: &str, params: serde_json::Value) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Tool {
            tool: tool.to_string(),
            with: params,
        },
        ..Default::default()
    }
}

async fn ok_tool_exec(_tool: String, _params: serde_json::Value) -> StdResult<String, String> {
    Ok("ok".to_string())
}

async fn fail_tool_exec(_tool: String, _params: serde_json::Value) -> StdResult<String, String> {
    Err("simulated tool failure".to_string())
}

#[tokio::test]
async fn test_executor_linear_single_step() {
    let tmp = std::env::temp_dir().join("nuphus_test_exec");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("linear_test");
    wf.steps = vec![make_tool_step(
        "test_step",
        "mock_echo",
        serde_json::json!({"command": "echo success_output"}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_ok(), "Execution failed: {:?}", result.err());
}

#[tokio::test]
async fn test_compiler_rejects_empty_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_compiler");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let wf = Workflow::new("empty_wf");
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    // V2: 空工作流通过校验，执行成功（空步骤列表直接完成）
    assert!(
        result.is_ok(),
        "Empty workflow should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_executor_with_real_system_shell() {
    let tmp = std::env::temp_dir().join("nuphus_test_real");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("real_shell_test");
    wf.steps = vec![make_tool_step(
        "echo_step",
        "system_shell",
        serde_json::json!({"command": "echo NUphus_TEST_REAL_SHELL_SUCCESS"}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tools = std::sync::Arc::new(crate::tools::ToolRegistry::builtin());
    let tool_exec = {
        let tools = tools.clone();
        move |tool: String, params: serde_json::Value| {
            let tools = tools.clone();
            async move {
                let result = tools
                    .execute(&tool, &params)
                    .await
                    .map_err(|e| e.to_string())?;
                if result.success {
                    Ok(result.output.unwrap_or_default())
                } else {
                    Err(result.error.unwrap_or_else(|| "unknown error".to_string()))
                }
            }
        }
    };

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "Real system_shell execution failed: {:?}",
        result.err()
    );

    // Verify last_run recorded with valid UUID
    let wf_after = store.get(&wf_id).await.unwrap();
    assert!(wf_after.last_run().is_some(), "last_run should be recorded");
    let last_run = wf_after.last_run().unwrap();
    assert!(!last_run.run_id.is_empty(), "Run ID should not be empty");
    assert_eq!(
        last_run.status,
        crate::workflow::types::RunStatus::Success,
        "Run should succeed, status: {:?}",
        last_run.status
    );
}

// ── wf_call 模块化测试 ──

fn make_call_step(name: &str, workflow_id: &str) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Call {
            call: workflow_id.to_string(),
            with: serde_json::json!({"workflow_id": workflow_id}),
        },
        ..Default::default()
    }
}

fn make_seq_step(name: &str, steps: Vec<Step>) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Seq { seq: steps },
        ..Default::default()
    }
}

#[tokio::test]
async fn test_wf_call_single_level() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall1");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 子工作流: 一个 tool 步骤
    let mut sub = Workflow::new("sub_tool");
    let sub_id = sub.id.clone();
    sub.steps = vec![make_tool_step(
        "sub_echo",
        "mock_echo",
        serde_json::json!({"cmd": "hello"}),
    )];
    store.save(&sub).await.unwrap();

    // 父工作流: 调用子工作流
    let mut parent = Workflow::new("parent");
    parent.steps = vec![make_call_step("call_sub", &sub_id)];
    let parent_id = parent.id.clone();
    store.save(&parent).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &parent_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "wf_call single level failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_with_seq_nesting() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_seq");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // L2: 最深子工作流
    let mut l2 = Workflow::new("level2");
    let l2_id = l2.id.clone();
    l2.steps = vec![
        make_tool_step("l2_step1", "mock_echo", serde_json::json!({"msg": "a"})),
        make_tool_step("l2_step2", "mock_echo", serde_json::json!({"msg": "b"})),
    ];
    store.save(&l2).await.unwrap();

    // L1: 使用 Seq 包装对 L2 的调用
    let mut l1 = Workflow::new("level1");
    let l1_id = l1.id.clone();
    l1.steps = vec![make_seq_step(
        "wrapper",
        vec![
            make_tool_step("l1_step", "mock_echo", serde_json::json!({"msg": "x"})),
            make_call_step("invoke_l2", &l2_id),
        ],
    )];
    store.save(&l1).await.unwrap();

    // 顶层: L0 seq 中调用 L1
    let mut l0 = Workflow::new("level0");
    l0.steps = vec![make_seq_step(
        "top_seq",
        vec![make_call_step("invoke_l1", &l1_id)],
    )];
    let l0_id = l0.id.clone();
    store.save(&l0).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &l0_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "Nested wf_call with seq failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_deep_nesting() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_deep");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // L2: 最深子工作流
    let mut l2 = Workflow::new("level2");
    let l2_id = l2.id.clone();
    l2.steps = vec![make_tool_step(
        "deepest",
        "mock_echo",
        serde_json::json!({"msg": "deep"}),
    )];
    store.save(&l2).await.unwrap();

    // L1: 调用 L2
    let mut l1 = Workflow::new("level1");
    let l1_id = l1.id.clone();
    l1.steps = vec![make_call_step("call_l2", &l2_id)];
    store.save(&l1).await.unwrap();

    // 顶层: L0 调用 L1
    let mut l0 = Workflow::new("level0");
    l0.steps = vec![make_call_step("call_l1", &l1_id)];
    let l0_id = l0.id.clone();
    store.save(&l0).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &l0_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "Nested wf_call L0→L1→L2 failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_error_propagation() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_err");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 子工作流: 会失败的步骤
    let mut sub = Workflow::new("failing_sub");
    let sub_id = sub.id.clone();
    sub.steps = vec![make_tool_step(
        "will_fail",
        "bad_tool",
        serde_json::json!({}),
    )];
    store.save(&sub).await.unwrap();

    // 父工作流
    let mut parent = Workflow::new("parent");
    parent.steps = vec![make_call_step("call_failing", &sub_id)];
    let parent_id = parent.id.clone();
    store.save(&parent).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &parent_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_err(),
        "Error in sub-workflow should propagate to parent"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("failing_sub") || err_msg.contains("simulated tool failure"),
        "Error should mention sub-workflow or failure cause, got: {}",
        err_msg
    );
}

// ── P0 修复验证：StepRunRecord 管道 / 失败记录 push / for_each 点号路径 ──

#[tokio::test]
async fn test_run_record_steps_populated() {
    let tmp = std::env::temp_dir().join("nuphus_test_runrec");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("steps_recording");
    let s1 = make_tool_step("s1", "mock_echo", serde_json::json!({"a": 1}));
    let s2 = make_tool_step("s2", "mock_echo", serde_json::json!({"b": 2}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(result.is_ok(), "Execution failed: {:?}", result.err());

    let wf_after = store.get(&wf_id).await.unwrap();
    let last_run = wf_after.last_run().expect("last_run should exist");
    assert_eq!(
        last_run.status,
        RunStatus::Success,
        "status should be Success"
    );
    // 两个步骤都被记录（含 step_id / status）
    let ids: Vec<&str> = last_run.steps.iter().map(|s| s.step_id.as_str()).collect();
    assert!(
        ids.contains(&s1_id.as_str()),
        "s1 should be recorded, got {:?}",
        ids
    );
    assert!(
        ids.contains(&s2_id.as_str()),
        "s2 should be recorded, got {:?}",
        ids
    );
    assert!(
        last_run
            .steps
            .iter()
            .all(|s| s.status == StepRunStatus::Success),
        "all recorded steps should be Success, got {:?}",
        last_run.steps
    );
}

#[tokio::test]
async fn test_for_each_nested_path() {
    let tmp = std::env::temp_dir().join("nuphus_test_for_each_path");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let body = make_tool_step(
        "loop_body",
        "mock_echo",
        serde_json::json!({"v": "{{item}}"}),
    );
    let body_id = body.id.clone();
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "foreach_panels".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: Some(ForEachDef {
                    items: VarRef::Var {
                        var: "panels.list".into(),
                    },
                    item_var: "item".into(),
                }),
                repeat: None,
                until: None,
                max: 100,
                steps: vec![body],
            },
        },
        ..Default::default()
    };
    let loop_id = loop_step.id.clone();

    let mut wf = Workflow::new("for_each_nested");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let inputs: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::from([(
            "panels".to_string(),
            serde_json::json!({"list": [1, 2, 3]}),
        )]);

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            None,
            None,
            None,
            Some(inputs),
            false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "for_each with nested path failed: {:?}",
        result.err()
    );

    let wf_after = store.get(&wf_id).await.unwrap();
    let last_run = wf_after.last_run().expect("last_run should exist");
    assert_eq!(
        last_run.status,
        RunStatus::Success,
        "status should be Success"
    );
    // {{panels.list}} 点号路径应取到嵌套数组 → 循环体执行 3 次（3 条 body 记录）
    let body_count = last_run
        .steps
        .iter()
        .filter(|s| s.step_id == body_id)
        .count();
    assert_eq!(
        body_count, 3,
        "loop body should run 3 times, got {} (steps: {:?})",
        body_count, last_run.steps
    );
    // 容器 loop 步骤本身也记录
    assert!(
        last_run.steps.iter().any(|s| s.step_id == loop_id),
        "loop container step should be recorded"
    );
}

#[tokio::test]
async fn test_failed_run_creates_error_record_without_corrupting_previous() {
    let tmp = std::env::temp_dir().join("nuphus_test_fail_push");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // Run 1: 成功
    let mut wf = Workflow::new("ok_then_fail");
    wf.steps = vec![make_tool_step("ok1", "mock_echo", serde_json::json!({}))];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let ok_tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            ok_tool_exec,
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_ok(), "first run should succeed: {:?}", r1.err());
    let wf_after_1 = store.get(&wf_id).await.unwrap();
    assert_eq!(wf_after_1.last_run().unwrap().status, RunStatus::Success);

    // Run 2: 失败
    let mut wf2 = store.get(&wf_id).await.unwrap();
    wf2.steps = vec![make_tool_step("failing", "bad_tool", serde_json::json!({}))];
    store.save(&wf2).await.unwrap();

    let fail_tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            fail_tool_exec,
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail");

    let wf_after_2 = store.get(&wf_id).await.unwrap();
    let history = &wf_after_2.run_history;
    assert!(
        history.len() >= 2,
        "history should have 2 records, got {}",
        history.len()
    );
    // 最新记录是 Error（新 push），上一条成功记录不得被污染
    assert!(
        matches!(history[0].status, RunStatus::Error(_)),
        "latest record should be Error, got {:?}",
        history[0].status
    );
    assert_eq!(
        history[1].status,
        RunStatus::Success,
        "previous success record must be untouched, got {:?}",
        history[1].status
    );
    // 失败步骤被记录为 Error
    assert!(
        history[0]
            .steps
            .iter()
            .any(|s| s.status != StepRunStatus::Success),
        "failed run should contain a non-Success step record: {:?}",
        history[0].steps
    );
}

#[tokio::test]
async fn test_resume_skips_completed_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_resume");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 工作流：[s1 成功, s2 失败]；s1 带计数器，重试时不应被重新执行
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("resume_skip");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "bad_tool", serde_json::json!({}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("simulated tool failure".to_string())
                    }
                }
            }
        }
    };

    // 首次执行：s1 成功、s2 失败
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_err(), "first run should fail at s2");

    let wf_after_1 = store.get(&wf_id).await.unwrap();
    assert!(
        matches!(wf_after_1.last_run().unwrap().status, RunStatus::Error(_)),
        "first failed run must be recorded"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 executed once in first run"
    );

    // 重试：completed_ids 应从上次 Error 记录提取 s1（Success）→ s1 跳过，只重跑 s2
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail at s2 again");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 must NOT be re-executed on resume (completed_ids skip)"
    );

    // 第二次 run 记录：含继承的 s1(Success) + 本次 s2(Error)
    let wf_after_2 = store.get(&wf_id).await.unwrap();
    let last = wf_after_2.last_run().unwrap();
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s1_id && s.status == StepRunStatus::Success),
        "resumed run should inherit s1 Success record: {:?}",
        last.steps
    );
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s2_id && matches!(s.status, StepRunStatus::Error(_))),
        "resumed run should record s2 Error: {:?}",
        last.steps
    );
}
#[tokio::test]
async fn test_paused_resume_skips_completed_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_paused_resume");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("paused_resume");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "counted_tool", serde_json::json!({}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    // 手动构造 Paused 状态的 last_run：s1 已完成（Success），模拟"暂停后进程重启"断点
    let mut wf_paused = store.get(&wf_id).await.unwrap();
    wf_paused.push_run(RunRecord {
        run_id: "paused-run-1".into(),
        started_at: chrono::Utc::now(),
        finished_at: None,
        status: RunStatus::Paused,
        steps: vec![StepRunRecord {
            step_id: s1_id.clone(),
            started_at: chrono::Utc::now(),
            finished_at: Some(chrono::Utc::now()),
            status: StepRunStatus::Success,
            output_summary: None,
        }],
        error: None,
        variables_snapshot: std::collections::HashMap::new(),
    });
    store.save(&wf_paused).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("unexpected tool".to_string())
                    }
                }
            }
        }
    };

    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "paused resume should succeed: {:?}",
        result.err()
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 (Paused 已完成) 不得重跑，只执行 s2"
    );

    let wf_after = store.get(&wf_id).await.unwrap();
    let last = wf_after.last_run().unwrap();
    assert_eq!(last.status, RunStatus::Success);
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s1_id && s.status == StepRunStatus::Success),
        "resumed run should inherit s1 Success: {:?}",
        last.steps
    );
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s2_id && s.status == StepRunStatus::Success),
        "resumed run should record s2 Success: {:?}",
        last.steps
    );
}

/// 生产路径复现：失败 → store.load_all() 热刷新（清缓存重读磁盘）→ 再执行。
/// 模拟 workflow_run 两次调用之间夹了一次 load_all 的情况。
/// 若 run_history(Error) 未持久化或 load_all 丢失 run_history，则 s1 会重跑 → 断言失败。
#[tokio::test]
async fn test_resume_survives_hot_reload() {
    let tmp = std::env::temp_dir().join("nuphus_test_resume_reload");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("resume_reload");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "bad_tool", serde_json::json!({}));
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("simulated tool failure".to_string())
                    }
                }
            }
        }
    };

    // 第一次执行：s1 成功、s2 失败
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_err(), "first run should fail at s2");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 生产路径：workflow_run 两次调用之间 store.load_all() 热刷新（清缓存重读磁盘）
    store.load_all().await.unwrap();

    // 第二次执行：resume 应跳过 s1，只重跑 s2
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail at s2 again");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 must NOT be re-executed on resume after load_all hot reload"
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

// ── Chat 步骤 per-step 模型路由 ──

/// 最小 mock ApiClient：固定返回文本（无工具调用 → chat 一轮结束）
struct MockChatClient;

#[async_trait::async_trait]
impl crate::api::ApiClient for MockChatClient {
    async fn stream(
        &self,
        _request: crate::api::MessageRequest,
    ) -> crate::Result<Vec<crate::api::AssistantEvent>> {
        Ok(vec![
            crate::api::AssistantEvent::TextDelta("mock reply".to_string()),
            crate::api::AssistantEvent::MessageStop,
        ])
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn provider_kind(&self) -> crate::api::ProviderKind {
        crate::api::ProviderKind::Custom
    }

    fn provider_name(&self) -> &str {
        ""
    }
}

fn make_chat_step(name: &str, message: &str, model: Option<&str>) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Chat {
            chat: message.to_string(),
            with: ChatOpts {
                model: model.map(|m| m.to_string()),
                ..Default::default()
            },
        },
        ..Default::default()
    }
}

/// registry 只含 routed-model（指向不可达地址）
fn make_test_factory() -> crate::llm::ClientFactory {
    let registry = crate::config::ModelRegistry::from_single(
        "routed-model".to_string(),
        "custom".to_string(),
        "test-key".to_string(),
        "http://127.0.0.1:1".to_string(),
        None,
    );
    crate::llm::ClientFactory::new(registry)
}

/// with.model 不在 registry → 回退主 client（裸模型名语义，向后兼容）
#[tokio::test]
async fn test_chat_model_fallback_bare_name() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_fallback");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_fallback_test");
    wf.steps = vec![make_chat_step(
        "chat_step",
        "hello",
        Some("bare-name-model"),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "unknown model id must fallback to main client: {:?}",
        result.err()
    );
}

/// with.model 命中 registry → 走 factory 装配的专属 client（不可达地址 → 失败即证明已路由，
/// 若走 mock 主 client 则会成功）
#[tokio::test]
async fn test_chat_model_routed_to_registry_client() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_routed");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_routed_test");
    wf.steps = vec![make_chat_step("chat_step", "hello", Some("routed-model"))];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let err = result.expect_err("routed client (unreachable) must fail, proving routing happened");
    assert!(
        err.to_string().contains("LLM call failed"),
        "error should come from routed client's stream call: {}",
        err
    );
}

/// 无 with.model → 现状不变（主 client + 默认模型名）
#[tokio::test]
async fn test_chat_no_model_uses_main_client() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_default");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_default_test");
    wf.steps = vec![make_chat_step("chat_step", "hello", None)];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "chat without with.model must use main client: {:?}",
        result.err()
    );
}

/// P2 修复验证：失败步骤必须同时发 Error（message 横幅）+ StepRunCompleted{Error}。
/// 旧实现只发 Error（无 step_id），前端无法定位失败步骤，run_completed 时被误收敛为绿色
/// completed。前端据此识别 {"Error": ...} 形状标红叉。
#[tokio::test]
async fn test_failed_step_emits_error_and_step_run_completed_error() {
    let tmp = std::env::temp_dir().join("nuphus_test_fail_emit");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let mut wf = Workflow::new("fail_emit_test");
    wf.steps = vec![make_tool_step(
        "fail_step",
        "mock_fail",
        serde_json::json!({}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_err(), "模拟失败步骤应使执行失败");

    // 断言事件流：Error + StepRunCompleted{Error} 均已发出（执行期间事件已入队，逐一 drain）
    let mut saw_error = false;
    let mut saw_step_completed_error = false;
    loop {
        match rx.try_recv() {
            Ok(WorkflowEvent::Error { .. }) => saw_error = true,
            Ok(WorkflowEvent::StepRunCompleted {
                status: StepRunStatus::Error(_),
                ..
            }) => saw_step_completed_error = true,
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(_) => break,
        }
    }
    assert!(saw_error, "失败步骤应发 Error 事件（message 横幅数据源）");
    assert!(
        saw_step_completed_error,
        "失败步骤应发 StepRunCompleted{{Error}}（前端据此标红叉，而非误收敛为绿色 completed）"
    );
}

/// wait 步骤的提示语必须做 `{{var}}` 模板替换（与 chat / script / tool / mcp 对齐）。
///
/// 回归背景：实测 HUD 上显示的是字面的 "等待: 已读取到 {{pending_raw}}…" ——
/// 用户既看不出在等什么，也看不到该步骤本该汇报的内容（待学课程清单）。
/// 根因：`Action::Wait` 把 wait 文案原样传给执行器，唯独它没走 resolve_vars_str，
/// 而同一条提示语里的变量在上游步骤已经 `capture` 成功。
#[tokio::test]
async fn wait_prompt_is_variable_substituted() {
    async fn pending_tool_exec(
        _tool: String,
        _params: serde_json::Value,
    ) -> StdResult<String, String> {
        Ok("5 门课未完成".to_string())
    }

    let tmp = std::env::temp_dir().join("nuphus_test_wait_vars");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    // 上游：工具步骤把输出 capture 成变量 pending_raw（非 JSON → 原样存字符串）
    let mut capture_step = make_tool_step("read_pending", "mock_echo", serde_json::json!({}));
    capture_step.capture = Some("pending_raw".to_string());

    // 下游：wait 步骤的提示语引用该变量
    let wait_step = Step {
        id: "report_plan".to_string(),
        name: "汇报学习计划".to_string(),
        action: Action::Wait {
            wait: "已读取到 {{pending_raw}}。点击继续开始。".to_string(),
            auto: vec![],
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("wait_vars_test");
    wf.steps = vec![capture_step, wait_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| pending_tool_exec(tool, params);
    // wait 步骤会一直等"继续"：StepRunPaused 在进入等待轮询**之前**就已发出，
    // 所以用超时掐掉即可断言事件，不需要真的 resume（否则测试要挂到 30 分钟超时）。
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(900),
        executor.execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        ),
    )
    .await;

    let mut reasons: Vec<String> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::StepRunPaused { reason, .. } = ev {
            reasons.push(reason);
        }
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(!reasons.is_empty(), "wait 步骤应发出 StepRunPaused");
    assert!(
        reasons.iter().any(|r| r.contains("5 门课未完成")),
        "wait 提示语里的 {{{{pending_raw}}}} 应被替换为变量值，实际: {reasons:?}"
    );
    assert!(
        !reasons.iter().any(|r| r.contains("{{")),
        "wait 提示语不应残留模板占位符，实际: {reasons:?}"
    );
}

// ── 声明式外部输入（workflow.inputs）──

fn input_spec(
    name: &str,
    required: bool,
    default: Option<serde_json::Value>,
    sensitive: bool,
) -> InputSpec {
    InputSpec {
        name: name.to_string(),
        kind: InputKind::String,
        required,
        default,
        description: None,
        sensitive,
    }
}

/// 未提供且非必填但有 default → 注入 default；显式提供 → 覆盖 default；非必填缺省 → 不注入
#[test]
fn resolve_declared_inputs_default_override_and_absent() {
    let specs = vec![
        input_spec("with_default", false, Some(serde_json::json!("d")), false),
        input_spec("optional_absent", false, None, false),
        input_spec("overridden", false, Some(serde_json::json!("d")), false),
    ];
    let mut provided = HashMap::new();
    provided.insert("overridden".to_string(), serde_json::json!("explicit"));

    let resolved = resolve_declared_inputs(&specs, &provided).expect("非必填缺省不应报错");
    assert_eq!(resolved.get("with_default"), Some(&serde_json::json!("d")));
    assert_eq!(
        resolved.get("overridden"),
        Some(&serde_json::json!("explicit"))
    );
    assert!(
        !resolved.contains_key("optional_absent"),
        "非必填且无 default 的输入不应注入该键: {resolved:?}"
    );
}

/// required 且无 default 且未提供 → 执行前报错，原因可读（含输入名）
#[test]
fn resolve_declared_inputs_required_missing_errors() {
    let specs = vec![input_spec("token", true, None, false)];
    let err = resolve_declared_inputs(&specs, &HashMap::new()).expect_err("缺必填输入应报错");
    let msg = err.to_string();
    assert!(msg.contains("缺少必填输入：token"), "错误原因应可读: {msg}");
}

/// required 但有 default → 用 default；显式提供优先于 required 判定
#[test]
fn resolve_declared_inputs_required_with_default() {
    let specs = vec![input_spec(
        "mode",
        true,
        Some(serde_json::json!("fast")),
        false,
    )];
    let resolved = resolve_declared_inputs(&specs, &HashMap::new()).expect("有 default 不应报错");
    assert_eq!(resolved.get("mode"), Some(&serde_json::json!("fast")));

    let mut provided = HashMap::new();
    provided.insert("mode".to_string(), serde_json::json!("slow"));
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert_eq!(resolved.get("mode"), Some(&serde_json::json!("slow")));
}

/// 未声明的显式输入不进入声明结果（是否散注入顶层由注入层决定）；空声明 → 空结果（旧工作流语义）
#[test]
fn resolve_declared_inputs_ignores_undeclared_and_empty_specs() {
    let specs: Vec<InputSpec> = Vec::new();
    let mut provided = HashMap::new();
    provided.insert("legacy".to_string(), serde_json::json!("v"));
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert!(resolved.is_empty(), "空声明应返回空结果: {resolved:?}");

    let specs = vec![input_spec("declared", false, None, false)];
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert!(
        !resolved.contains_key("legacy"),
        "未声明的运行时输入不应被提升为声明项: {resolved:?}"
    );
}

/// 声明项双写取值：{{inputs.x}} 与 {{x}} 均可取到；支持管道与文本内嵌
#[test]
fn variables_input_namespace_and_top_level() {
    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    vars.insert(
        "inputs".to_string(),
        serde_json::json!({"name": "Nuphus", "flag": true}),
    );
    vars.insert("name".to_string(), serde_json::json!("Nuphus"));

    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.name}}"), &vars),
        serde_json::json!("Nuphus")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{name}}"), &vars),
        serde_json::json!("Nuphus")
    );
    // 类型不字符串化（布尔/数字保持原始类型）
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.flag}}"), &vars),
        serde_json::json!(true)
    );
    // 允许 {{ inputs.x }} 带空格；文本内嵌与管道写法均支持
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{ inputs.name }}"), &vars),
        serde_json::json!("Nuphus")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("dir-{{inputs.name}}/x"), &vars),
        serde_json::json!("dir-Nuphus/x")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.name | default \"d\"}}"), &vars),
        serde_json::json!("Nuphus")
    );
    // 字符串模板路径（wait 提示语 / chat message / script code 同源）
    assert_eq!(
        super::variables::resolve_vars_str("hi {{inputs.name}}", &vars),
        "hi Nuphus"
    );
}
