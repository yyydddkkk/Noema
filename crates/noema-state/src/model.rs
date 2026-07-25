use std::collections::BTreeMap;

use noema_ir::{
    ArtifactRef, DesiredWorkloadState, GenerationId, HealthSpec, ObservedWorkloadState,
    RestartPolicy, WorkloadId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Workload {
    pub(crate) artifact: ArtifactRef,
    pub(crate) desired: DesiredWorkloadState,
    pub(crate) observed: ObservedWorkloadState,
    pub(crate) health: HealthSpec,
    pub(crate) restart_policy: RestartPolicy,
    pub(crate) last_changed_in: GenerationId,
}

impl Workload {
    #[must_use]
    pub const fn artifact(&self) -> &ArtifactRef {
        &self.artifact
    }

    #[must_use]
    pub const fn desired(&self) -> DesiredWorkloadState {
        self.desired
    }

    #[must_use]
    pub const fn observed(&self) -> ObservedWorkloadState {
        self.observed
    }

    #[must_use]
    pub const fn health(&self) -> &HealthSpec {
        &self.health
    }

    #[must_use]
    pub const fn restart_policy(&self) -> RestartPolicy {
        self.restart_policy
    }

    #[must_use]
    pub const fn last_changed_in(&self) -> GenerationId {
        self.last_changed_in
    }

    pub(crate) fn new(
        artifact: ArtifactRef,
        desired: DesiredWorkloadState,
        health: HealthSpec,
        restart_policy: RestartPolicy,
        generation: GenerationId,
    ) -> Self {
        Self {
            artifact,
            desired,
            observed: ObservedWorkloadState::Absent,
            health,
            restart_policy,
            last_changed_in: generation,
        }
    }

    pub(crate) fn matches_declaration(
        &self,
        artifact: &ArtifactRef,
        desired: DesiredWorkloadState,
        health: &HealthSpec,
        restart_policy: RestartPolicy,
    ) -> bool {
        self.artifact == *artifact
            && self.desired == desired
            && self.health == *health
            && self.restart_policy == restart_policy
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorldState {
    generation: GenerationId,
    workloads: BTreeMap<WorkloadId, Workload>,
}

impl WorldState {
    #[must_use]
    pub fn initial() -> Self {
        Self {
            generation: GenerationId::INITIAL,
            workloads: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn generation(&self) -> GenerationId {
        self.generation
    }

    #[must_use]
    pub fn workload(&self, id: &WorkloadId) -> Option<&Workload> {
        self.workloads.get(id)
    }

    #[must_use]
    pub fn workloads(&self) -> &BTreeMap<WorkloadId, Workload> {
        &self.workloads
    }

    pub(crate) fn for_candidate(&self, generation: GenerationId) -> Self {
        let mut candidate = self.clone();
        candidate.generation = generation;
        candidate
    }

    pub(crate) fn workload_mut(&mut self, id: &WorkloadId) -> Option<&mut Workload> {
        self.workloads.get_mut(id)
    }

    pub(crate) fn insert_workload(&mut self, id: WorkloadId, workload: Workload) {
        self.workloads.insert(id, workload);
    }

    pub(crate) fn remove_workload(&mut self, id: &WorkloadId) {
        self.workloads.remove(id);
    }
}

impl Default for WorldState {
    fn default() -> Self {
        Self::initial()
    }
}
