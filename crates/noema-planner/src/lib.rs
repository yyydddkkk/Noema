//! Pure, deterministic compilation from model-authored intent to execution IR.

use std::{error::Error, fmt};

use noema_ir::{
    Constraint, DesiredWorkloadState, ExecutionAction, ExecutionIr, ExecutionStep, FailurePolicy,
    GenerationId, IntentSir, InvariantCheck, Mutation, StepId, TransactionId, ValidationError,
    WorkloadId, validate_intent,
};
use noema_state::WorldState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    InvalidIntent(Vec<ValidationError>),
    StaleBase {
        current: GenerationId,
        requested: GenerationId,
    },
    WorkloadAlreadyExists(WorkloadId),
    WorkloadNotFound(WorkloadId),
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIntent(errors) => {
                write!(
                    formatter,
                    "intent failed validation with {} error(s)",
                    errors.len()
                )
            }
            Self::StaleBase { current, requested } => write!(
                formatter,
                "intent is based on generation {requested}, but current is {current}"
            ),
            Self::WorkloadAlreadyExists(workload) => {
                write!(formatter, "workload '{workload}' already exists")
            }
            Self::WorkloadNotFound(workload) => {
                write!(formatter, "workload '{workload}' does not exist")
            }
        }
    }
}

impl Error for PlanError {}

/// Compiles a validated Intent SIR against a read-only world-state snapshot.
///
/// The planner performs no I/O and does not mutate `current`. The same intent
/// and snapshot always produce an identical plan.
///
/// # Errors
///
/// Returns a validation or live-state error before producing any steps.
pub fn plan(intent: &IntentSir, current: &WorldState) -> Result<ExecutionIr, PlanError> {
    validate_intent(intent).map_err(PlanError::InvalidIntent)?;
    if intent.base_generation != current.generation() {
        return Err(PlanError::StaleBase {
            current: current.generation(),
            requested: intent.base_generation,
        });
    }
    validate_live_references(intent, current)?;

    let mut builder = PlanBuilder::new();
    let candidate_step = builder.push(Vec::new(), ExecutionAction::CreateCandidateGeneration);

    for mutation in &intent.mutations {
        compile_mutation(&mut builder, candidate_step, mutation, &intent.constraints);
    }

    let terminal_steps = builder.terminal_steps();
    builder.push(terminal_steps, ExecutionAction::CommitGeneration);

    let mut invariants = vec![InvariantCheck::BaseGenerationIsCurrent {
        expected: current.generation(),
    }];
    for mutation in &intent.mutations {
        if let Mutation::CreateWorkload { id, artifact, .. } = mutation {
            invariants.push(InvariantCheck::WorkloadDoesNotExist {
                workload: id.clone(),
            });
            invariants.push(InvariantCheck::ArtifactResolved {
                artifact: artifact.clone(),
            });
        }
    }
    for constraint in &intent.constraints {
        if let Constraint::MustPassHealthCheck { workload } = constraint {
            invariants.push(InvariantCheck::WorkloadHealthy {
                workload: workload.clone(),
            });
        }
    }
    invariants.push(InvariantCheck::RecoveryGenerationRetained);

    Ok(ExecutionIr {
        transaction_id: transaction_id(intent),
        base_generation: current.generation(),
        steps: builder.steps,
        invariants,
        failure_policy: FailurePolicy::AbandonCandidate,
    })
}

fn validate_live_references(intent: &IntentSir, current: &WorldState) -> Result<(), PlanError> {
    for mutation in &intent.mutations {
        match mutation {
            Mutation::CreateWorkload { id, .. } if current.workload(id).is_some() => {
                return Err(PlanError::WorkloadAlreadyExists(id.clone()));
            }
            Mutation::SetDesiredState { workload, .. } | Mutation::RemoveWorkload { workload }
                if current.workload(workload).is_none() =>
            {
                return Err(PlanError::WorkloadNotFound(workload.clone()));
            }
            _ => {}
        }
    }
    Ok(())
}

