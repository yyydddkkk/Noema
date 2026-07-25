use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use noema_ir::{ArtifactRef, HealthSpec, ObservedWorkloadState, WorkloadId};

use crate::ExecutionBackend;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimulationFault {
    CrashOnStart,
    StartTimeout,
    HealthCheckFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionFailure {
    ArtifactNotResolved(ArtifactRef),
    UnsupportedArtifact(ArtifactRef),
    WorkloadNotPrepared(WorkloadId),
    CrashedOnStart(WorkloadId),
    StartTimedOut(WorkloadId),
    ProcessExited {
        workload: WorkloadId,
        code: Option<i32>,
    },
    TransactionAlreadyActive,
    NoActiveTransaction,
    Runtime {
        operation: &'static str,
        message: String,
    },
}

impl ExecutionFailure {
    #[must_use]
    pub const fn observation(&self) -> Option<ObservedWorkloadState> {
        match self {
            Self::CrashedOnStart(_) | Self::ProcessExited { .. } => {
                Some(ObservedWorkloadState::Failed)
            }
            Self::StartTimedOut(_) => Some(ObservedWorkloadState::Starting),
            Self::ArtifactNotResolved(_)
            | Self::UnsupportedArtifact(_)
            | Self::WorkloadNotPrepared(_)
            | Self::TransactionAlreadyActive
            | Self::NoActiveTransaction
            | Self::Runtime { .. } => None,
        }
    }

    #[must_use]
    pub fn runtime(operation: &'static str, error: impl fmt::Display) -> Self {
        Self::Runtime {
            operation,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtifactNotResolved(artifact) => {
                write!(formatter, "artifact '{artifact}' was not resolved")
            }
            Self::UnsupportedArtifact(artifact) => {
                write!(
                    formatter,
                    "artifact '{artifact}' is not supported by this backend"
                )
            }
            Self::WorkloadNotPrepared(workload) => {
                write!(formatter, "workload '{workload}' is not prepared")
            }
            Self::CrashedOnStart(workload) => {
                write!(formatter, "workload '{workload}' crashed during start")
            }
            Self::StartTimedOut(workload) => {
                write!(formatter, "workload '{workload}' did not finish starting")
            }
            Self::ProcessExited { workload, code } => {
                write!(formatter, "workload '{workload}' exited with code {code:?}")
            }
            Self::TransactionAlreadyActive => {
                formatter.write_str("an execution transaction is already active")
            }
            Self::NoActiveTransaction => formatter.write_str("no execution transaction is active"),
            Self::Runtime { operation, message } => {
                write!(
                    formatter,
                    "runtime operation '{operation}' failed: {message}"
                )
            }
        }
    }
}

impl Error for ExecutionFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VirtualWorkload {
    artifact: ArtifactRef,
    health: HealthSpec,
    observed: ObservedWorkloadState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SimulationState {
    resolved: BTreeSet<ArtifactRef>,
    workloads: BTreeMap<WorkloadId, VirtualWorkload>,
}

/// Deterministic virtual runtime used by M2 scenario tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SimulationBackend {
    committed: SimulationState,
    candidate: Option<SimulationState>,
    faults: BTreeMap<WorkloadId, SimulationFault>,
}

impl SimulationBackend {
    fn state(&self) -> &SimulationState {
        self.candidate.as_ref().unwrap_or(&self.committed)
    }

    fn state_mut(&mut self) -> &mut SimulationState {
        self.candidate.as_mut().unwrap_or(&mut self.committed)
    }

    /// Marks a built-in artifact as available.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionFailure::UnsupportedArtifact`] for non-built-in
    /// references; M2 deliberately has no network or package resolver.
    pub fn resolve(&mut self, artifact: &ArtifactRef) -> Result<(), ExecutionFailure> {
        if !artifact.as_str().starts_with("builtin:") {
            return Err(ExecutionFailure::UnsupportedArtifact(artifact.clone()));
        }
        self.state_mut().resolved.insert(artifact.clone());
        Ok(())
    }

    /// Creates or refreshes a virtual workload in stopped state.
    ///
    /// # Errors
    ///
    /// Returns an error until the artifact has been resolved.
    pub fn prepare(
        &mut self,
        workload: &WorkloadId,
        artifact: &ArtifactRef,
        health: &HealthSpec,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        if !self.state().resolved.contains(artifact) {
            return Err(ExecutionFailure::ArtifactNotResolved(artifact.clone()));
        }
        self.state_mut().workloads.insert(
            workload.clone(),
            VirtualWorkload {
                artifact: artifact.clone(),
                health: health.clone(),
                observed: ObservedWorkloadState::Stopped,
            },
        );
        Ok(ObservedWorkloadState::Stopped)
    }

    /// Starts a prepared virtual workload or returns an injected failure.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for an unknown workload, crash, or timeout.
    pub fn start(
        &mut self,
        workload: &WorkloadId,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        let fault = self.faults.get(workload).copied();
        let entry = self
            .state_mut()
            .workloads
            .get_mut(workload)
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        match fault {
            Some(SimulationFault::CrashOnStart) => {
                entry.observed = ObservedWorkloadState::Failed;
                Err(ExecutionFailure::CrashedOnStart(workload.clone()))
            }
            Some(SimulationFault::StartTimeout) => {
                entry.observed = ObservedWorkloadState::Starting;
                Err(ExecutionFailure::StartTimedOut(workload.clone()))
            }
            Some(SimulationFault::HealthCheckFailure) | None => {
                entry.observed = ObservedWorkloadState::Running;
                Ok(entry.observed)
            }
        }
    }

    /// Stops a workload. Stopping an absent workload is idempotent.
    #[must_use]
    pub fn stop(&mut self, workload: &WorkloadId) -> ObservedWorkloadState {
        if let Some(entry) = self.state_mut().workloads.get_mut(workload) {
            entry.observed = ObservedWorkloadState::Stopped;
            entry.observed
        } else {
            ObservedWorkloadState::Absent
        }
    }

    /// Removes a workload from the virtual runtime.
    #[must_use]
    pub fn remove(&mut self, workload: &WorkloadId) -> ObservedWorkloadState {
        self.state_mut().workloads.remove(workload);
        ObservedWorkloadState::Absent
    }

    #[must_use]
    pub fn check_health(&self, workload: &WorkloadId) -> bool {
        let Some(entry) = self.state().workloads.get(workload) else {
            return false;
        };
        if self.faults.get(workload) == Some(&SimulationFault::HealthCheckFailure) {
            return false;
        }
        match entry.health {
            HealthSpec::None => false,
            HealthSpec::Process | HealthSpec::Http { .. } => {
                entry.observed == ObservedWorkloadState::Running
            }
        }
    }

    #[must_use]
    pub fn observed(&self, workload: &WorkloadId) -> ObservedWorkloadState {
        self.state()
            .workloads
            .get(workload)
            .map_or(ObservedWorkloadState::Absent, |entry| entry.observed)
    }

    pub fn inject_fault(&mut self, workload: WorkloadId, fault: SimulationFault) {
        self.faults.insert(workload, fault);
    }

    pub fn clear_fault(&mut self, workload: &WorkloadId) {
        self.faults.remove(workload);
    }

    /// Marks a workload failed after a trusted health probe rejects it.
    ///
    /// # Errors
    ///
    /// Returns an error if no runtime workload exists.
    pub fn mark_failed(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        let entry = self
            .state_mut()
            .workloads
            .get_mut(workload)
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        entry.observed = ObservedWorkloadState::Failed;
        Ok(())
    }

    /// Simulates an out-of-band process crash after a successful commit.
    ///
    /// # Errors
    ///
    /// Returns an error if no runtime workload exists.
    pub fn crash(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        self.mark_failed(workload)
    }
}

impl ExecutionBackend for SimulationBackend {
    fn begin_transaction(&mut self) -> Result<(), ExecutionFailure> {
        if self.candidate.is_some() {
            return Err(ExecutionFailure::TransactionAlreadyActive);
        }
        self.candidate = Some(self.committed.clone());
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), ExecutionFailure> {
        self.committed = self
            .candidate
            .take()
            .ok_or(ExecutionFailure::NoActiveTransaction)?;
        Ok(())
    }

    fn rollback_transaction(&mut self) -> Result<(), ExecutionFailure> {
        self.candidate
            .take()
            .ok_or(ExecutionFailure::NoActiveTransaction)?;
        Ok(())
    }

    fn resolve(&mut self, artifact: &ArtifactRef) -> Result<(), ExecutionFailure> {
        Self::resolve(self, artifact)
    }

    fn prepare(
        &mut self,
        workload: &WorkloadId,
        artifact: &ArtifactRef,
        health: &HealthSpec,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        Self::prepare(self, workload, artifact, health)
    }

    fn start(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        Self::start(self, workload)
    }

    fn stop(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        Ok(Self::stop(self, workload))
    }

    fn remove(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        Ok(Self::remove(self, workload))
    }

    fn check_health(&mut self, workload: &WorkloadId) -> Result<bool, ExecutionFailure> {
        Ok(Self::check_health(self, workload))
    }

    fn observed(
        &mut self,
        workload: &WorkloadId,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        Ok(Self::observed(self, workload))
    }

    fn mark_failed(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        Self::mark_failed(self, workload)
    }

    fn name(&self) -> &'static str {
        "simulation"
    }
}
