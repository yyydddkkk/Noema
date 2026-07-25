use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Constraint, DesiredWorkloadState, EffectClass, HealthSpec, IntentSir, Mutation, SirVersion,
};

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_ARTIFACT_REF_BYTES: usize = 512;
const MAX_HTTP_PATH_BYTES: usize = 1_024;
const MAX_HEALTH_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    UnsupportedVersion,
    EmptyMutations,
    InvalidIdentifier,
    InvalidArtifactReference,
    DuplicateWorkloadCreation,
    ConflictingMutations,
    InvalidDesiredState,
    InvalidHealthSpec,
    UnknownWorkloadReference,
    InconsistentEffectPolicy,
    EffectBudgetExceeded,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationError {
    pub code: ValidationCode,
    pub path: String,
    pub message: String,
}

impl ValidationError {
    fn new(code: ValidationCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Default)]
struct MutationFlags {
    created: bool,
    state_set: bool,
    removed: bool,
}

/// Purely validates an Intent SIR envelope.
///
/// This function does not inspect live state and cannot produce an execution
/// plan. Generation freshness and resource feasibility belong to the planner.
///
/// # Errors
///
/// Returns every independently detectable validation error in stable input
/// order. A caller must not cross an effect boundary when validation fails.
pub fn validate_intent(intent: &IntentSir) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    validate_envelope(intent, &mut errors);
    let proposal = validate_mutations(intent, &mut errors);
    validate_constraints(intent, &proposal.addressed, &proposal.health, &mut errors);
    validate_effect_policy(intent, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_envelope(intent: &IntentSir, errors: &mut Vec<ValidationError>) {
    if intent.sir_version != SirVersion::V0 {
        errors.push(ValidationError::new(
            ValidationCode::UnsupportedVersion,
            "/sir_version",
            format!("SIR version {} is not supported", intent.sir_version.0),
        ));
    }

    validate_identifier(
        intent.proposal_id.as_str(),
        "/proposal_id",
        "proposal",
        errors,
    );

    if intent.mutations.is_empty() {
        errors.push(ValidationError::new(
            ValidationCode::EmptyMutations,
            "/mutations",
            "an intent must contain at least one mutation",
        ));
    }
}

struct ProposalSummary {
    addressed: BTreeSet<String>,
    health: BTreeMap<String, bool>,
}

fn validate_mutations(intent: &IntentSir, errors: &mut Vec<ValidationError>) -> ProposalSummary {
    let mut addressed = BTreeSet::new();
    let mut health = BTreeMap::new();
    let mut flags = BTreeMap::<String, MutationFlags>::new();

    for (index, mutation) in intent.mutations.iter().enumerate() {
        let base_path = format!("/mutations/{index}");
        let workload = mutation.workload();
        addressed.insert(workload.as_str().to_owned());
        let entry = flags.entry(workload.as_str().to_owned()).or_default();

        match mutation {
            Mutation::CreateWorkload {
                id,
                artifact,
                desired,
                health: health_spec,
                ..
            } => {
                validate_identifier(id.as_str(), &format!("{base_path}/id"), "workload", errors);
                validate_artifact_ref(artifact.as_str(), &format!("{base_path}/artifact"), errors);
                validate_create_state(*desired, &base_path, errors);
                validate_health(health_spec, &base_path, errors);

                if entry.created {
                    errors.push(ValidationError::new(
                        ValidationCode::DuplicateWorkloadCreation,
                        &base_path,
                        format!("workload '{id}' is created more than once"),
                    ));
                }
                if entry.removed {
                    push_conflict(id.as_str(), &base_path, errors);
                }
                entry.created = true;
                health.insert(
                    id.as_str().to_owned(),
                    !matches!(health_spec, HealthSpec::None),
                );
            }
            Mutation::SetDesiredState { workload, .. } => {
                validate_identifier(
                    workload.as_str(),
                    &format!("{base_path}/workload"),
                    "workload",
                    errors,
                );
                if entry.removed {
                    push_conflict(workload.as_str(), &base_path, errors);
                }
                entry.state_set = true;
            }
            Mutation::RemoveWorkload { workload } => {
                validate_identifier(
                    workload.as_str(),
                    &format!("{base_path}/workload"),
                    "workload",
                    errors,
                );
                if entry.created || entry.state_set || entry.removed {
                    push_conflict(workload.as_str(), &base_path, errors);
                }
                entry.removed = true;
            }
        }
    }

    ProposalSummary { addressed, health }
}

fn validate_constraints(
    intent: &IntentSir,
    addressed: &BTreeSet<String>,
    health: &BTreeMap<String, bool>,
    errors: &mut Vec<ValidationError>,
) {
    for (index, constraint) in intent.constraints.iter().enumerate() {
        let Constraint::MustPassHealthCheck { workload } = constraint else {
            continue;
        };
        let path = format!("/constraints/{index}/workload");
        validate_identifier(workload.as_str(), &path, "workload", errors);

        if !addressed.contains(workload.as_str()) {
            errors.push(ValidationError::new(
                ValidationCode::UnknownWorkloadReference,
                &path,
                format!(
                    "health constraint references workload '{workload}', which the proposal does not address"
                ),
            ));
        } else if health.get(workload.as_str()) == Some(&false) {
            errors.push(ValidationError::new(
                ValidationCode::InvalidHealthSpec,
                &path,
                format!("workload '{workload}' has no health check"),
            ));
        }
    }
}

fn validate_effect_policy(intent: &IntentSir, errors: &mut Vec<ValidationError>) {
    let policy = &intent.effect_policy;
    let maximum_is_irreversible = policy.maximum_effect == EffectClass::Irreversible;
    if policy.allow_irreversible != maximum_is_irreversible {
        errors.push(ValidationError::new(
            ValidationCode::InconsistentEffectPolicy,
            "/effect_policy",
            "allow_irreversible must be true exactly when maximum_effect is irreversible",
        ));
    }

    for (index, mutation) in intent.mutations.iter().enumerate() {
        let required = mutation.required_effect();
        if !policy.maximum_effect.permits(required) {
            errors.push(ValidationError::new(
                ValidationCode::EffectBudgetExceeded,
                format!("/mutations/{index}"),
                format!(
                    "mutation requires {required:?}, but the policy permits at most {:?}",
                    policy.maximum_effect
                ),
            ));
        }
    }
}

fn validate_create_state(
    desired: DesiredWorkloadState,
    base_path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if desired == DesiredWorkloadState::Absent {
        errors.push(ValidationError::new(
            ValidationCode::InvalidDesiredState,
            format!("{base_path}/desired"),
            "a newly created workload cannot have desired state absent",
        ));
    }
}

fn validate_health(health: &HealthSpec, base_path: &str, errors: &mut Vec<ValidationError>) {
    let HealthSpec::Http {
        port,
        path,
        timeout_ms,
    } = health
    else {
        return;
    };

    if *port == 0 {
        errors.push(ValidationError::new(
            ValidationCode::InvalidHealthSpec,
            format!("{base_path}/health/port"),
            "HTTP health-check port must be between 1 and 65535",
        ));
    }
    if !path.starts_with('/') || path.len() > MAX_HTTP_PATH_BYTES || path.contains(['\r', '\n']) {
        errors.push(ValidationError::new(
            ValidationCode::InvalidHealthSpec,
            format!("{base_path}/health/path"),
            "HTTP health-check path must start with '/', contain no newlines, and be at most 1024 bytes",
        ));
    }
    if !(1..=MAX_HEALTH_TIMEOUT_MS).contains(timeout_ms) {
        errors.push(ValidationError::new(
            ValidationCode::InvalidHealthSpec,
            format!("{base_path}/health/timeout_ms"),
            format!("health-check timeout must be between 1 and {MAX_HEALTH_TIMEOUT_MS} ms"),
        ));
    }
}

fn validate_identifier(value: &str, path: &str, kind: &str, errors: &mut Vec<ValidationError>) {
    let valid = !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid {
        errors.push(ValidationError::new(
            ValidationCode::InvalidIdentifier,
            path,
            format!(
                "{kind} identifier must be 1..={MAX_IDENTIFIER_BYTES} ASCII bytes using letters, digits, '.', '_', or '-'"
            ),
        ));
    }
}

fn validate_artifact_ref(value: &str, path: &str, errors: &mut Vec<ValidationError>) {
    let valid = !value.is_empty()
        && value.len() <= MAX_ARTIFACT_REF_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/' | b'@')
        });
    if !valid {
        errors.push(ValidationError::new(
            ValidationCode::InvalidArtifactReference,
            path,
            "artifact reference is empty, too long, or contains unsupported characters",
        ));
    }
}

fn push_conflict(workload: &str, path: &str, errors: &mut Vec<ValidationError>) {
    errors.push(ValidationError::new(
        ValidationCode::ConflictingMutations,
        path,
        format!("workload '{workload}' has conflicting mutations"),
    ));
}
