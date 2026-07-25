//! Transaction coordinator and desired-state reconciliation loop.

use std::{error::Error, fmt};

use noema_executor::{ExecutionFailure, SimulationBackend};
use noema_ir::{
    EvidenceIr, ExecutionAction, ExecutionIr, GenerationId, IntentSir, InvariantCheck,
    InvariantResult, Mutation, ObjectRef, Observation, ObservationSource, ObservedWorkloadState,
    RestartPolicy, StateChange, TransactionId, WorkloadId,
};
use noema_planner::{PlanError, plan};
use noema_state::{
    CandidateGeneration, GenerationStore, MemoryGenerationStore, StateError, WorldState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionStatus {
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionOutcome {
    pub plan: ExecutionIr,
    pub evidence: EvidenceIr,
    pub status: TransactionStatus,
}

#[derive(Debug)]
pub enum ReconcileError {
    Plan(PlanError),
    State(StateError),
    InvalidPlan(String),
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(formatter),
            Self::State(error) => error.fmt(formatter),
            Self::InvalidPlan(message) => write!(formatter, "invalid execution plan: {message}"),
        }
    }
}

impl Error for ReconcileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Plan(error) => Some(error),
            Self::State(error) => Some(error),
            Self::InvalidPlan(_) => None,
        }
    }
}

impl From<PlanError> for ReconcileError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<StateError> for ReconcileError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

/// Owns committed state and the matching committed execution backend.
pub struct Reconciler {
    store: MemoryGenerationStore,
    backend: SimulationBackend,
    next_reconcile_transaction: u64,
}

