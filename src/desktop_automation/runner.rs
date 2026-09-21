use super::types::*;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[async_trait]
pub trait ComputerObserver: Send + Sync {
    fn capabilities(&self) -> PlatformCapabilities;
    async fn observe(&self, scope: &ObservationScope) -> Result<Observation, AutomationError>;
}

pub trait CandidateBuilder: Send + Sync {
    fn build(
        &self,
        goal: &str,
        observation: &Observation,
    ) -> Result<Vec<ActionCandidate>, AutomationError>;
}

#[async_trait]
pub trait ComputerExecutor: Send + Sync {
    async fn execute(
        &self,
        fresh: &Observation,
        action: &ActionCandidate,
    ) -> Result<ActionReceipt, AutomationError>;
}

#[async_trait]
pub trait Verifier: Send + Sync {
    async fn verify(
        &self,
        before: &Observation,
        action: &ActionCandidate,
        receipt: &ActionReceipt,
        after: &Observation,
    ) -> Result<Verification, AutomationError>;
}

/// One shared bounded loop. `enhanced_mode` switches only the decision
/// provider; observation, policy, execution and verification are identical.
pub struct AutomationRunner {
    observer: Arc<dyn ComputerObserver>,
    candidates: Arc<dyn CandidateBuilder>,
    normal_decider: Arc<dyn DecisionProvider>,
    jev_decider: Option<Arc<dyn DecisionProvider>>,
    policy: Arc<dyn Policy>,
    executor: Arc<dyn ComputerExecutor>,
    verifier: Arc<dyn Verifier>,
    limits: RunLimits,
}

impl AutomationRunner {
    pub fn new(
        observer: Arc<dyn ComputerObserver>,
        candidates: Arc<dyn CandidateBuilder>,
        normal_decider: Arc<dyn DecisionProvider>,
        policy: Arc<dyn Policy>,
        executor: Arc<dyn ComputerExecutor>,
        verifier: Arc<dyn Verifier>,
        limits: RunLimits,
    ) -> Self {
        Self {
            observer,
            candidates,
            normal_decider,
            jev_decider: None,
            policy,
            executor,
            verifier,
            limits,
        }
    }

    pub fn with_jev_decider(mut self, jev_decider: Arc<dyn DecisionProvider>) -> Self {
        self.jev_decider = Some(jev_decider);
        self
    }

