use noema_ir::{
    ArtifactRef, Constraint, DesiredWorkloadState, EffectClass, EffectPolicy, ExecutionAction,
    GenerationId, HealthSpec, IntentSir, Mutation, ProposalId, RestartPolicy, SirVersion,
    WorkloadId,
};
use noema_planner::{PlanError, plan};
use noema_state::WorldState;

fn create_intent(base_generation: GenerationId) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from("create-hello"),
        base_generation,
        mutations: vec![Mutation::CreateWorkload {
            id: WorkloadId::from("hello"),
            artifact: ArtifactRef::from("builtin:noema-test-workload"),
            desired: DesiredWorkloadState::Running,
            health: HealthSpec::Process,
            restart_policy: RestartPolicy::OnFailure,
        }],
        constraints: vec![
            Constraint::MustPassHealthCheck {
                workload: WorkloadId::from("hello"),
            },
            Constraint::RollbackOnFailure,
        ],
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::LocallyReversible,
            allow_irreversible: false,
        },
    }
}

#[test]
fn planning_is_deterministic() {
    let state = WorldState::initial();
    let intent = create_intent(GenerationId::INITIAL);
    assert_eq!(plan(&intent, &state), plan(&intent, &state));
}

#[test]
fn create_plan_has_structured_steps_and_no_shell() {
    let execution = plan(
        &create_intent(GenerationId::INITIAL),
        &WorldState::initial(),
    )
    .expect("plan valid intent");

    assert!(matches!(
        execution.steps.first().map(|step| &step.action),
        Some(ExecutionAction::CreateCandidateGeneration)
    ));
    assert!(
        execution
            .steps
            .iter()
            .any(|step| matches!(step.action, ExecutionAction::ApplyMutation { .. }))
    );
    assert!(matches!(
        execution.steps.last().map(|step| &step.action),
        Some(ExecutionAction::CommitGeneration)
    ));
    let json = serde_json::to_string(&execution).expect("serialize execution IR");
    assert!(!json.contains("shell"));
    assert!(!json.contains("command"));
}

#[test]
fn stale_generation_fails_before_a_plan_exists() {
    let error = plan(&create_intent(GenerationId(9)), &WorldState::initial())
        .expect_err("stale intent must fail");
    assert!(matches!(error, PlanError::StaleBase { .. }));
}
