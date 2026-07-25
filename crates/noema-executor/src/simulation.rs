use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use noema_ir::{ArtifactRef, HealthSpec, ObservedWorkloadState, WorkloadId};

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
}

impl ExecutionFailure {
    #[must_use]
    pub const fn observation(&self) -> Option<ObservedWorkloadState> {
        match self {
            Self::CrashedOnStart(_) => Some(ObservedWorkloadState::Failed),
            Self::StartTimedOut(_) => Some(ObservedWorkloadState::Starting),
            Self::ArtifactNotResolved(_)
            | Self::UnsupportedArtifact(_)
            | Self::WorkloadNotPrepared(_) => None,
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
                    "artifact '{artifact}' is not supported by the simulator"
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

/// Deterministic virtual runtime used by M2 scenario tests.
///
/// Cloning the backend creates a transaction-local runtime snapshot. The
/// reconciler executes against that snapshot and replaces the committed
/// backend only after every invariant passes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SimulationBackend {
    resolved: BTreeSet<ArtifactRef>,
    workloads: BTreeMap<WorkloadId, VirtualWorkload>,
    faults: BTreeMap<WorkloadId, SimulationFault>,
}

impl SimulationBackend {
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
        self.resolved.insert(artifact.clone());
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
        if !self.resolved.contains(artifact) {
            return Err(ExecutionFailure::ArtifactNotResolved(artifact.clone()));
        }
        self.workloads.insert(
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
        let entry = self
            .workloads
            .get_mut(workload)
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        match self.faults.get(workload) {
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
        if let Some(entry) = self.workloads.get_mut(workload) {
            entry.observed = ObservedWorkloadState::Stopped;
            entry.observed
        } else {
            ObservedWorkloadState::Absent
        }
    }

    /// Removes a workload from the virtual runtime.
    #[must_use]
    pub fn remove(&mut self, workload: &WorkloadId) -> ObservedWorkloadState {
        self.workloads.remove(workload);
        ObservedWorkloadState::Absent
    }

    #[must_use]
    pub fn check_health(&self, workload: &WorkloadId) -> bool {
        let Some(entry) = self.workloads.get(workload) else {
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
        self.workloads
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