    pub async fn run(&self, request: RunRequest) -> Result<RunOutcome, AutomationError> {
        let started = Instant::now();
        let hard_elapsed = Duration::from_millis(self.limits.hard_max_elapsed_ms);
        let soft_elapsed = Duration::from_millis(self.limits.soft_max_elapsed_ms);
        let mut trace = Vec::new();
        let mut executed_steps = 0_u32;
        let mut recent = Vec::new();
        let mut stall_count = 0_u32;
        let mut soft_reported = false;

        loop {
            if executed_steps >= self.limits.hard_max_steps || started.elapsed() >= hard_elapsed {
                trace.push(stop_event("hard run budget exceeded"));
                return Ok(RunOutcome {
                    status: RunStatus::BudgetExceeded,
                    executed_steps,
                    trace,
                });
            }
            if !soft_reported
                && (executed_steps >= self.limits.soft_max_steps
                    || started.elapsed() >= soft_elapsed)
            {
                soft_reported = true;
                trace.push(TraceEvent {
                    kind: TraceEventKind::SoftBudgetReached,
                    observation_revision: None,
                    candidate_id: None,
                    detail: "soft budget reached; continuing within the hard bound".into(),
                });
            }

            let observation = self.observer.observe(&request.scope).await?;
            trace.push(TraceEvent {
                kind: TraceEventKind::Observed,
                observation_revision: Some(observation.revision),
                candidate_id: None,
                detail: observation.fingerprint.clone(),
            });

            let candidates = self.candidates.build(&request.goal, &observation)?;
            validate_candidates(&observation, &candidates)?;
            trace.push(TraceEvent {
                kind: TraceEventKind::CandidatesBuilt,
                observation_revision: Some(observation.revision),
                candidate_id: None,
                detail: format!("{} bounded candidates", candidates.len()),
            });

            let input = DecisionInput {
                goal: request.goal.clone(),
                observation: observation.clone(),
                candidates: candidates.clone(),
                recent_candidate_ids: recent.clone(),
            };
            let decider: &dyn DecisionProvider = if request.enhanced_mode {
                self.jev_decider.as_deref().ok_or_else(|| {
                    AutomationError::EnhancedUnavailable(
                        "Jev decision provider is not configured".into(),
                    )
                })?
            } else {
                self.normal_decider.as_ref()
            };
            let decision = decider.choose(input).await?;
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.id == decision.candidate_id)
                .cloned()
                .ok_or_else(|| AutomationError::UnknownCandidate(decision.candidate_id.clone()))?;
            trace.push(TraceEvent {
                kind: TraceEventKind::DecisionMade,
                observation_revision: Some(observation.revision),
                candidate_id: Some(candidate.id.clone()),
                detail: decision
                    .actual_model
                    .unwrap_or_else(|| "local decision provider".into()),
            });

            match candidate.kind {
                CandidateKind::Done => {
                    trace.push(stop_event("goal complete"));
                    return Ok(RunOutcome {
                        status: RunStatus::Completed,
                        executed_steps,
                        trace,
                    });
                }
                CandidateKind::AskUser => {
                    trace.push(stop_event("decision provider requested user input"));
                    return Ok(RunOutcome {
                        status: RunStatus::WaitingForUser,
                        executed_steps,
                        trace,
                    });
                }
                CandidateKind::CannotProceed => {
                    trace.push(stop_event("no offered action can proceed"));
                    return Ok(RunOutcome {
                        status: RunStatus::FailedSafely,
                        executed_steps,
                        trace,
                    });
                }
                _ => {}
            }

            match self
                .policy
                .evaluate(&observation, &candidate, &request.grant)
            {
                PolicyDecision::Allow => trace.push(TraceEvent {
                    kind: TraceEventKind::PolicyAllowed,
                    observation_revision: Some(observation.revision),
                    candidate_id: Some(candidate.id.clone()),
                    detail: "allowed by local policy and execution grant".into(),
                }),
                PolicyDecision::NeedsIncrementalGrant(reason)
                | PolicyDecision::NeedsConfirmation(reason) => {
                    trace.push(stop_event(&reason));
                    return Ok(RunOutcome {
                        status: RunStatus::WaitingForUser,
                        executed_steps,
                        trace,
                    });
                }
                PolicyDecision::Deny(reason) => {
                    trace.push(stop_event(&reason));
                    return Ok(RunOutcome {
                        status: RunStatus::FailedSafely,
                        executed_steps,
                        trace,
                    });
                }
            }

            // Fresh re-observation prevents dispatching against a changed UI.
            let fresh = self.observer.observe(&request.scope).await?;
            if fresh.revision != candidate.observation_revision
                || fresh.fingerprint != observation.fingerprint
            {
                trace.push(TraceEvent {
                    kind: TraceEventKind::StaleObservation,
                    observation_revision: Some(fresh.revision),
                    candidate_id: Some(candidate.id.clone()),
                    detail: "UI changed before dispatch; rebuilding candidates".into(),
                });
                continue;
            }

            let receipt = self.executor.execute(&fresh, &candidate).await?;
            if receipt.candidate_id != candidate.id {
                return Err(AutomationError::Execution(
                    "executor receipt candidate id does not match dispatched candidate".into(),
                ));
            }
            executed_steps += 1;
            recent.push(candidate.id.clone());
            if recent.len() > 8 {
                recent.remove(0);
            }
            trace.push(TraceEvent {
                kind: TraceEventKind::Executed,
                observation_revision: Some(fresh.revision),
                candidate_id: Some(candidate.id.clone()),
                detail: "one locally validated action dispatched".into(),
            });

            let after = self.observer.observe(&request.scope).await?;
            let mut verification = self
                .verifier
                .verify(&fresh, &candidate, &receipt, &after)
                .await?;
            if verification == Verification::Unknown {
                let reconciled = self.observer.observe(&request.scope).await?;
                verification = self
                    .verifier
                    .verify(&fresh, &candidate, &receipt, &reconciled)
                    .await?;
                trace.push(TraceEvent {
                    kind: TraceEventKind::Reconciled,
                    observation_revision: Some(reconciled.revision),
                    candidate_id: Some(candidate.id.clone()),
                    detail: format!("reconciliation result: {verification:?}"),
                });
            }
            trace.push(TraceEvent {
                kind: TraceEventKind::Verified,
                observation_revision: Some(after.revision),
                candidate_id: Some(candidate.id.clone()),
                detail: format!("{verification:?}"),
            });

            match verification {
                Verification::Achieved | Verification::Progress => stall_count = 0,
                Verification::NoChange => {
                    stall_count += 1;
                    if stall_count >= self.limits.max_stall_count {
                        trace.push(stop_event("repeated action produced no relevant progress"));
                        return Ok(RunOutcome {
                            status: RunStatus::NeedsAttention,
                            executed_steps,
                            trace,
                        });
                    }
                }
                Verification::Unexpected | Verification::Unknown => {
                    trace.push(stop_event(
                        "outcome requires attention; non-idempotent action was not replayed",
                    ));
                    return Ok(RunOutcome {
                        status: RunStatus::NeedsAttention,
                        executed_steps,
                        trace,
                    });
                }
            }
        }
    }
}

