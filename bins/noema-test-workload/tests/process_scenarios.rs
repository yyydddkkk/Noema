use std::net::TcpListener;

use noema_executor::{ExecutionBackend, ProcessBackend};
use noema_ir::{
    ArtifactRef, Constraint, DesiredWorkloadState, EffectClass, EffectPolicy, GenerationId,
    HealthSpec, IntentSir, Mutation, ObservedWorkloadState, ProposalId, RestartPolicy, SirVersion,
    WorkloadId,
};
use noema_reconciler::{Reconciler, TransactionStatus};

const NORMAL: &str = "builtin:noema-test-workload";
const CRASH: &str = "builtin:noema-test-workload:crash";
const STARTUP_TIMEOUT: &str = "builtin:noema-test-workload:startup-timeout";
const UNHEALTHY: &str = "builtin:noema-test-workload:unhealthy";

fn backend() -> ProcessBackend {
    ProcessBackend::new(env!("CARGO_BIN_EXE_noema-test-workload"))
}

fn unused_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("allocate test port")
        .local_addr()
        .expect("read test address")
        .port()
}

fn create_intent(artifact: &str, port: u16, proposal: &str) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from(proposal),
        base_generation: GenerationId::INITIAL,
        mutations: vec![Mutation::CreateWorkload {
            id: WorkloadId::from("hello"),
            artifact: ArtifactRef::from(artifact),
            desired: DesiredWorkloadState::Running,
            health: HealthSpec::Http {
                port,
                path: "/health".to_owned(),
                timeout_ms: 500,
            },
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

#[test]
fn real_process_starts_and_passes_http_health_check() {
    let mut reconciler = Reconciler::with_backend(backend());
    let workload = WorkloadId::from("hello");
    let outcome = reconciler
        .submit(&create_intent(NORMAL, unused_port(), "real-start"))
        .expect("execute real process transaction");

    assert_eq!(outcome.status, TransactionStatus::Committed);
    assert_eq!(outcome.evidence.new_generation, Some(GenerationId(1)));
    assert!(reconciler.backend().process_id(&workload).is_some());
    assert_eq!(
        reconciler
            .backend_mut()
            .observed(&workload)
            .expect("observe process"),
        ObservedWorkloadState::Running
    );
}

#[test]
fn unhealthy_process_is_terminated_on_rollback() {
    let mut reconciler = Reconciler::with_backend(backend());
    let workload = WorkloadId::from("hello");
    let outcome = reconciler
        .submit(&create_intent(UNHEALTHY, unused_port(), "unhealthy"))
        .expect("health failure produces evidence");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert_eq!(outcome.evidence.new_generation, None);
    assert!(reconciler.backend().process_id(&workload).is_none());
    assert_eq!(
        reconciler
            .backend_mut()
            .observed(&workload)
            .expect("observe rolled back process"),
        ObservedWorkloadState::Absent
    );
}

#[test]
fn immediate_crash_is_observed_without_committing() {
    let mut reconciler = Reconciler::with_backend(backend());
    let outcome = reconciler
        .submit(&create_intent(CRASH, unused_port(), "immediate-crash"))
        .expect("crash produces rollback evidence");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert!(
        outcome
            .evidence
            .observations
            .iter()
            .any(|observation| observation.observed == ObservedWorkloadState::Failed)
    );
}

#[test]
fn startup_timeout_is_compensated() {
    let mut reconciler = Reconciler::with_backend(backend());
    let workload = WorkloadId::from("hello");
    let outcome = reconciler
        .submit(&create_intent(
            STARTUP_TIMEOUT,
            unused_port(),
            "startup-timeout",
        ))
        .expect("timeout produces rollback evidence");

    assert_eq!(outcome.status, TransactionStatus::RolledBack);
    assert!(reconciler.backend().process_id(&workload).is_none());
}

#[test]
fn reconciler_restarts_a_crashed_real_process() {
    let mut reconciler = Reconciler::with_backend(backend());
    let workload = WorkloadId::from("hello");
    reconciler
        .submit(&create_intent(NORMAL, unused_port(), "before-crash"))
        .expect("start process");
    let first_pid = reconciler
        .backend()
        .process_id(&workload)
        .expect("running process id");
    reconciler
        .backend_mut()
        .force_crash(&workload)
        .expect("force child crash");

    let evidence = reconciler
        .reconcile_once()
        .expect("reconcile process crash")
        .expect("crash must produce evidence");
    let second_pid = reconciler
        .backend()
        .process_id(&workload)
        .expect("restarted process id");

    assert_ne!(first_pid, second_pid);
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
}
