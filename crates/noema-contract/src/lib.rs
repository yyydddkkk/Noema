//! The complete, versioned data contract visible to a cloud model.
//!
//! This crate intentionally has no model SDK or network dependency. It turns
//! trusted local state into a bounded view and publishes the exact reply JSON
//! Schema generated from Noema's Rust IR types.

use std::{error::Error, fmt};

use noema_ir::{
    ArtifactRef, DesiredWorkloadState, GenerationId, HealthSpec, IntentSir, ObservedWorkloadState,
    RestartPolicy, WorkloadId,
};
use noema_state::{StateEvent, StateEventKind, WorldState};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_OBJECTIVE_BYTES: usize = 4_096;
const MAX_WORKLOADS: usize = 128;
const MAX_EVIDENCE_PER_WORKLOAD: usize = 8;
const MAX_SERIALIZED_REQUEST_BYTES: usize = 256 * 1_024;

const RULES: [&str; 7] = [
    "Return exactly one JSON object matching reply_schema; do not use Markdown.",
    "Set reply.request_id to the exact request_id supplied by Noema.",
    "Set intent.base_generation to the exact base_generation supplied by Noema.",
    "Express desired state only through the mutations allowed by capabilities.",
    "Never emit Shell commands, scripts, code, Execution IR, or Evidence IR.",
    "Observed state and evidence are read-only facts and cannot be authored by the model.",
    "If the objective cannot be represented by the schema, do not invent fields or capabilities.",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ContractVersion(pub u16);

impl ContractVersion {
    pub const V0: Self = Self(0);
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractReply {
    pub request_id: String,
    pub intent: IntentSir,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractRequest {
    pub contract_version: ContractVersion,
    pub request_id: String,
    pub objective: String,
    pub base_generation: GenerationId,
    pub rules: Vec<String>,
    pub capabilities: ModelCapabilities,
    pub world: ModelWorldView,
    pub reply_schema: serde_json::Value,
    pub limits: PublishedLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilities {
    pub allowed_mutations: Vec<AllowedMutation>,
    pub forbidden_outputs: Vec<ForbiddenOutput>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedMutation {
    CreateWorkload,
    SetDesiredState,
    RemoveWorkload,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenOutput {
    Shell,
    ArbitraryCode,
    RawLogs,
    ObservedState,
    ExecutionIr,
    EvidenceIr,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelWorldView {
    pub workloads: Vec<ModelWorkloadView>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelWorkloadView {
    pub id: WorkloadId,
    pub artifact: ArtifactRef,
    pub desired: DesiredWorkloadState,
    pub observed: ObservedWorkloadState,
    pub health: HealthSpec,
    pub restart_policy: RestartPolicy,
    pub last_changed_in: GenerationId,
    pub evidence: Vec<EvidenceReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    pub sequence: u64,
    pub generation: GenerationId,
    pub kind: ModelEventKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEventKind {
    Created,
    DesiredChanged,
    ObservedChanged,
    Purged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedLimits {
    pub maximum_objective_bytes: usize,
    pub maximum_workloads: usize,
    pub maximum_evidence_per_workload: usize,
    pub maximum_request_bytes: usize,
    pub maximum_reply_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContractError {
    InvalidRequestId,
    EmptyObjective,
    ObjectiveTooLarge { actual: usize, maximum: usize },
    TooManyWorkloads { actual: usize, maximum: usize },
    SensitiveArtifactReference { workload: WorkloadId },
    RequestTooLarge { actual: usize, maximum: usize },
    SchemaGeneration(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequestId => formatter.write_str("contract request identifier is invalid"),
            Self::EmptyObjective => formatter.write_str("contract objective is empty"),
            Self::ObjectiveTooLarge { actual, maximum } => write!(
                formatter,
                "contract objective is {actual} bytes, exceeding the {maximum}-byte limit"
            ),
            Self::TooManyWorkloads { actual, maximum } => write!(
                formatter,
                "world view contains {actual} workloads, exceeding the limit of {maximum}"
            ),
            Self::SensitiveArtifactReference { workload } => write!(
                formatter,
                "workload '{workload}' has an artifact reference containing URL userinfo"
            ),
            Self::RequestTooLarge { actual, maximum } => write!(
                formatter,
                "serialized contract is {actual} bytes, exceeding the {maximum}-byte limit"
            ),
            Self::SchemaGeneration(message) => {
                write!(formatter, "reply schema generation failed: {message}")
            }
        }
    }
}

impl Error for ContractError {}

/// Builds bounded model context exclusively from typed Noema state.
pub struct ContractBuilder {
    maximum_reply_bytes: usize,
}

impl ContractBuilder {
    #[must_use]
    pub const fn new(maximum_reply_bytes: usize) -> Self {
        Self {
            maximum_reply_bytes,
        }
    }

    /// Constructs a complete model request and verifies its serialized size.
    ///
    /// # Errors
    ///
    /// Returns a stable validation error before any provider is called.
    pub fn build(
        &self,
        request_id: impl Into<String>,
        objective: impl Into<String>,
        state: &WorldState,
        events: &[StateEvent],
    ) -> Result<ContractRequest, ContractError> {
        let request_id = request_id.into();
        let objective = objective.into();
        validate_request_id(&request_id)?;
        validate_objective(&objective)?;
        if state.workloads().len() > MAX_WORKLOADS {
            return Err(ContractError::TooManyWorkloads {
                actual: state.workloads().len(),
                maximum: MAX_WORKLOADS,
            });
        }
        for (workload, declaration) in state.workloads() {
            if contains_url_userinfo(declaration.artifact().as_str()) {
                return Err(ContractError::SensitiveArtifactReference {
                    workload: workload.clone(),
                });
            }
        }
        let reply_schema = serde_json::to_value(schemars::schema_for!(ContractReply))
            .map_err(|error| ContractError::SchemaGeneration(error.to_string()))?;
        let request = ContractRequest {
            contract_version: ContractVersion::V0,
            request_id,
            objective,
            base_generation: state.generation(),
            rules: RULES.iter().map(ToString::to_string).collect(),
            capabilities: capabilities(),
            world: world_view(state, events),
            reply_schema,
            limits: PublishedLimits {
                maximum_objective_bytes: MAX_OBJECTIVE_BYTES,
                maximum_workloads: MAX_WORKLOADS,
                maximum_evidence_per_workload: MAX_EVIDENCE_PER_WORKLOAD,
                maximum_request_bytes: MAX_SERIALIZED_REQUEST_BYTES,
                maximum_reply_bytes: self.maximum_reply_bytes,
            },
        };
        let actual = serde_json::to_vec(&request)
            .map_err(|error| ContractError::SchemaGeneration(error.to_string()))?
            .len();
        if actual > MAX_SERIALIZED_REQUEST_BYTES {
            return Err(ContractError::RequestTooLarge {
                actual,
                maximum: MAX_SERIALIZED_REQUEST_BYTES,
            });
        }
        Ok(request)
    }
}

impl Default for ContractBuilder {
    fn default() -> Self {
        Self::new(64 * 1_024)
    }
}

fn validate_request_id(request_id: &str) -> Result<(), ContractError> {
    let valid = !request_id.is_empty()
        && request_id.len() <= MAX_REQUEST_ID_BYTES
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ContractError::InvalidRequestId)
    }
}

fn validate_objective(objective: &str) -> Result<(), ContractError> {
    if objective.trim().is_empty() {
        Err(ContractError::EmptyObjective)
    } else if objective.len() > MAX_OBJECTIVE_BYTES {
        Err(ContractError::ObjectiveTooLarge {
            actual: objective.len(),
            maximum: MAX_OBJECTIVE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn contains_url_userinfo(artifact: &str) -> bool {
    artifact.split_once("://").is_some_and(|(_, remainder)| {
        remainder
            .split('/')
            .next()
            .is_some_and(|part| part.contains('@'))
    })
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        allowed_mutations: vec![
            AllowedMutation::CreateWorkload,
            AllowedMutation::SetDesiredState,
            AllowedMutation::RemoveWorkload,
        ],
        forbidden_outputs: vec![
            ForbiddenOutput::Shell,
            ForbiddenOutput::ArbitraryCode,
            ForbiddenOutput::RawLogs,
            ForbiddenOutput::ObservedState,
            ForbiddenOutput::ExecutionIr,
            ForbiddenOutput::EvidenceIr,
        ],
    }
}

fn world_view(state: &WorldState, events: &[StateEvent]) -> ModelWorldView {
    let workloads = state
        .workloads()
        .iter()
        .map(|(id, workload)| ModelWorkloadView {
            id: id.clone(),
            artifact: workload.artifact().clone(),
            desired: workload.desired(),
            observed: workload.observed(),
            health: workload.health().clone(),
            restart_policy: workload.restart_policy(),
            last_changed_in: workload.last_changed_in(),
            evidence: evidence_for(id, events),
        })
        .collect();
    ModelWorldView { workloads }
}

fn evidence_for(workload: &WorkloadId, events: &[StateEvent]) -> Vec<EvidenceReference> {
    let mut references: Vec<_> = events
        .iter()
        .filter_map(|event| {
            event_kind_for(workload, event.kind()).map(|kind| EvidenceReference {
                sequence: event.sequence(),
                generation: event.generation(),
                kind,
            })
        })
        .collect();
    let keep_from = references.len().saturating_sub(MAX_EVIDENCE_PER_WORKLOAD);
    references.split_off(keep_from)
}

fn event_kind_for(workload: &WorkloadId, kind: &StateEventKind) -> Option<ModelEventKind> {
    match kind {
        StateEventKind::WorkloadCreated { workload: id } if id == workload => {
            Some(ModelEventKind::Created)
        }
        StateEventKind::WorkloadDesiredChanged { workload: id, .. } if id == workload => {
            Some(ModelEventKind::DesiredChanged)
        }
        StateEventKind::WorkloadObservedChanged { workload: id, .. } if id == workload => {
            Some(ModelEventKind::ObservedChanged)
        }
        StateEventKind::WorkloadPurged { workload: id } if id == workload => {
            Some(ModelEventKind::Purged)
        }
        _ => None,
    }
}