fn validate_candidates(
    observation: &Observation,
    candidates: &[ActionCandidate],
) -> Result<(), AutomationError> {
    if candidates.is_empty() {
        return Err(AutomationError::Candidates("candidate set is empty".into()));
    }
    if candidates.len() > 255 {
        return Err(AutomationError::Candidates(
            "candidate set exceeds the System One Choice hard limit".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for candidate in candidates {
        if candidate.id.trim().is_empty() || !ids.insert(candidate.id.as_str()) {
            return Err(AutomationError::Candidates(
                "candidate ids must be non-empty and unique".into(),
            ));
        }
        if candidate.observation_revision != observation.revision {
            return Err(AutomationError::Candidates(format!(
                "candidate '{}' is bound to a different observation revision",
                candidate.id
            )));
        }
    }
    Ok(())
}

fn stop_event(detail: &str) -> TraceEvent {
    TraceEvent {
        kind: TraceEventKind::Stopped,
        observation_revision: None,
        candidate_id: None,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    struct FakeObserver {
        observations: Mutex<Vec<Observation>>,
    }

    #[async_trait]
    impl ComputerObserver for FakeObserver {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                supported: vec![PlatformCapability::ReadSemanticTree],
                accessibility_permission: true,
            }
        }

        async fn observe(&self, _scope: &ObservationScope) -> Result<Observation, AutomationError> {
            let mut guard = self.observations.lock().await;
            if guard.len() > 1 {
                Ok(guard.remove(0))
            } else {
                Ok(guard[0].clone())
            }
        }
    }

    struct FakeBuilder;

    impl CandidateBuilder for FakeBuilder {
        fn build(
            &self,
            _goal: &str,
            observation: &Observation,
        ) -> Result<Vec<ActionCandidate>, AutomationError> {
            let kind = if observation.fingerprint == "done" {
                CandidateKind::Done
            } else {
                CandidateKind::Invoke
            };
            Ok(vec![ActionCandidate {
                id: if matches!(kind, CandidateKind::Done) {
                    "done".into()
                } else {
                    "invoke-save".into()
                },
                observation_revision: observation.revision,
                target: Some("save-button".into()),
                kind,
                public_description: "Save document".into(),
                local_risk: RiskClass::Reversible,
                preconditions: vec![],
                expected_effects: vec![],
            }])
        }
    }

    struct FirstDecision {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl DecisionProvider for FirstDecision {
        async fn choose(&self, input: DecisionInput) -> Result<Decision, AutomationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Decision {
                candidate_id: input.candidates[0].id.clone(),
                confidence: None,
                probabilities: Default::default(),
                actual_model: None,
            })
        }
    }

    struct FakeExecutor;

    #[async_trait]
    impl ComputerExecutor for FakeExecutor {
        async fn execute(
            &self,
            _fresh: &Observation,
            action: &ActionCandidate,
        ) -> Result<ActionReceipt, AutomationError> {
            Ok(ActionReceipt {
                candidate_id: action.id.clone(),
                dispatched: true,
                detail: None,
            })
        }
    }

    struct FakeVerifier;

    #[async_trait]
    impl Verifier for FakeVerifier {
        async fn verify(
            &self,
            _before: &Observation,
            _action: &ActionCandidate,
            _receipt: &ActionReceipt,
            after: &Observation,
        ) -> Result<Verification, AutomationError> {
            Ok(if after.fingerprint == "done" {
                Verification::Achieved
            } else {
                Verification::NoChange
            })
        }
    }

    fn observation(revision: u64, fingerprint: &str) -> Observation {
        Observation {
            revision,
            fingerprint: fingerprint.into(),
            app: AppIdentity {
                id: "editor".into(),
                display_name: "Editor".into(),
            },
            window: WindowIdentity {
                id: "main".into(),
                title: "Document".into(),
            },
            nodes: vec![],
            captured_at_ms: 0,
        }
    }

    fn grant() -> ExecutionGrant {
        ExecutionGrant {
            workflow_id: "wf".into(),
            workflow_version: "1".into(),
            capability_manifest_digest: "digest".into(),
            allowed_apps: vec!["editor".into()],
            action_classes: vec![ActionClass::NativeAction, ActionClass::Control],
            secret_slot_ids: vec![],
            visual_fallback: false,
            unattended: false,
            revoked: false,
        }
    }

    #[tokio::test]
    async fn fake_components_complete_the_bounded_loop() {
        let normal = Arc::new(FirstDecision {
            calls: AtomicUsize::new(0),
        });
        let observer = Arc::new(FakeObserver {
            observations: Mutex::new(vec![
                observation(1, "ready"),
                observation(1, "ready"),
                observation(2, "done"),
                observation(2, "done"),
            ]),
        });
        let runner = AutomationRunner::new(
            observer,
            Arc::new(FakeBuilder),
            normal,
            Arc::new(LocalPolicy),
            Arc::new(FakeExecutor),
            Arc::new(FakeVerifier),
            RunLimits::default(),
        );

        let outcome = runner
            .run(RunRequest {
                goal: "save".into(),
                scope: ObservationScope::default(),
                grant: grant(),
                enhanced_mode: false,
            })
            .await
            .unwrap();

        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(outcome.executed_steps, 1);
    }

    #[tokio::test]
    async fn enhanced_off_never_calls_jev_provider() {
        let normal = Arc::new(FirstDecision {
            calls: AtomicUsize::new(0),
        });
        let jev = Arc::new(FirstDecision {
            calls: AtomicUsize::new(0),
        });
        let observer = Arc::new(FakeObserver {
            observations: Mutex::new(vec![observation(1, "done")]),
        });
        let runner = AutomationRunner::new(
            observer,
            Arc::new(FakeBuilder),
            normal.clone(),
            Arc::new(LocalPolicy),
            Arc::new(FakeExecutor),
            Arc::new(FakeVerifier),
            RunLimits::default(),
        )
        .with_jev_decider(jev.clone());

        let outcome = runner
            .run(RunRequest {
                goal: "already done".into(),
                scope: ObservationScope::default(),
                grant: grant(),
                enhanced_mode: false,
            })
            .await
            .unwrap();

        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(normal.calls.load(Ordering::SeqCst), 1);
        assert_eq!(jev.calls.load(Ordering::SeqCst), 0);
    }
}
