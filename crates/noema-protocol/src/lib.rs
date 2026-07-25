//! Strict protocol boundary between Noema and an untrusted cloud model.
//!
//! Providers transport a complete [`ContractRequest`] and return opaque
//! bytes. The [`Gateway`] is the only component that decodes those bytes into
//! model-authored Intent SIR. It does not own a state store or an executor.

use std::{error::Error, fmt};

use noema_contract::{ContractReply, ContractRequest};
use noema_ir::{
    ArtifactRef, DesiredWorkloadState, EffectClass, EffectPolicy, HealthSpec, IntentSir, Mutation,
    ProposalId, RestartPolicy, SirVersion, ValidationError, WorkloadId, validate_intent,
};

#[cfg(feature = "openai")]
mod openai;

#[cfg(feature = "openai")]
pub use openai::{OpenAiProvider, OpenAiTransport, ReqwestOpenAiTransport};

/// A transport adapter for one remote model service.
pub trait ModelProvider {
    /// Returns the provider's opaque response body.
    ///
    /// # Errors
    ///
    /// Returns a transport-level error. Providers must not decode Intent SIR
    /// or directly invoke any Noema effect boundary.
    fn complete(&mut self, request: &ContractRequest) -> Result<Vec<u8>, ProviderError>;

    fn name(&self) -> &'static str;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError {
    message: String,
}

impl ProviderError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ProviderError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayError {
    Provider(ProviderError),
    ReplyTooLarge {
        actual: usize,
        maximum: usize,
    },
    InvalidJson(String),
    MismatchedRequestId {
        expected: String,
        actual: String,
    },
    StaleBase {
        expected: noema_ir::GenerationId,
        actual: noema_ir::GenerationId,
    },
    InvalidIntent(Vec<ValidationError>),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(error) => write!(formatter, "model provider failed: {error}"),
            Self::ReplyTooLarge { actual, maximum } => write!(
                formatter,
                "model reply is {actual} bytes, exceeding the {maximum}-byte limit"
            ),
            Self::InvalidJson(error) => write!(formatter, "model reply is invalid: {error}"),
            Self::MismatchedRequestId { expected, actual } => write!(
                formatter,
                "model reply request identifier '{actual}' does not match '{expected}'"
            ),
            Self::StaleBase { expected, actual } => write!(
                formatter,
                "model intent generation {actual} does not match request generation {expected}"
            ),
            Self::InvalidIntent(errors) => {
                write!(
                    formatter,
                    "model intent failed {} validation checks",
                    errors.len()
                )
            }
        }
    }
}

impl Error for GatewayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Provider(error) => Some(error),
            _ => None,
        }
    }
}

/// Decodes an untrusted provider reply without crossing an effect boundary.
pub struct Gateway<P> {
    provider: P,
}

impl<P: ModelProvider> Gateway<P> {
    #[must_use]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }

    #[must_use]
    pub const fn provider(&self) -> &P {
        &self.provider
    }

    /// Requests and validates one model-authored Intent SIR.
    ///
    /// # Errors
    ///
    /// Rejects transport failures, oversized or structurally invalid replies,
    /// incorrect request/generation bindings, and invalid Intent SIR.
    pub fn request_intent(&mut self, request: &ContractRequest) -> Result<IntentSir, GatewayError> {
        let bytes = self
            .provider
            .complete(request)
            .map_err(GatewayError::Provider)?;
        let maximum = request.limits.maximum_reply_bytes;
        if bytes.len() > maximum {
            return Err(GatewayError::ReplyTooLarge {
                actual: bytes.len(),
                maximum,
            });
        }

        let reply: ContractReply = serde_json::from_slice(&bytes)
            .map_err(|error| GatewayError::InvalidJson(error.to_string()))?;
        if reply.request_id != request.request_id {
            return Err(GatewayError::MismatchedRequestId {
                expected: request.request_id.clone(),
                actual: reply.request_id,
            });
        }
        if reply.intent.base_generation != request.base_generation {
            return Err(GatewayError::StaleBase {
                expected: request.base_generation,
                actual: reply.intent.base_generation,
            });
        }
        validate_intent(&reply.intent).map_err(GatewayError::InvalidIntent)?;
        Ok(reply.intent)
    }
}

/// A deterministic protocol test double. It performs no model inference.
pub struct DeterministicMockProvider {
    response: MockResponse,
}

enum MockResponse {
    CreateWorkload {
        workload: WorkloadId,
        artifact: ArtifactRef,
    },
    Raw(Vec<u8>),
    Error(ProviderError),
}

impl DeterministicMockProvider {
    #[must_use]
    pub fn create_workload(
        workload: impl Into<WorkloadId>,
        artifact: impl Into<ArtifactRef>,
    ) -> Self {
        Self {
            response: MockResponse::CreateWorkload {
                workload: workload.into(),
                artifact: artifact.into(),
            },
        }
    }

    #[must_use]
    pub fn raw(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            response: MockResponse::Raw(bytes.into()),
        }
    }

    #[must_use]
    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            response: MockResponse::Error(ProviderError::new(message)),
        }
    }
}

impl ModelProvider for DeterministicMockProvider {
    fn complete(&mut self, request: &ContractRequest) -> Result<Vec<u8>, ProviderError> {
        match &self.response {
            MockResponse::CreateWorkload { workload, artifact } => {
                let reply = ContractReply {
                    request_id: request.request_id.clone(),
                    intent: IntentSir {
                        sir_version: SirVersion::V0,
                        proposal_id: ProposalId::from(format!("mock-{}", request.request_id)),
                        base_generation: request.base_generation,
                        mutations: vec![Mutation::CreateWorkload {
                            id: workload.clone(),
                            artifact: artifact.clone(),
                            desired: DesiredWorkloadState::Running,
                            health: HealthSpec::Process,
                            restart_policy: RestartPolicy::OnFailure,
                        }],
                        constraints: Vec::new(),
                        effect_policy: EffectPolicy {
                            maximum_effect: EffectClass::LocallyReversible,
                            allow_irreversible: false,
                        },
                    },
                };
                serde_json::to_vec(&reply).map_err(|error| ProviderError::new(error.to_string()))
            }
            MockResponse::Raw(bytes) => Ok(bytes.clone()),
            MockResponse::Error(error) => Err(error.clone()),
        }
    }

    fn name(&self) -> &'static str {
        "deterministic-mock"
    }
}
