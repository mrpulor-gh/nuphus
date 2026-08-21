//! 生命周期控制：暂停、恢复、取消
use super::*;

impl Executor {
    /// 暂停执行（用户主动暂停）
    pub async fn pause(&self, workflow_id: &str) {
        self.pause_notifies
            .write()
            .await
            .entry(workflow_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Notify::new()));
    }

    /// 查询是否处于暂停状态
    pub async fn is_paused(&self, workflow_id: &str) -> bool {
        self.pause_notifies.read().await.contains_key(workflow_id)
    }

    /// 恢复执行
    pub async fn resume(&self, workflow_id: &str) {
        if let Some(notify) = self.pause_notifies.write().await.remove(workflow_id) {
            notify.notify_one();
        }
    }

    /// 取消执行
    pub async fn cancel(&self, workflow_id: &str) {
        if let Some(flag) = self.cancel_flags.write().await.get(workflow_id).cloned() {
            flag.store(true, Ordering::Relaxed);
        }
        // Also wake the pause notify, so a paused workflow can be cancelled
        if let Some(notify) = self.pause_notifies.write().await.remove(workflow_id) {
            notify.notify_one();
        }
    }

    /// 检查取消标志，被取消时返回 Err
    pub(super) async fn check_cancel(&self, workflow_id: &str) -> crate::Result<()> {
        if let Some(flag) = self.cancel_flags.read().await.get(workflow_id) {
            if flag.load(Ordering::Relaxed) {
                return Err(crate::NuphusError::Agent(crate::AgentError::Other(
                    "工作流已被取消".to_string(),
                )));
            }
        }
        Ok(())
    }

    /// 检查暂停标志，被暂停时阻塞直到恢复或取消
    /// 唤醒后检查 cancel_flag：若因取消唤醒则返回 Err
    ///
    /// 暂停前保存变量快照到 RunRecord（断点续连恢复变量池用）。
    pub(super) async fn check_pause(
        &self,
        workflow_id: &str,
        events: &EventBus,
        step_id: &str,
        step_name: &str,
        step_depth: u32,
        step_kind: &str,
        store: Option<&WorkflowStore>,
        variables: Option<&HashMap<String, serde_json::Value>>,
    ) -> crate::Result<()> {
        let notifies = self.pause_notifies.read().await;
        if let Some(notify) = notifies.get(workflow_id) {
            let notify = notify.clone();
            drop(notifies);

            // ── 暂停前保存变量快照 ──
            if let (Some(store), Some(vars)) = (store, variables) {
                if let Some(mut wf) = store.get(workflow_id).await {
                    if let Some(ref mut rec) = wf.last_run_mut() {
                        let snapshot: HashMap<String, serde_json::Value> =
                            vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        if !snapshot.is_empty() {
                            rec.variables_snapshot = snapshot;
                        }
                    }
                    let _ = store.save(&wf).await;
                }
            }

            events.emit(WorkflowEvent::StepRunPaused {
                step_id: step_id.to_string(),
                step_name: step_name.to_string(),
                reason: "用户暂停".to_string(),
            });
            notify.notified().await;
            // 唤醒后检查：是否因取消而唤醒（cancel 会移除通知器并 notify_one）
            if let Some(flag) = self.cancel_flags.read().await.get(workflow_id) {
                if flag.load(Ordering::Relaxed) {
                    return Err(crate::NuphusError::Agent(crate::AgentError::Other(
                        "工作流已被取消".to_string(),
                    )));
                }
            }
            events.emit(WorkflowEvent::StepRunStarted {
                step_id: step_id.to_string(),
                step_name: step_name.to_string(),
                depth: step_depth,
                kind: step_kind.to_string(),
            });
        }
        Ok(())
    }
}
