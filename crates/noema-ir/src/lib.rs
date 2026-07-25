//! Versioned protocol types at Noema's trust boundaries.
//!
//! Cloud models may author [`IntentSir`]. Only trusted local components may
//! author [`ExecutionIr`] and [`EvidenceIr`].

mod evidence;
mod execution;
mod ids;
mod intent;
mod state;
mod validation;

pub use evidence::{EvidenceIr, InvariantResult, Observation, ObservationSource, StateChange};
pub use execution::{ExecutionAction, ExecutionIr, ExecutionStep, FailurePolicy, InvariantCheck};
pub use ids::{ArtifactRef, GenerationId, ProposalId, StepId, TransactionId, WorkloadId};
pub use intent::{Constraint, EffectClass, EffectPolicy, IntentSir, Mutation, SirVersion};
pub use state::{
    DesiredWorkloadState, HealthSpec, ObjectRef, ObservedWorkloadState, RestartPolicy,
};
pub use validation::{ValidationCode, ValidationError, validate_intent};