impl Reconciler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: MemoryGenerationStore::new(),
            backend: SimulationBackend::default(),
            next_reconcile_transaction: 1,
        }
    }

    #[must_use]
    pub fn current(&self) -> &WorldState {
        self.store.current()
    }

    #[must_use]
    pub const fn backend(&self) -> &SimulationBackend {
        &self.backend
    }

    pub const fn backend_mut(&mut self) -> &mut SimulationBackend {
        &mut self.backend
    }

    #[must_use]
    pub const fn store(&self) -> &MemoryGenerationStore {
        &self.store
    }

    /// Plans and executes one model-authored transaction in isolated state and
    /// runtime snapshots.
    ///
    /// Execution failures are returned as a successful `RolledBack` outcome
    /// with Evidence IR. Planning and internal consistency failures are errors.
    ///
    /// # Errors
    ///
    /// Returns an error if validation/planning fails or a generated plan is
    /// inconsistent with the state core.
    pub fn submit(&mut self, intent: &IntentSir) -> Result<TransactionOutcome, ReconcileError> {
        let execution = plan(intent, self.store.current())?;
        let old_generation = self.store.current().generation();
        let mut evidence = EvidenceBuilder::new(execution.transaction_id.clone(), old_generation);
        add_preflight_invariants(&mut evidence, &execution, self.store.current());

        let mut candidate = self
            .store
            .begin(execution.transaction_id.clone(), execution.base_generation)?;
        let mut candidate_backend = self.backend.clone();
        let mut failure = None;

        for step in &execution.steps {
            if matches!(step.action, ExecutionAction::CommitGeneration) {
                continue;
            }
            if let Err(error) = execute_action(
                &step.action,
                &mut candidate,
                &mut candidate_backend,
                &mut evidence,
            ) {
                failure = Some(error);
                break;
            }
        }

        if let Some(message) = failure {
            evidence.invariant(
                InvariantCheck::RecoveryGenerationRetained,
                true,
                Some(format!("generation {old_generation} retained: {message}")),
            );
            self.store.abort(candidate);
            return Ok(TransactionOutcome {
                plan: execution,
                evidence: evidence.finish(None),
                status: TransactionStatus::RolledBack,
            });
        }

        let candidate_id = candidate.id();
        let committed = self.store.commit(candidate)?;
        if committed != candidate_id {
            return Err(ReconcileError::InvalidPlan(format!(
                "committed generation {committed} differs from candidate {candidate_id}"
            )));
        }
        self.backend = candidate_backend;
        evidence.change(StateChange::GenerationCommitted {
            from: old_generation,
            to: committed,
        });
        evidence.invariant(
            InvariantCheck::RecoveryGenerationRetained,
            true,
            Some(format!("generation {old_generation} remains recoverable")),
        );
        Ok(TransactionOutcome {
            plan: execution,
            evidence: evidence.finish(Some(committed)),
            status: TransactionStatus::Committed,
        })
    }

    /// Performs one deterministic convergence pass after runtime drift.
    ///
    /// At most one workload is reconciled per call so each returned Evidence IR
    /// has a small, explicit causal scope.
    ///
    /// # Errors
    ///
    /// Returns an error if generation state cannot be updated consistently.
    pub fn reconcile_once(&mut self) -> Result<Option<EvidenceIr>, ReconcileError> {
        let drift = self
            .store
            .current()
            .workloads()
            .iter()
            .find_map(|(id, workload)| {
                let runtime_observed = self.backend.observed(id);
                let restartable = should_restart(workload, runtime_observed);
                (runtime_observed != workload.observed() || restartable)
                    .then(|| (id.clone(), workload.clone(), runtime_observed))
            });
        let Some((workload_id, workload, runtime_observed)) = drift else {
            return Ok(None);
        };

        let old_generation = self.store.current().generation();
        let transaction_id = TransactionId::from(format!(
            "reconcile-{}-{}",
            old_generation, self.next_reconcile_transaction
        ));
        self.next_reconcile_transaction += 1;
        let mut evidence = EvidenceBuilder::new(transaction_id.clone(), old_generation);
        let mut candidate = self.store.begin(transaction_id, old_generation)?;
        record_observation(
            &mut candidate,
            &mut evidence,
            &workload_id,
            runtime_observed,
            ObservationSource::ProcessMonitor,
        )?;

        if should_restart(&workload, runtime_observed) {
            if runtime_observed == ObservedWorkloadState::Absent {
                self.backend
                    .resolve(workload.artifact())
                    .map_err(|error| execution_as_invalid_plan(&error))?;
                self.backend
                    .prepare(&workload_id, workload.artifact(), workload.health())
                    .map_err(|error| execution_as_invalid_plan(&error))?;
            }
            match self.backend.start(&workload_id) {
                Ok(observed) => {
                    record_observation(
                        &mut candidate,
                        &mut evidence,
                        &workload_id,
                        observed,
                        ObservationSource::Recovery,
                    )?;
                    if !matches!(workload.health(), noema_ir::HealthSpec::None) {
                        let healthy = self.backend.check_health(&workload_id);
                        evidence.invariant(
                            InvariantCheck::WorkloadHealthy {
                                workload: workload_id.clone(),
                            },
                            healthy,
                            Some(if healthy {
                                "recovered workload passed its health check".to_owned()
                            } else {
                                "recovered workload failed its health check".to_owned()
                            }),
                        );
                        if !healthy {
                            self.backend
                                .mark_failed(&workload_id)
                                .map_err(|error| execution_as_invalid_plan(&error))?;
                            record_observation(
                                &mut candidate,
                                &mut evidence,
                                &workload_id,
                                ObservedWorkloadState::Failed,
                                ObservationSource::HealthProbe,
                            )?;
                        }
                    }
                }
                Err(error) => {
                    if let Some(observed) = error.observation() {
                        record_observation(
                            &mut candidate,
                            &mut evidence,
                            &workload_id,
                            observed,
                            ObservationSource::Recovery,
                        )?;
                    }
                }
            }
        }

        let committed = self.store.commit(candidate)?;
        evidence.change(StateChange::GenerationCommitted {
            from: old_generation,
            to: committed,
        });
        Ok(Some(evidence.finish(Some(committed))))
    }
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new()
    }
}

fn should_restart(workload: &noema_state::Workload, observed: ObservedWorkloadState) -> bool {
    workload.desired() == noema_ir::DesiredWorkloadState::Running
        && match workload.restart_policy() {
            RestartPolicy::Never => false,
            RestartPolicy::OnFailure => observed == ObservedWorkloadState::Failed,
            RestartPolicy::Always => observed != ObservedWorkloadState::Running,
        }
}

