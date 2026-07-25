use noema_ir::{ArtifactRef, HealthSpec, ObservedWorkloadState, WorkloadId};

use crate::ExecutionFailure;

/// Trusted local runtime boundary used by the Reconciler.
///
/// Implementations stage mutations between `begin_transaction` and either
/// `commit_transaction` or `rollback_transaction`. Model-authored data never
/// receives a backend handle and cannot bypass typed operations.
pub trait ExecutionBackend {
    /// Starts an isolated or compensatable runtime transaction.
    ///
    /// # Errors
    ///
    /// Returns an error when another transaction is already active.
    fn begin_transaction(&mut self) -> Result<(), ExecutionFailure>;

    /// Makes the active runtime transaction authoritative.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no active transaction.
    fn commit_transaction(&mut self) -> Result<(), ExecutionFailure>;

    /// Compensates all effects performed by the active transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if compensation cannot restore the prior runtime.
    fn rollback_transaction(&mut self) -> Result<(), ExecutionFailure>;

    /// Resolves an artifact without executing it.
    ///
    /// # Errors
    ///
    /// Returns an error when the reference is unsupported or unavailable.
    fn resolve(&mut self, artifact: &ArtifactRef) -> Result<(), ExecutionFailure>;

    /// Prepares a workload definition without starting it.
    ///
    /// # Errors
    ///
    /// Returns an error when its artifact has not been resolved.
    fn prepare(
        &mut self,
        workload: &WorkloadId,
        artifact: &ArtifactRef,
        health: &HealthSpec,
    ) -> Result<ObservedWorkloadState, ExecutionFailure>;

    /// Starts a prepared workload.
    ///
    /// # Errors
    ///
    /// Returns an error if the workload is missing or fails during startup.
    fn start(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure>;

    /// Stops a workload using the backend's graceful termination policy.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime cannot terminate the workload.
    fn stop(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure>;

    /// Removes all prepared runtime state for a workload.
    ///
    /// # Errors
    ///
    /// Returns an error if a running workload cannot first be terminated.
    fn remove(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure>;

    /// Runs the workload's locally-defined health probe.
    ///
    /// # Errors
    ///
    /// Returns an error if the workload is unknown or the probe cannot run.
    fn check_health(&mut self, workload: &WorkloadId) -> Result<bool, ExecutionFailure>;

    /// Reads the backend's trusted runtime observation.
    ///
    /// # Errors
    ///
    /// Returns an error if runtime state cannot be inspected.
    fn observed(
        &mut self,
        workload: &WorkloadId,
    ) -> Result<ObservedWorkloadState, ExecutionFailure>;

    /// Records a failed health result in the runtime backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the workload is unknown.
    fn mark_failed(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure>;

    /// Human-readable backend name for locally-authored evidence.
    fn name(&self) -> &'static str;
}
