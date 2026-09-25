use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;

fn step(id: &str, action: Value) -> Step {
    serde_json::from_value(json!({"id":id,"name":id,"do":action})).unwrap()
}

fn setup(steps: Vec<Step>) -> (WorkflowEngine, Workflow, std::path::PathBuf) {
    let mut workflow = Workflow::new("debug test");
    workflow.steps = steps;
    let directory =
        std::env::temp_dir().join(format!("nuphus-debug-test-{}", uuid::Uuid::new_v4()));
    let mut engine = WorkflowEngine::new();
    engine.store = WorkflowStore::frozen(directory.clone(), vec![workflow.clone()]);
    (engine, workflow, directory)
}

fn request(workflow: &Workflow, selected: &str, mode: DebugMode) -> DebugRequest {
    DebugRequest {
        workflow_id: workflow.id.clone(),
        steps: workflow.steps.clone(),
        inputs: None,
        selected_step_id: selected.into(),
        mode,
        variables: HashMap::new(),
        runtime_inputs: HashMap::new(),
        use_retry_policy: false,
        source: Some(json!({"kind":"manual"})),
    }
}

#[tokio::test]
async fn missing_legacy_variable_does_not_dispatch_and_unrelated_required_input_does_not_block() {
    let (mut engine, mut workflow, directory) = setup(vec![step(
        "selected",
        json!({"tool":"write","with":{"target":"{{missing}}"}}),
    )]);
    workflow.inputs =
        serde_json::from_value(json!([{"name":"unrelated", "required":true}])).unwrap();
    engine.store = WorkflowStore::frozen(directory.clone(), vec![workflow.clone()]);
    let session = engine
        .prepare_debug(request(&workflow, "selected", DebugMode::Node))
        .await
        .unwrap();
    let result = CURRENT
        .scope(
            session,
            engine.execute_workflow(
                &workflow.id,
                |_, _| async { panic!("missing legacy variable must not dispatch") },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("missing_test_variable"));
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn until_field_is_checked_after_body_and_skipped_breakpoint_is_reported() {
    let (engine, workflow, directory) = setup(vec![step(
        "loop",
        json!({"loop":{"until":{"equals":[{"var":"result[\"done\"]"},"true"]},"max":2,"do":[{"id":"body","name":"Body","capture":"result","do":{"tool":"read","with":{}}}]}}),
    )]);
    let session = engine
        .prepare_debug(request(&workflow, "loop", DebugMode::Node))
        .await
        .unwrap();
    let result = CURRENT
        .scope(
            session,
            engine.execute_workflow(
                &workflow.id,
                |_, _| async { Ok(r#"{"done":true}"#.into()) },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result.is_ok(), "{result:?}");
    let branch = step(
        "branch",
        json!({"if":{"condition":{"equals":["yes","no"]},"then":[{"id":"target","name":"Target","do":{"sleep":0.01}}]}}),
    );
    let mut req = request(&workflow, "target", DebugMode::Through);
    req.steps = vec![branch];
    let session = engine.prepare_debug(req).await.unwrap();
    let result = CURRENT
        .scope(
            session,
            engine.execute_workflow(
                &workflow.id,
                |_, _| async { panic!("no tools") },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("debug_target_not_reached"));
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn single_node_runs_only_selected_subtree_with_manual_values_and_no_normal_history() {
    let (engine, workflow, directory) = setup(vec![
        step("prerequisite", json!({"tool":"write","with":{}})),
        step(
            "selected",
            json!({"tool":"write","with":{"value":"{{wn[\"window_id\"]}}"}}),
        ),
        step("later", json!({"tool":"write","with":{}})),
    ]);
    let mut req = request(&workflow, "selected", DebugMode::Node);
    req.variables.insert("wn".into(), json!({"window_id":123}));
    let session = engine.prepare_debug(req).await.unwrap();
    let calls = std::sync::Mutex::new(Vec::new());
    let result = CURRENT
        .scope(
            session.clone(),
            engine.execute_workflow(
                &workflow.id,
                |_, params| {
                    calls.lock().unwrap().push(params);
                    async { Ok("full native output".into()) }
                },
                None,
                None,
                Some(session.runtime_inputs.clone()),
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(*calls.lock().unwrap(), vec![json!({"value":123})]);
    assert!(engine
        .store
        .get(&workflow.id)
        .await
        .unwrap()
        .run_history
        .is_empty());
    assert!(!directory.join(&workflow.id).join("workflow.json").exists());
    let traces = super::super::trace::list(&directory, &workflow.id, true)
        .await
        .unwrap();
    assert_eq!(traces.len(), 1);
    let invocation = traces[0]
        .invocations
        .iter()
        .find(|i| i.step_id == "selected")
        .unwrap();
    let trace = super::super::trace::read(
        &directory,
        &workflow.id,
        &session.run_id,
        true,
        invocation.id,
    )
    .await
    .unwrap();
    assert_eq!(trace.inputs, json!({"value":123}));
    assert_eq!(trace.attempts.len(), 1);
    assert_eq!(
        trace.output.as_deref(),
        Some("tool_completed:full native output")
    );
    assert!(trace.verification.is_none());
    assert!(super::super::trace::list(&directory, &workflow.id, false)
        .await
        .unwrap()
        .is_empty());
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn through_pauses_after_first_selected_invocation_then_resumes_remaining_iterations() {
    let repeated = step("selected", json!({"tool":"write","with":{}}));
    let (engine, workflow, directory) = setup(vec![
        step("loop", json!({"loop":{"repeat":2,"do":[repeated]}})),
        step("later", json!({"tool":"write","with":{}})),
    ]);
    let session = engine
        .prepare_debug(request(&workflow, "selected", DebugMode::Through))
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    let execute = CURRENT.scope(
        session.clone(),
        engine.execute_workflow(
            &workflow.id,
            |_, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok("done".into()) }
            },
            None,
            None,
            None,
            true,
            super::super::WorkflowRunSource::Ui,
        ),
    );
    let control = async {
        loop {
            if engine.is_paused(&workflow.id).await {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "breakpoint is inclusive and fires immediately"
        );
        let runs = super::super::trace::list(&directory, &workflow.id, true)
            .await
            .unwrap();
        assert!(runs[0]
            .invocations
            .iter()
            .any(|i| i.step_id == "selected" && i.status == "success"));
        assert!(engine
            .debug_control(&workflow.id, "wrong-run", "resume")
            .await
            .is_err());
        engine
            .debug_control(&workflow.id, &session.run_id, "resume")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(execute, control)
    })
    .await
    .unwrap();
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let runs = super::super::trace::list(&directory, &workflow.id, true)
        .await
        .unwrap();
    let selected: Vec<_> = runs[0]
        .invocations
        .iter()
        .filter(|i| i.step_id == "selected")
        .collect();
    assert_eq!(selected.len(), 2);
    assert_ne!(selected[0].id, selected[1].id);
    assert_eq!(selected[0].parent_id, selected[1].parent_id);
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn debug_disables_automatic_retry_and_records_missing_fields_as_failed_invocations() {
    let (engine, workflow, directory) =
        setup(vec![step("selected", json!({"tool":"write","with":{}}))]);
    let session = engine
        .prepare_debug(request(&workflow, "selected", DebugMode::Node))
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    let result = CURRENT
        .scope(
            session.clone(),
            engine.execute_workflow(
                &workflow.id,
                |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Err("uncertain side effect".into()) }
                },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut req = request(&workflow, "selected", DebugMode::Node);
    req.steps[0] = step(
        "selected",
        json!({"tool":"write","with":{"target":"{{wn[\"missing\"]}}"}}),
    );
    req.variables.insert("wn".into(), json!({}));
    let session = engine.prepare_debug(req).await.unwrap();
    let result = CURRENT
        .scope(
            session.clone(),
            engine.execute_workflow(
                &workflow.id,
                |_, _| async { panic!("missing field must never dispatch") },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        )
        .await;
    assert!(result.is_err());
    let runs = super::super::trace::list(&directory, &workflow.id, true)
        .await
        .unwrap();
    let run = runs.iter().find(|r| r.run_id == session.run_id).unwrap();
    let failed = run
        .invocations
        .iter()
        .find(|i| i.step_id == "selected")
        .unwrap();
    let invocation =
        super::super::trace::read(&directory, &workflow.id, &run.run_id, true, failed.id)
            .await
            .unwrap();
    assert!(invocation
        .error
        .unwrap()
        .contains("missing_field_reference"));
    assert!(invocation.attempts.is_empty());
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn cancel_interrupts_frozen_child_workflow_and_preserves_source_history() {
    let mut child = Workflow::new("child");
    child.steps = vec![step("child-sleep", json!({"sleep":60}))];
    let (engine, workflow, directory) = setup(vec![step("call", json!({"call":child.id}))]);
    engine.store.save(&child).await.unwrap();
    // The source store is memory-only in this test; its list must include child definitions.
    let mut engine = engine;
    engine.store = WorkflowStore::frozen(directory.clone(), vec![workflow.clone(), child.clone()]);
    let session = engine
        .prepare_debug(request(&workflow, "call", DebugMode::Node))
        .await
        .unwrap();
    child.steps = vec![step("changed", json!({"tool":"unexpected","with":{}}))];
    engine.store.save(&child).await.unwrap();
    let execute = CURRENT.scope(
        session.clone(),
        engine.execute_workflow(
            &workflow.id,
            |_, _| async { panic!("child snapshot changed") },
            None,
            None,
            None,
            true,
            super::super::WorkflowRunSource::Ui,
        ),
    );
    let control = async {
        loop {
            let runs = super::super::trace::list(&directory, &workflow.id, true)
                .await
                .unwrap();
            if runs[0]
                .invocations
                .iter()
                .any(|i| i.step_id == "child-sleep")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        engine
            .debug_control(&workflow.id, &session.run_id, "cancel")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(execute, control)
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(engine
        .store
        .get(&child.id)
        .await
        .unwrap()
        .run_history
        .is_empty());
    assert!(!engine.is_paused(&workflow.id).await);
    let runs = super::super::trace::list(&directory, &workflow.id, true)
        .await
        .unwrap();
    assert_eq!(runs[0].status, "cancelled");
    assert!(runs[0].invocations.iter().all(|i| i.status != "running"));
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn empty_repeat_is_bounded_by_debug_budget() {
    let (engine, workflow, directory) = setup(vec![step(
        "loop",
        json!({"loop":{"repeat":1000000,"do":[]}}),
    )]);
    let session = engine
        .prepare_debug(request(&workflow, "loop", DebugMode::Node))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        CURRENT.scope(
            session,
            engine.execute_workflow(
                &workflow.id,
                |_, _| async { panic!("no tools") },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        ),
    )
    .await
    .unwrap();
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("debug_step_budget_exceeded"));
    let _ = tokio::fs::remove_dir_all(directory).await;
}

#[tokio::test]
async fn through_break_and_container_stop_before_downstream_side_effects() {
    for selected in ["stop", "loop"] {
        let (engine, workflow, directory) = setup(vec![
            step(
                "loop",
                json!({"loop":{"repeat":2,"do":[
                    {"id":"first","name":"First","do":{"tool":"write","with":{}}},
                    {"id":"stop","name":"Stop","do":{"break":true}},
                    {"id":"unreachable","name":"Unreachable","do":{"tool":"write","with":{}}}
                ]}}),
            ),
            step("later", json!({"tool":"write","with":{}})),
        ]);
        let session = engine
            .prepare_debug(request(&workflow, selected, DebugMode::Through))
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);
        let execute = CURRENT.scope(
            session.clone(),
            engine.execute_workflow(
                &workflow.id,
                |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("done".into()) }
                },
                None,
                None,
                None,
                true,
                super::super::WorkflowRunSource::Ui,
            ),
        );
        let control = async {
            while !engine.is_paused(&workflow.id).await {
                tokio::task::yield_now().await;
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            engine
                .debug_control(&workflow.id, &session.run_id, "resume")
                .await
                .unwrap();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(execute, control)
        })
        .await
        .unwrap();
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _ = tokio::fs::remove_dir_all(directory).await;
    }
}
