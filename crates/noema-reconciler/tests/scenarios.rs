use noema_executor::SimulationFault;
use noema_ir::{
    ArtifactRef, Constraint, DesiredWorkloadState, EffectClass, EffectPolicy, GenerationId,
    HealthSpec, IntentSir, Mutation, ObservedWorkloadState, ProposalId, RestartPolicy, SirVersion,
    WorkloadId,
};
use noema_planner::PlanError;
use noema_reconciler::{ReconcileError, Reconciler, TransactionStatus};
use noema_state::GenerationStore;

fn create_intent(base: GenerationId, proposal: &str) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from(proposal),
        base_generation: base,
        mutations: vec![Mutation::CreateWorkload {
            id: WorkloadId::from("hello"),
            artifact: ArtifactRef::from("builtin:noema-test-workload"),
            desired: DesiredWorkloadState::Running,
            health: HealthSpec::Process,
            restart_policy: RestartPolicy::OnFailure,
        }],
        constraints: vec![
            Constraint::MustPassHealthCheck {
                workload: WorkloadId::from("hello"),
            },
            Constraint::RollbackOnFailure,
        ],
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::LocallyReversible,
            allow_irreversible: false,
        },
    }
}

fn remove_intent(base: GenerationId) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from("remove-hello"),
        base_generation: base,
        mutations: vec![Mutation::RemoveWorkload {
            workload: WorkloadId::from("hello"),
        }],
        constraints: vec![Constraint::RollbackOnFailure],
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::LocallyReversible,
            allow_irreversible: false,
        },
    }
}

#[test]
fn workload_start_commits_state_runtime_and_evidence() {
    let mut reconciler = Reconciler::new();
    let outcome = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "normal-start"))
        .expect("transaction must execute");

    assert_eq!(outcome.status, TransactionStatus::Committed);
    assert_eq!(outcome.evidence.new_generation, Some(GenerationId(1)));
    assert!(
        outcome
            .evidence
            .invariant_results
            .iter()
            .all(|result| result.passed)
    );
    assert_eq!(
        reconciler
            .current()
            .workload(&WorkloadId::from("hello"))
            .expect("committed workload")
            .observed(),
        ObservedWorkloadState::Running
    );
    assert_eq!(
        reconciler.backend().observed(&WorkloadId::from("hello")),
        ObservedWorkloadState::Running
    );
}

#[test]
fn health_failure_abandons_both_candidate_snapshots() {
    let mut reconciler = Reconciler::new();
    reconciler.backend_mut().inject_fault(
        WorkloadId::from("hello"),
        SimulationFault::HealthCheckFailure,
    );
    let outcome = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "bad-health"))
        .expect("execution failure is evidence, not an API error");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert_eq!(outcome.evidence.new_generation, None);
    assert_eq!(reconciler.current().generation(), GenerationId::INITIAL);
    assert!(
        reconciler
            .current()
            .workload(&WorkloadId::from("hello"))
            .is_none()
    );
    assert_eq!(
        reconciler.backend().observed(&WorkloadId::from("hello")),
        ObservedWorkloadState::Absent
    );
    assert!(
        outcome
            .evidence
            .invariant_results
            .iter()
            .any(|result| !result.passed)
    );
}

#[test]
fn generation_id_is_not_reused_after_rollback() {
    let mut reconciler = Reconciler::new();
    let workload = WorkloadId::from("hello");
    reconciler
        .backend_mut()
        .inject_fault(workload.clone(), SimulationFault::HealthCheckFailure);
    let failed = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "first-attempt"))
        .expect("first attempt produces rollback evidence");
    assert_eq!(failed.status, TransactionStatus::RolledBack);
    reconciler.backend_mut().clear_fault(&workload);

    let succeeded = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "second-attempt"))
        .expect("second attempt must use the next allocated generation");

    assert_eq!(succeeded.status, TransactionStatus::Committed);
    assert_eq!(succeeded.evidence.new_generation, Some(GenerationId(2)));
}

