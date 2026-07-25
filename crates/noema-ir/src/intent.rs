use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactRef, DesiredWorkloadState, GenerationId, HealthSpec, ProposalId, RestartPolicy,
    WorkloadId,
};

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct SirVersion(pub u16);

impl SirVersion {
    pub const V0: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    ReadOnly,
    LocallyReversible,
    Compensatable,
    Irreversible,
}

impl EffectClass {
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::LocallyReversible => 1,
            Self::Compensatable => 2,
            Self::Irreversible => 3,
        }
    }

    #[must_use]
    pub const fn permits(self, required: Self) -> bool {
        self.rank() >= required.rank()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectPolicy {
    pub maximum_effect: EffectClass,
    pub allow_irreversible: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mutation {
    CreateWorkload {
        id: WorkloadId,
        artifact: ArtifactRef,
        desired: DesiredWorkloadState,
        health: HealthSpec,
        restart_policy: RestartPolicy,
    },
    SetDesiredState {
        workload: WorkloadId,
        state: DesiredWorkloadState,
    },
    RemoveWorkload {
        workload: WorkloadId,
    },
}

impl Mutation {
    #[must_use]
    pub const fn required_effect(&self) -> EffectClass {
        EffectClass::LocallyReversible
    }

    #[must_use]
    pub fn workload(&self) -> &WorkloadId {
        match self {
            Self::CreateWorkload { id, .. } => id,
            Self::SetDesiredState { workload, .. } | Self::RemoveWorkload { workload } => workload,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Constraint {
    MustPassHealthCheck { workload: WorkloadId },
    RollbackOnFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntentSir {
    pub sir_version: SirVersion,
    pub proposal_id: ProposalId,
    pub base_generation: GenerationId,
    pub mutations: Vec<Mutation>,
    pub constraints: Vec<Constraint>,
    pub effect_policy: EffectPolicy,
}