fn add_preflight_invariants(
    evidence: &mut EvidenceBuilder,
    execution: &ExecutionIr,
    current: &WorldState,
) {
    for invariant in &execution.invariants {
        match invariant {
            InvariantCheck::BaseGenerationIsCurrent { expected } => evidence.invariant(
                invariant.clone(),
                current.generation() == *expected,
                Some(format!("current generation is {}", current.generation())),
            ),
            InvariantCheck::WorkloadDoesNotExist { workload } => evidence.invariant(
                invariant.clone(),
                current.workload(workload).is_none(),
                Some(format!("checked workload '{workload}' before mutation")),
            ),
            InvariantCheck::ArtifactResolved { .. }
            | InvariantCheck::WorkloadHealthy { .. }
            | InvariantCheck::RecoveryGenerationRetained => {}
        }
    }
}

fn execute_action(
    action: &ExecutionAction,
    candidate: &mut CandidateGeneration,
    backend: &mut SimulationBackend,
    evidence: &mut EvidenceBuilder,
) -> Result<(), String> {
    match action {
        ExecutionAction::CreateCandidateGeneration | ExecutionAction::CommitGeneration => {}
        ExecutionAction::ResolveArtifact { artifact } => {
            if let Err(error) = backend.resolve(artifact) {
                evidence.invariant(
                    InvariantCheck::ArtifactResolved {
                        artifact: artifact.clone(),
                    },
                    false,
                    Some(error.to_string()),
                );
                return Err(error.to_string());
            }
            evidence.artifacts.push(artifact.clone());
            evidence.invariant(
                InvariantCheck::ArtifactResolved {
                    artifact: artifact.clone(),
                },
                true,
                Some("simulation backend resolved built-in artifact".to_owned()),
            );
        }
        ExecutionAction::ApplyMutation { mutation } => {
            record_desired_change(candidate, evidence, mutation);
            candidate
                .apply_mutation(mutation)
                .map_err(|error| error.to_string())?;
        }
        ExecutionAction::PrepareWorkload { workload } => {
            prepare_workload(candidate, backend, evidence, workload)?;
        }
        ExecutionAction::StartWorkload { workload } => {
            start_workload(candidate, backend, evidence, workload)?;
        }
        ExecutionAction::StopWorkload { workload } => {
            let observed = backend.stop(workload);
            record_executor_observation(candidate, evidence, workload, observed)?;
        }
        ExecutionAction::RemoveWorkload { workload } => {
            remove_workload(candidate, backend, evidence, workload)?;
        }
        ExecutionAction::CheckHealth { workload } => {
            check_health(backend, evidence, workload)?;
        }
        ExecutionAction::AbandonGeneration => {
            return Err("happy-path plan unexpectedly abandons its candidate".to_owned());
        }
    }
    Ok(())
}

fn prepare_workload(
    candidate: &mut CandidateGeneration,
    backend: &mut SimulationBackend,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
) -> Result<(), String> {
    let spec = candidate
        .state()
        .workload(workload)
        .ok_or_else(|| format!("workload '{workload}' missing from candidate"))?;
    let artifact = spec.artifact().clone();
    let health = spec.health().clone();
    let observed = backend
        .prepare(workload, &artifact, &health)
        .map_err(|error| error.to_string())?;
    record_executor_observation(candidate, evidence, workload, observed)
}

fn start_workload(
    candidate: &mut CandidateGeneration,
    backend: &mut SimulationBackend,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
) -> Result<(), String> {
    match backend.start(workload) {
        Ok(observed) => record_executor_observation(candidate, evidence, workload, observed),
        Err(error) => {
            if let Some(observed) = error.observation() {
                record_executor_observation(candidate, evidence, workload, observed)?;
            }
            evidence.invariant(
                InvariantCheck::WorkloadHealthy {
                    workload: workload.clone(),
                },
                false,
                Some(error.to_string()),
            );
            Err(error.to_string())
        }
    }
}

fn remove_workload(
    candidate: &mut CandidateGeneration,
    backend: &mut SimulationBackend,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
) -> Result<(), String> {
    let observed = backend.remove(workload);
    record_executor_observation(candidate, evidence, workload, observed)?;
    candidate
        .purge_workload(workload)
        .map_err(|error| error.to_string())
}