fn compile_mutation(
    builder: &mut PlanBuilder,
    candidate_step: StepId,
    mutation: &Mutation,
    constraints: &[Constraint],
) {
    let mut dependency = candidate_step;
    if let Mutation::CreateWorkload { artifact, .. } = mutation {
        dependency = builder.push(
            vec![dependency],
            ExecutionAction::ResolveArtifact {
                artifact: artifact.clone(),
            },
        );
    }

    dependency = builder.push(
        vec![dependency],
        ExecutionAction::ApplyMutation {
            mutation: mutation.clone(),
        },
    );

    match mutation {
        Mutation::CreateWorkload { id, desired, .. } => {
            dependency = builder.push(
                vec![dependency],
                ExecutionAction::PrepareWorkload {
                    workload: id.clone(),
                },
            );
            dependency = match desired {
                DesiredWorkloadState::Running => builder.push(
                    vec![dependency],
                    ExecutionAction::StartWorkload {
                        workload: id.clone(),
                    },
                ),
                DesiredWorkloadState::Stopped => dependency,
                DesiredWorkloadState::Absent => unreachable!("validated by Intent SIR"),
            };
            if needs_health_check(id, constraints) {
                builder.push(
                    vec![dependency],
                    ExecutionAction::CheckHealth {
                        workload: id.clone(),
                    },
                );
            }
        }
        Mutation::SetDesiredState { workload, state } => match state {
            DesiredWorkloadState::Running => {
                let started = builder.push(
                    vec![dependency],
                    ExecutionAction::StartWorkload {
                        workload: workload.clone(),
                    },
                );
                if needs_health_check(workload, constraints) {
                    builder.push(
                        vec![started],
                        ExecutionAction::CheckHealth {
                            workload: workload.clone(),
                        },
                    );
                }
            }
            DesiredWorkloadState::Stopped => {
                builder.push(
                    vec![dependency],
                    ExecutionAction::StopWorkload {
                        workload: workload.clone(),
                    },
                );
            }
            DesiredWorkloadState::Absent => {
                let stopped = builder.push(
                    vec![dependency],
                    ExecutionAction::StopWorkload {
                        workload: workload.clone(),
                    },
                );
                builder.push(
                    vec![stopped],
                    ExecutionAction::RemoveWorkload {
                        workload: workload.clone(),
                    },
                );
            }
        },
        Mutation::RemoveWorkload { workload } => {
            let stopped = builder.push(
                vec![dependency],
                ExecutionAction::StopWorkload {
                    workload: workload.clone(),
                },
            );
            builder.push(
                vec![stopped],
                ExecutionAction::RemoveWorkload {
                    workload: workload.clone(),
                },
            );
        }
    }
}

fn needs_health_check(workload: &WorkloadId, constraints: &[Constraint]) -> bool {
    constraints.iter().any(|constraint| {
        matches!(constraint, Constraint::MustPassHealthCheck { workload: target } if target == workload)
    })
}

fn transaction_id(intent: &IntentSir) -> TransactionId {
    TransactionId::from(format!("tx-{}", intent.proposal_id))
}

struct PlanBuilder {
    steps: Vec<ExecutionStep>,
    next_id: u32,
}

impl PlanBuilder {
    const fn new() -> Self {
        Self {
            steps: Vec::new(),
            next_id: 1,
        }
    }

    fn push(&mut self, depends_on: Vec<StepId>, action: ExecutionAction) -> StepId {
        let id = StepId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("plan step limit exceeded");
        self.steps.push(ExecutionStep {
            id,
            depends_on,
            action,
        });
        id
    }

    fn terminal_steps(&self) -> Vec<StepId> {
        let dependencies: std::collections::BTreeSet<_> = self
            .steps
            .iter()
            .flat_map(|step| step.depends_on.iter().copied())
            .collect();
        self.steps
            .iter()
            .filter(|step| !dependencies.contains(&step.id))
            .map(|step| step.id)
            .collect()
    }
}
