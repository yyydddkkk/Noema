#![cfg(feature = "openai")]

use noema_contract::ContractBuilder;
use noema_protocol::{Gateway, GatewayError, OpenAiProvider, OpenAiTransport, ProviderError};
use noema_state::{GenerationStore, MemoryGenerationStore};

struct RecordingTransport {
    body: Option<Vec<u8>>,
    response: Vec<u8>,
}

impl OpenAiTransport for RecordingTransport {
    fn send(
        &mut self,
        api_key: &str,
        request_body: &[u8],
        maximum_response_bytes: usize,
    ) -> Result<Vec<u8>, ProviderError> {
        assert_eq!(api_key, "test-key");
        assert_eq!(maximum_response_bytes, 1_024 * 1_024);
        self.body = Some(request_body.to_vec());
        Ok(self.response.clone())
    }
}

fn request() -> noema_contract::ContractRequest {
    let store = MemoryGenerationStore::new();
    ContractBuilder::default()
        .build(
            "openai-request-1",
            "Create a test workload",
            store.current(),
            store.events(),
        )
        .expect("build request")
}

fn completed_response(text: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }]
    }))
    .expect("encode fixture")
}

#[test]
fn openai_request_uses_structured_outputs_without_tools_or_storage() {
    let request = request();
    let transport = RecordingTransport {
        body: None,
        response: completed_response("{}"),
    };
    let provider = OpenAiProvider::with_transport("test-key", "gpt-5.6", 2_048, transport)
        .expect("construct provider");
    let mut gateway = Gateway::new(provider);

    assert!(matches!(
        gateway.request_intent(&request),
        Err(GatewayError::InvalidJson(_))
    ));

    let body = gateway
        .provider()
        .transport()
        .body
        .as_ref()
        .expect("request recorded");
    let value: serde_json::Value = serde_json::from_slice(body).expect("decode request");
    assert_eq!(value["model"], "gpt-5.6");
    assert_eq!(value["store"], false);
    assert_eq!(value["max_output_tokens"], 2_048);
    assert_eq!(value["text"]["format"]["type"], "json_schema");
    assert_eq!(value["text"]["format"]["strict"], true);
    assert!(value.get("tools").is_none());
    assert!(value["text"]["format"]["schema"].get("$schema").is_none());

    let contract: serde_json::Value =
        serde_json::from_str(value["input"].as_str().expect("contract is JSON text"))
            .expect("decode embedded contract");
    assert_eq!(contract["request_id"], "openai-request-1");
    assert_eq!(contract["objective"], "Create a test workload");
}

#[test]
fn incomplete_refusal_and_ambiguous_text_are_transport_errors() {
    let cases = [
        serde_json::json!({"status": "incomplete", "output": []}),
        serde_json::json!({
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "refusal", "refusal": "no"}]}]
        }),
        serde_json::json!({
            "status": "completed",
            "output": [{"type": "message", "content": [
                {"type": "output_text", "text": "{}"},
                {"type": "output_text", "text": "{}"}
            ]}]
        }),
    ];

    for response in cases {
        let transport = RecordingTransport {
            body: None,
            response: serde_json::to_vec(&response).expect("encode fixture"),
        };
        let provider = OpenAiProvider::with_transport("test-key", "gpt-5.6", 2_048, transport)
            .expect("construct provider");
        let mut gateway = Gateway::new(provider);
        assert!(matches!(
            gateway.request_intent(&request()),
            Err(GatewayError::Provider(_))
        ));
    }
}

#[test]
fn credentials_and_model_identifiers_are_validated_locally() {
    let transport = RecordingTransport {
        body: None,
        response: Vec::new(),
    };
    assert!(OpenAiProvider::with_transport("bad\nkey", "gpt-5.6", 100, transport).is_err());

    let transport = RecordingTransport {
        body: None,
        response: Vec::new(),
    };
    assert!(OpenAiProvider::with_transport("test-key", "bad/model", 100, transport).is_err());
}