#[test]
fn startup_timeout_rolls_back_and_records_starting_observation() {
    let mut reconciler = Reconciler::new();
    reconciler
        .backend_mut()
        .inject_fault(WorkloadId::from("hello"), SimulationFault::StartTimeout);
    let outcome = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "start-timeout"))
        .expect("timeout produces rollback evidence");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert!(
        outcome
            .evidence
            .observations
            .iter()
            .any(|observation| observation.observed == ObservedWorkloadState::Starting)
    );
    assert!(
        outcome
            .evidence
            .invariant_results
            .iter()
            .any(|result| !result.passed)
    );
    assert_eq!(reconciler.current().generation(), GenerationId::INITIAL);
}

#[test]
fn crash_on_start_rolls_back_with_failed_observation() {
    let mut reconciler = Reconciler::new();
    reconciler
        .backend_mut()
        .inject_fault(WorkloadId::from("hello"), SimulationFault::CrashOnStart);
    let outcome = reconciler
        .submit(&create_intent(GenerationId::INITIAL, "crash-on-start"))
        .expect("crash produces rollback evidence");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert!(
        outcome
            .evidence
            .observations
            .iter()
            .any(|observation| observation.observed == ObservedWorkloadState::Failed)
    );
    assert!(
        outcome
            .evidence
            .invariant_results
            .iter()
            .any(|result| !result.passed)
    );
}

#[test]
fn removal_purges_state_and_runtime_in_one_generation() {
    let mut reconciler = Reconciler::new();
    reconciler
        .submit(&create_intent(GenerationId::INITIAL, "before-remove"))
        .expect("create workload");
    let outcome = reconciler
        .submit(&remove_intent(GenerationId(1)))
        .expect("remove workload");

    assert_eq!(outcome.status, TransactionStatus::Committed);
    assert_eq!(outcome.evidence.new_generation, Some(GenerationId(2)));
    assert!(
        reconciler
            .current()
            .workload(&WorkloadId::from("hello"))
            .is_none()
    );
    assert_eq!(
        reconciler.backend().observed(&WorkloadId::from("hello")),
        ObservedWorkloadState::Absent
    );
}

#[test]
fn stale_intent_has_zero_state_side_effects() {
    let mut reconciler = Reconciler::new();
    let error = reconciler
        .submit(&create_intent(GenerationId(8), "stale"))
        .expect_err("stale intent must fail planning");

    assert!(matches!(
        error,
        ReconcileError::Plan(PlanError::StaleBase { .. })
    ));
    assert_eq!(reconciler.current().generation(), GenerationId::INITIAL);
    assert!(reconciler.store().events().is_empty());
}

#[test]
fn crash_is_observed_and_reconciler_restores_running_state() {
    let mut reconciler = Reconciler::new();
    reconciler
        .submit(&create_intent(GenerationId::INITIAL, "before-crash"))
        .expect("initial transaction");
    let workload = WorkloadId::from("hello");
    reconciler
        .backend_mut()
        .crash(&workload)
        .expect("crash virtual workload");

    let evidence = reconciler
        .reconcile_once()
        .expect("reconcile drift")
        .expect("drift must produce evidence");

    assert_eq!(evidence.old_generation, GenerationId(1));
    assert_eq!(evidence.new_generation, Some(GenerationId(2)));
    assert!(
        evidence
            .observations
            .iter()
            .any(|observation| observation.observed == ObservedWorkloadState::Failed)
    );
    assert!(
        evidence
            .observations
            .iter()
            .any(|observation| observation.observed == ObservedWorkloadState::Running)
    );
    assert_eq!(
        reconciler.current().workload(&workload).unwrap().observed(),
        ObservedWorkloadState::Running
    );
    assert!(reconciler.reconcile_once().expect("stable pass").is_none());
}
