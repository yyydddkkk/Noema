use noema_contract::{ContractBuilder, ContractReply};
use noema_ir::{
    EffectClass, EffectPolicy, GenerationId, IntentSir, ProposalId, SirVersion, ValidationCode,
};
use noema_protocol::{DeterministicMockProvider, Gateway, GatewayError, ModelProvider};
use noema_state::{GenerationStore, MemoryGenerationStore};

fn request() -> noema_contract::ContractRequest {
    let store = MemoryGenerationStore::new();
    ContractBuilder::new(1_024)
        .build(
            "request-1",
            "Create the test workload",
            store.current(),
            store.events(),
        )
        .expect("build request")
}

fn empty_intent(base_generation: GenerationId) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from("proposal-test"),
        base_generation,
        mutations: Vec::new(),
        constraints: Vec::new(),
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::LocallyReversible,
            allow_irreversible: false,
        },
    }
}

fn encoded_reply(request_id: &str, intent: IntentSir) -> Vec<u8> {
    serde_json::to_vec(&ContractReply {
        request_id: request_id.to_owned(),
        intent,
    })
    .expect("serialize reply")
}

#[test]
fn valid_mock_reply_becomes_intent_only() {
    let request = request();
    let provider =
        DeterministicMockProvider::create_workload("hello", "builtin:noema-test-workload");
    let mut gateway = Gateway::new(provider);

    let intent = gateway.request_intent(&request).expect("valid intent");

    assert_eq!(intent.base_generation, request.base_generation);
    assert_eq!(intent.mutations.len(), 1);
    assert_eq!(gateway.provider().name(), "deterministic-mock");
}

#[test]
fn reply_is_bound_to_request_and_generation() {
    let request = request();
    let wrong_request = encoded_reply("request-other", empty_intent(request.base_generation));
    let mut gateway = Gateway::new(DeterministicMockProvider::raw(wrong_request));
    assert!(matches!(
        gateway.request_intent(&request),
        Err(GatewayError::MismatchedRequestId { .. })
    ));

    let stale = encoded_reply("request-1", empty_intent(GenerationId(99)));
    let mut gateway = Gateway::new(DeterministicMockProvider::raw(stale));
    assert!(matches!(
        gateway.request_intent(&request),
        Err(GatewayError::StaleBase { .. })
    ));
}

#[test]
fn unknown_shell_field_is_rejected_by_strict_decoder() {
    let request = request();
    let valid = encoded_reply("request-1", empty_intent(request.base_generation));
    let mut value: serde_json::Value = serde_json::from_slice(&valid).expect("decode fixture");
    value["shell"] = serde_json::json!("echo escaped");
    let bytes = serde_json::to_vec(&value).expect("encode fixture");
    let mut gateway = Gateway::new(DeterministicMockProvider::raw(bytes));

    assert!(matches!(
        gateway.request_intent(&request),
        Err(GatewayError::InvalidJson(_))
    ));
}

#[test]
fn reply_size_is_checked_before_json_decoding() {
    let request = request();
    let bytes = vec![b'x'; request.limits.maximum_reply_bytes + 1];
    let mut gateway = Gateway::new(DeterministicMockProvider::raw(bytes));

    assert_eq!(
        gateway.request_intent(&request),
        Err(GatewayError::ReplyTooLarge {
            actual: request.limits.maximum_reply_bytes + 1,
            maximum: request.limits.maximum_reply_bytes,
        })
    );
}

#[test]
fn semantic_validation_runs_after_binding() {
    let request = request();
    let bytes = encoded_reply("request-1", empty_intent(request.base_generation));
    let mut gateway = Gateway::new(DeterministicMockProvider::raw(bytes));

    let error = gateway
        .request_intent(&request)
        .expect_err("empty intent must fail");
    let GatewayError::InvalidIntent(errors) = error else {
        panic!("expected semantic validation error");
    };
    assert!(
        errors
            .iter()
            .any(|error| error.code == ValidationCode::EmptyMutations)
    );
}
