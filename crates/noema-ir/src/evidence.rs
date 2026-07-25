use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactRef, DesiredWorkloadState, GenerationId, InvariantCheck, ObjectRef,
    ObservedWorkloadState, TransactionId, WorkloadId,
};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIr {
    pub transaction_id: TransactionId,
    pub old_generation: GenerationId,
    pub new_generation: Option<GenerationId>,
    pub observations: Vec<Observation>,
    pub state_changes: Vec<StateChange>,
    pub invariant_results: Vec<InvariantResult>,
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub sequence: u64,
    pub object: ObjectRef,
    pub observed: ObservedWorkloadState,
    pub source: ObservationSource,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Executor,
    ProcessMonitor,
    HealthProbe,
    Recovery,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateChange {
    WorkloadDesired {
        workload: WorkloadId,
        from: Option<DesiredWorkloadState>,
        to: DesiredWorkloadState,
    },
    WorkloadObserved {
        workload: WorkloadId,
        from: Option<ObservedWorkloadState>,
        to: ObservedWorkloadState,
    },
    GenerationCommitted {
        from: GenerationId,
        to: GenerationId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantResult {
    pub invariant: InvariantCheck,
    pub passed: bool,
    pub evidence: Option<String>,
}