fn check_health(
    backend: &SimulationBackend,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
) -> Result<(), String> {
    let healthy = backend.check_health(workload);
    evidence.invariant(
        InvariantCheck::WorkloadHealthy {
            workload: workload.clone(),
        },
        healthy,
        Some(if healthy {
            "simulation health probe passed".to_owned()
        } else {
            "simulation health probe failed".to_owned()
        }),
    );
    if healthy {
        Ok(())
    } else {
        Err(format!("workload '{workload}' failed its health check"))
    }
}

fn record_executor_observation(
    candidate: &mut CandidateGeneration,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
    observed: ObservedWorkloadState,
) -> Result<(), String> {
    record_observation(
        candidate,
        evidence,
        workload,
        observed,
        ObservationSource::Executor,
    )
    .map_err(|error| error.to_string())
}

fn record_desired_change(
    candidate: &CandidateGeneration,
    evidence: &mut EvidenceBuilder,
    mutation: &Mutation,
) {
    match mutation {
        Mutation::CreateWorkload { id, desired, .. } => {
            evidence.change(StateChange::WorkloadDesired {
                workload: id.clone(),
                from: None,
                to: *desired,
            });
        }
        Mutation::SetDesiredState { workload, state } => {
            let from = candidate
                .state()
                .workload(workload)
                .map(noema_state::Workload::desired);
            evidence.change(StateChange::WorkloadDesired {
                workload: workload.clone(),
                from,
                to: *state,
            });
        }
        Mutation::RemoveWorkload { workload } => {
            let from = candidate
                .state()
                .workload(workload)
                .map(noema_state::Workload::desired);
            evidence.change(StateChange::WorkloadDesired {
                workload: workload.clone(),
                from,
                to: noema_ir::DesiredWorkloadState::Absent,
            });
        }
    }
}

fn record_observation(
    candidate: &mut CandidateGeneration,
    evidence: &mut EvidenceBuilder,
    workload: &WorkloadId,
    observed: ObservedWorkloadState,
    source: ObservationSource,
) -> Result<(), StateError> {
    let previous = candidate.state().workload(workload).map_or(
        ObservedWorkloadState::Absent,
        noema_state::Workload::observed,
    );
    candidate.observe_workload(workload, observed)?;
    evidence.observe(workload.clone(), observed, source);
    if previous != observed {
        evidence.change(StateChange::WorkloadObserved {
            workload: workload.clone(),
            from: Some(previous),
            to: observed,
        });
    }
    Ok(())
}

fn execution_as_invalid_plan(error: &ExecutionFailure) -> ReconcileError {
    ReconcileError::InvalidPlan(error.to_string())
}

struct EvidenceBuilder {
    transaction_id: TransactionId,
    old_generation: GenerationId,
    next_observation: u64,
    observations: Vec<Observation>,
    state_changes: Vec<StateChange>,
    invariant_results: Vec<InvariantResult>,
    artifacts: Vec<noema_ir::ArtifactRef>,
}

impl EvidenceBuilder {
    const fn new(transaction_id: TransactionId, old_generation: GenerationId) -> Self {
        Self {
            transaction_id,
            old_generation,
            next_observation: 1,
            observations: Vec::new(),
            state_changes: Vec::new(),
            invariant_results: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    fn observe(
        &mut self,
        workload: WorkloadId,
        observed: ObservedWorkloadState,
        source: ObservationSource,
    ) {
        self.observations.push(Observation {
            sequence: self.next_observation,
            object: ObjectRef::Workload { id: workload },
            observed,
            source,
        });
        self.next_observation += 1;
    }

    fn change(&mut self, change: StateChange) {
        self.state_changes.push(change);
    }

    fn invariant(&mut self, invariant: InvariantCheck, passed: bool, evidence: Option<String>) {
        self.invariant_results.push(InvariantResult {
            invariant,
            passed,
            evidence,
        });
    }

    fn finish(self, new_generation: Option<GenerationId>) -> EvidenceIr {
        EvidenceIr {
            transaction_id: self.transaction_id,
            old_generation: self.old_generation,
            new_generation,
            observations: self.observations,
            state_changes: self.state_changes,
            invariant_results: self.invariant_results,
            artifacts: self.artifacts,
        }
    }
}
