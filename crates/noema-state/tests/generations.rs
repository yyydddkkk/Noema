use noema_ir::{
    ArtifactRef, DesiredWorkloadState, EffectClass, EffectPolicy, GenerationId, HealthSpec,
    IntentSir, Mutation, ObservedWorkloadState, ProposalId, RestartPolicy, SirVersion,
    TransactionId, WorkloadId, validate_intent,
};
use noema_state::{GenerationStore, MemoryGenerationStore, StateError, StateEventKind};

fn create_workload(id: &str) -> Mutation {
    Mutation::CreateWorkload {
        id: WorkloadId::from(id),
        artifact: ArtifactRef::from("builtin:noema-test-workload"),
        desired: DesiredWorkloadState::Running,
        health: HealthSpec::Process,
        restart_policy: RestartPolicy::OnFailure,
    }
}

fn transaction(id: &str) -> TransactionId {
    TransactionId::from(id)
}

#[test]
fn candidate_is_isolated_until_commit() {
    let mut store = MemoryGenerationStore::new();
    let mut candidate = store
        .begin(transaction("tx-1"), GenerationId::INITIAL)
        .expect("begin candidate");
    candidate
        .apply_mutation(&create_workload("hello"))
        .expect("apply mutation");

    assert!(
        store
            .current()
            .workload(&WorkloadId::from("hello"))
            .is_none()
    );
    assert!(
        candidate
            .state()
            .workload(&WorkloadId::from("hello"))
            .is_some()
    );

    let committed = store.commit(candidate).expect("commit candidate");
    assert_eq!(committed, GenerationId(1));
    assert_eq!(store.current().generation(), GenerationId(1));
    assert!(
        store
            .current()
            .workload(&WorkloadId::from("hello"))
            .is_some()
    );
}

#[test]
fn abort_preserves_current_state() {
    let mut store = MemoryGenerationStore::new();
    let before = store.current().clone();
    let mut candidate = store
        .begin(transaction("tx-abort"), GenerationId::INITIAL)
        .expect("begin candidate");
    candidate
        .apply_mutation(&create_workload("discarded"))
        .expect("apply mutation");

    store.abort(candidate);

    assert_eq!(store.current(), &before);
    assert!(matches!(
        store.events().last().map(noema_state::StateEvent::kind),
        Some(StateEventKind::CandidateAborted { .. })
    ));
}

#[test]
fn stale_base_is_rejected_without_allocating_a_candidate() {
    let mut store = MemoryGenerationStore::new();
    let error = store
        .begin(transaction("tx-stale"), GenerationId(99))
        .expect_err("stale base must fail");
    assert_eq!(
        error,
        StateError::StaleBase {
            current: GenerationId::INITIAL,
            requested: GenerationId(99),
        }
    );
    assert!(store.events().is_empty());

    let candidate = store
        .begin(transaction("tx-valid"), GenerationId::INITIAL)
        .expect("valid begin");
    assert_eq!(candidate.id(), GenerationId(1));
}

#[test]
fn competing_candidate_cannot_replace_a_newer_generation() {
    let mut store = MemoryGenerationStore::new();
    let mut first = store
        .begin(transaction("tx-first"), GenerationId::INITIAL)
        .expect("first candidate");
    let mut second = store
        .begin(transaction("tx-second"), GenerationId::INITIAL)
        .expect("second candidate");
    first
        .apply_mutation(&create_workload("winner"))
        .expect("first mutation");
    second
        .apply_mutation(&create_workload("loser"))
        .expect("second mutation");

    store.commit(first).expect("first candidate commits");
    let error = store.commit(second).expect_err("second candidate is stale");

    assert_eq!(
        error,
        StateError::StaleCandidate {
            current: GenerationId(1),
            based_on: GenerationId::INITIAL,
        }
    );
    assert!(
        store
            .current()
            .workload(&WorkloadId::from("winner"))
            .is_some()
    );
    assert!(
        store
            .current()
            .workload(&WorkloadId::from("loser"))
            .is_none()
    );
    assert!(matches!(
        store.events().last().map(noema_state::StateEvent::kind),
        Some(StateEventKind::CandidateCommitRejected { .. })
    ));
}

#[test]
fn generation_identifiers_are_not_reused_after_abort() {
    let mut store = MemoryGenerationStore::new();
    let first = store
        .begin(transaction("tx-1"), GenerationId::INITIAL)
        .expect("first candidate");
    assert_eq!(first.id(), GenerationId(1));
    store.abort(first);

    let second = store
        .begin(transaction("tx-2"), GenerationId::INITIAL)
        .expect("second candidate");
    assert_eq!(second.id(), GenerationId(2));
}

