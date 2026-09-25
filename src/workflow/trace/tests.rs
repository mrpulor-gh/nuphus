use super::*;
use serde_json::json;

fn fixture() -> (Workflow, PathBuf, String) {
    (
        Workflow::new("Evidence test"),
        std::env::temp_dir().join(format!("nuphus-trace-test-{}", uuid::Uuid::new_v4())),
        uuid::Uuid::new_v4().to_string(),
    )
}

#[tokio::test]
async fn secrets_do_not_corrupt_record_identifiers_and_dynamic_child_values_are_redacted() {
    let (wf, root, run_id) = fixture();
    let recorder = TraceRecorder::create(&root, &wf, &run_id, false, vec!["1".into()])
        .await
        .unwrap();
    let step = Step::new_seq("step1", "Node 1", vec![]);
    let vars = HashMap::from([
        ("secret".into(), json!(1)),
        ("text".into(), json!("later-secret")),
    ]);
    let id = recorder
        .begin(&wf.id, &step, json!({"value":1}), &vars)
        .await;
    recorder.add_sensitive([json!("later-secret")]);
    recorder
        .finish(id, &vars, Ok("result 1 later-secret"), None)
        .await;
    recorder.complete("success").await;
    let runs = list(&root, &wf.id, false).await.unwrap();
    assert_eq!(runs[0].run_id, run_id);
    assert_eq!(runs[0].invocations[0].id, id);
    assert_eq!(runs[0].invocations[0].step_id, "step1");
    let trace = read(&root, &wf.id, &run_id, false, id).await.unwrap();
    assert_eq!(trace.variables_before["secret"], "[redacted]");
    assert_eq!(trace.variables_before["text"], "[redacted]");
    assert_eq!(
        trace.output.as_deref(),
        Some("result [redacted] [redacted]")
    );
    assert!(trace.verification.is_none());
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn dropped_child_does_not_become_next_siblings_parent_and_completion_is_idempotent() {
    let (wf, root, run_id) = fixture();
    let recorder = TraceRecorder::create(&root, &wf, &run_id, true, vec![])
        .await
        .unwrap();
    let vars = HashMap::new();
    let parent = recorder
        .begin(
            &wf.id,
            &Step::new_seq("parent", "Parent", vec![]),
            Value::Null,
            &vars,
        )
        .await;
    let child = recorder
        .begin(
            &wf.id,
            &Step::new_seq("child", "Child", vec![]),
            Value::Null,
            &vars,
        )
        .await;
    recorder.finish(parent, &vars, Err("timeout"), None).await;
    let sibling = recorder
        .begin(
            &wf.id,
            &Step::new_seq("next", "Next", vec![]),
            Value::Null,
            &vars,
        )
        .await;
    recorder.complete("cancelled").await;
    recorder.complete("success").await;
    let trace = read(&root, &wf.id, &run_id, true, child).await.unwrap();
    assert_eq!(trace.summary.status, "interrupted");
    let trace = read(&root, &wf.id, &run_id, true, sibling).await.unwrap();
    assert_eq!(trace.summary.parent_id, None);
    assert_eq!(
        list(&root, &wf.id, true).await.unwrap()[0].status,
        "cancelled"
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[test]
fn verification_is_native_evidence_not_a_success_guess() {
    assert!(verification("done").is_none());
    assert!(verification(r#"{"success":true}"#).is_none());
    assert_eq!(
        verification(r#"tool_completed:{"verification":{"status":"unverifiable"}}"#),
        Some(json!({"verification":{"status":"unverifiable"}}))
    );
    assert!(run_directory(
        Path::new("E:/tests"),
        "../other",
        &uuid::Uuid::new_v4().to_string(),
        false
    )
    .is_err());
}
