use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ArtifactRef, GenerationId, Mutation, StepId, TransactionId, WorkloadId};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIr {
    pub transaction_id: TransactionId,
    pub base_generation: GenerationId,
    pub steps: Vec<ExecutionStep>,
    pub invariants: Vec<InvariantCheck>,
    pub failure_policy: FailurePolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionStep {
    pub id: StepId,
    pub depends_on: Vec<StepId>,
    pub action: ExecutionAction,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionAction {
    ResolveArtifact { artifact: ArtifactRef },
    CreateCandidateGeneration,
    ApplyMutation { mutation: Mutation },
    PrepareWorkload { workload: WorkloadId },
    StartWorkload { workload: WorkloadId },
    StopWorkload { workload: WorkloadId },
    RemoveWorkload { workload: WorkloadId },
    CheckHealth { workload: WorkloadId },
    CommitGeneration,
    AbandonGeneration,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InvariantCheck {
    BaseGenerationIsCurrent { expected: GenerationId },
    WorkloadDoesNotExist { workload: WorkloadId },
    ArtifactResolved { artifact: ArtifactRef },
    WorkloadHealthy { workload: WorkloadId },
    RecoveryGenerationRetained,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    PreserveCurrentGeneration,
    AbandonCandidate,
}
