use noema_executor::{ExecutionFailure, SimulationBackend, SimulationFault};
use noema_ir::{ArtifactRef, HealthSpec, ObservedWorkloadState, WorkloadId};

#[test]
fn virtual_workload_runs_and_reports_health() {
    let mut backend = SimulationBackend::default();
    let workload = WorkloadId::from("hello");
    let artifact = ArtifactRef::from("builtin:noema-test-workload");
    backend.resolve(&artifact).expect("resolve built-in");
    backend
        .prepare(&workload, &artifact, &HealthSpec::Process)
        .expect("prepare workload");
    assert_eq!(backend.start(&workload), Ok(ObservedWorkloadState::Running));
    assert!(backend.check_health(&workload));
}

#[test]
fn start_timeout_has_a_trusted_observation() {
    let mut backend = SimulationBackend::default();
    let workload = WorkloadId::from("hello");
    let artifact = ArtifactRef::from("builtin:noema-test-workload");
    backend.resolve(&artifact).expect("resolve built-in");
    backend
        .prepare(&workload, &artifact, &HealthSpec::Process)
        .expect("prepare workload");
    backend.inject_fault(workload.clone(), SimulationFault::StartTimeout);

    let failure = backend.start(&workload).expect_err("start must time out");
    assert_eq!(failure, ExecutionFailure::StartTimedOut(workload));
    assert_eq!(failure.observation(), Some(ObservedWorkloadState::Starting));
}

#[test]
fn simulator_never_resolves_external_artifacts() {
    let mut backend = SimulationBackend::default();
    let artifact = ArtifactRef::from("https://example.invalid/workload");
    assert_eq!(
        backend.resolve(&artifact),
        Err(ExecutionFailure::UnsupportedArtifact(artifact))
    );
}