#[test]
fn desired_and_observed_state_have_separate_write_paths() {
    let mut store = MemoryGenerationStore::new();
    let id = WorkloadId::from("hello");
    let mut candidate = store
        .begin(transaction("tx-observe"), GenerationId::INITIAL)
        .expect("begin candidate");
    candidate
        .apply_mutation(&create_workload("hello"))
        .expect("create workload");

    let workload = candidate.state().workload(&id).expect("workload exists");
    assert_eq!(workload.desired(), DesiredWorkloadState::Running);
    assert_eq!(workload.observed(), ObservedWorkloadState::Absent);

    candidate
        .observe_workload(&id, ObservedWorkloadState::Starting)
        .expect("record observation");
    let workload = candidate.state().workload(&id).expect("workload exists");
    assert_eq!(workload.desired(), DesiredWorkloadState::Running);
    assert_eq!(workload.observed(), ObservedWorkloadState::Starting);
}

#[test]
fn workload_can_only_be_purged_after_desired_and_observed_are_absent() {
    let mut store = MemoryGenerationStore::new();
    let id = WorkloadId::from("hello");
    let mut create = store
        .begin(transaction("tx-create"), GenerationId::INITIAL)
        .expect("begin create");
    create
        .apply_mutation(&create_workload("hello"))
        .expect("create workload");
    create
        .observe_workload(&id, ObservedWorkloadState::Running)
        .expect("observe running");
    store.commit(create).expect("commit create");

    let mut remove = store
        .begin(transaction("tx-remove"), GenerationId(1))
        .expect("begin remove");
    remove
        .apply_mutation(&Mutation::RemoveWorkload {
            workload: id.clone(),
        })
        .expect("request removal");

    assert!(matches!(
        remove.purge_workload(&id),
        Err(StateError::WorkloadNotAbsent { .. })
    ));
    remove
        .observe_workload(&id, ObservedWorkloadState::Absent)
        .expect("observe absent");
    remove.purge_workload(&id).expect("purge absent workload");
    store.commit(remove).expect("commit removal");
    assert!(store.current().workload(&id).is_none());
}

#[test]
fn empty_candidate_cannot_create_a_generation() {
    let mut store = MemoryGenerationStore::new();
    let candidate = store
        .begin(transaction("tx-empty"), GenerationId::INITIAL)
        .expect("begin candidate");
    let error = store.commit(candidate).expect_err("empty commit must fail");

    assert_eq!(error, StateError::EmptyCandidate);
    assert_eq!(store.current().generation(), GenerationId::INITIAL);
    assert!(matches!(
        store.events().last().map(noema_state::StateEvent::kind),
        Some(StateEventKind::CandidateAborted { .. })
    ));
}

#[test]
fn invalid_intent_is_rejected_before_state_transaction_begins() {
    let intent = IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from("proposal-invalid"),
        base_generation: GenerationId::INITIAL,
        mutations: Vec::new(),
        constraints: Vec::new(),
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::LocallyReversible,
            allow_irreversible: false,
        },
    };
    let mut store = MemoryGenerationStore::new();

    if validate_intent(&intent).is_ok() {
        store
            .begin(transaction("tx-invalid"), intent.base_generation)
            .expect("validated intent should begin");
    }

    assert_eq!(store.current().generation(), GenerationId::INITIAL);
    assert!(store.events().is_empty());
}

#[test]
fn committed_events_are_ordered_and_causally_linked() {
    let mut store = MemoryGenerationStore::new();
    let transaction_id = transaction("tx-events");
    let mut candidate = store
        .begin(transaction_id.clone(), GenerationId::INITIAL)
        .expect("begin candidate");
    candidate
        .apply_mutation(&create_workload("hello"))
        .expect("create workload");
    store.commit(candidate).expect("commit candidate");

    assert_eq!(store.events().len(), 3);
    for (index, event) in store.events().iter().enumerate() {
        assert_eq!(
            event.sequence(),
            u64::try_from(index + 1).expect("small index")
        );
        assert_eq!(event.transaction_id(), &transaction_id);
        assert_eq!(event.generation(), GenerationId(1));
    }
    assert!(matches!(
        store.events()[0].kind(),
        StateEventKind::CandidateStarted { .. }
    ));
    assert!(matches!(
        store.events()[1].kind(),
        StateEventKind::WorkloadCreated { .. }
    ));
    assert!(matches!(
        store.events()[2].kind(),
        StateEventKind::CandidateCommitted { .. }
    ));
}

#[test]
fn world_state_round_trips_without_mutable_access() {
    let state = noema_state::WorldState::initial();
    let json = serde_json::to_string(&state).expect("serialize world state");
    let decoded: noema_state::WorldState =
        serde_json::from_str(&json).expect("deserialize world state");
    assert_eq!(decoded, state);
}
