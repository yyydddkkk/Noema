use std::{env, error::Error, process::ExitCode, time::Duration};

use noema_contract::{ContractBuilder, ContractRequest};
use noema_ir::{
    ArtifactRef, DesiredWorkloadState, EffectClass, GenerationId, Mutation, WorkloadId,
};
use noema_protocol::{Gateway, OpenAiProvider};
use noema_reconciler::{Reconciler, TransactionStatus};

const ENABLE_VALUE: &str = "ONE_REQUEST_TO_OPENAI";
const INPUT_TOKEN_SAFETY_MARGIN: u32 = 4_096;
const MAXIMUM_OUTPUT_TOKENS: u32 = 2_048;
const TIMEOUT: Duration = Duration::from_secs(30);
const OBJECTIVE: &str = "Create exactly one workload named hello using artifact builtin:noema-test-workload. Its desired state must be running, its effect policy must be locally reversible, and it must not perform any other mutation.";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("LIVE_EVAL_FAILED {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mode = parse_mode()?;
    let model = required_environment("NOEMA_OPENAI_MODEL")?;
    let request = contract()?;
    let pricing = Pricing::from_environment()?;

    match mode {
        Mode::DryRun => {
            let request_bytes = conservative_request_bytes(&request, &model)?;
            let maximum_cost =
                pricing.maximum_cost(maximum_input_tokens(request_bytes)?, MAXIMUM_OUTPUT_TOKENS);
            pricing.enforce_budget(maximum_cost)?;
            print_budget("LIVE_EVAL_DRY_RUN", &model, request_bytes, maximum_cost);
            Ok(())
        }
        Mode::Live => run_live(&request, &model, &pricing),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    DryRun,
    Live,
}

fn parse_mode() -> Result<Mode, Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let mode = match arguments.next().as_deref() {
        Some("--dry-run") => Mode::DryRun,
        Some("--live") => Mode::Live,
        _ => return Err("usage: noema-live-eval <--dry-run|--live>".into()),
    };
    if arguments.next().is_some() {
        return Err("usage: noema-live-eval <--dry-run|--live>".into());
    }
    Ok(mode)
}

fn contract() -> Result<ContractRequest, Box<dyn Error>> {
    let reconciler = Reconciler::new();
    Ok(ContractBuilder::default().build("m4-live-eval-1", OBJECTIVE, reconciler.current(), &[])?)
}

fn conservative_request_bytes(
    request: &ContractRequest,
    model: &str,
) -> Result<usize, Box<dyn Error>> {
    let provider =
        OpenAiProvider::with_limits("dry-run-placeholder", model, MAXIMUM_OUTPUT_TOKENS, TIMEOUT)?;
    Ok(provider.encoded_request_bytes(request)?)
}

fn run_live(
    request: &ContractRequest,
    model: &str,
    pricing: &Pricing,
) -> Result<(), Box<dyn Error>> {
    require_live_acknowledgement()?;
    let api_key = required_environment("OPENAI_API_KEY")?;
    let provider = OpenAiProvider::with_limits(api_key, model, MAXIMUM_OUTPUT_TOKENS, TIMEOUT)?;
    let request_bytes = provider.encoded_request_bytes(request)?;
    let maximum_cost =
        pricing.maximum_cost(maximum_input_tokens(request_bytes)?, MAXIMUM_OUTPUT_TOKENS);
    pricing.enforce_budget(maximum_cost)?;
    print_budget("LIVE_EVAL_SENDING", model, request_bytes, maximum_cost);

    let mut gateway = Gateway::new(provider);
    let intent = gateway.request_intent(request)?;
    verify_candidate(&intent)?;
    let mut reconciler = Reconciler::new();
    let outcome = reconciler.submit(&intent)?;
    if outcome.status != TransactionStatus::Committed
        || reconciler.current().generation() != GenerationId(1)
    {
        return Err("validated model intent did not commit generation 1".into());
    }
    let workload = reconciler
        .current()
        .workload(&WorkloadId::from("hello"))
        .ok_or("validated model intent did not create workload hello")?;
    if workload.desired() != DesiredWorkloadState::Running {
        return Err("validated model intent did not keep workload hello running".into());
    }

    println!(
        "LIVE_EVAL_OK model={model} requests=1 generation=1 proposal={}",
        intent.proposal_id
    );
    Ok(())
}

fn verify_candidate(intent: &noema_ir::IntentSir) -> Result<(), Box<dyn Error>> {
    if intent.effect_policy.maximum_effect != EffectClass::LocallyReversible
        || intent.effect_policy.allow_irreversible
    {
        return Err("model intent exceeded the live-eval effect policy".into());
    }
    let [
        Mutation::CreateWorkload {
            id,
            artifact,
            desired,
            ..
        },
    ] = intent.mutations.as_slice()
    else {
        return Err("model intent did not contain exactly one create mutation".into());
    };
    if id != &WorkloadId::from("hello")
        || artifact != &ArtifactRef::from("builtin:noema-test-workload")
        || *desired != DesiredWorkloadState::Running
    {
        return Err("model intent did not match the fixed evaluation objective".into());
    }
    Ok(())
}

fn require_live_acknowledgement() -> Result<(), Box<dyn Error>> {
    if env::var("NOEMA_LIVE_EVAL").as_deref() == Ok(ENABLE_VALUE) {
        Ok(())
    } else {
        Err(format!("set NOEMA_LIVE_EVAL={ENABLE_VALUE} to authorize one request").into())
    }
}

fn required_environment(name: &'static str) -> Result<String, Box<dyn Error>> {
    let value = env::var(name).map_err(|_| format!("environment variable {name} is required"))?;
    if value.trim().is_empty() || value.trim() != value {
        Err(format!("environment variable {name} is invalid").into())
    } else {
        Ok(value)
    }
}

struct Pricing {
    input_per_million: f64,
    output_per_million: f64,
    maximum_usd: f64,
}

impl Pricing {
    fn from_environment() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            input_per_million: positive_number("NOEMA_OPENAI_INPUT_USD_PER_MILLION")?,
            output_per_million: positive_number("NOEMA_OPENAI_OUTPUT_USD_PER_MILLION")?,
            maximum_usd: positive_number("NOEMA_LIVE_EVAL_MAX_USD")?,
        })
    }

    fn maximum_cost(&self, maximum_input_tokens: u32, maximum_output_tokens: u32) -> f64 {
        f64::from(maximum_input_tokens).mul_add(
            self.input_per_million / 1_000_000.0,
            f64::from(maximum_output_tokens) * self.output_per_million / 1_000_000.0,
        )
    }

    fn enforce_budget(&self, maximum_cost: f64) -> Result<(), Box<dyn Error>> {
        if maximum_cost <= self.maximum_usd {
            Ok(())
        } else {
            Err(format!(
                "conservative cost bound ${maximum_cost:.6} exceeds budget ${:.6}",
                self.maximum_usd
            )
            .into())
        }
    }
}

fn maximum_input_tokens(request_bytes: usize) -> Result<u32, Box<dyn Error>> {
    u32::try_from(request_bytes)
        .map_err(|_| "encoded request is too large for the live-eval budget".into())
        .and_then(|bytes| {
            bytes
                .checked_add(INPUT_TOKEN_SAFETY_MARGIN)
                .ok_or_else(|| "live-eval input estimate overflowed".into())
        })
}

fn positive_number(name: &'static str) -> Result<f64, Box<dyn Error>> {
    let raw = required_environment(name)?;
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("environment variable {name} must be a number"))?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!("environment variable {name} must be finite and positive").into())
    }
}

fn print_budget(prefix: &str, model: &str, request_bytes: usize, maximum_cost: f64) {
    let maximum_input_tokens = maximum_input_tokens(request_bytes)
        .expect("a previously encoded bounded contract must fit the live-eval estimate");
    println!(
        "{prefix} model={model} requests=1 request_bytes={request_bytes} estimated_maximum_input_tokens={maximum_input_tokens} maximum_output_tokens={MAXIMUM_OUTPUT_TOKENS} timeout_seconds={} maximum_cost_usd={maximum_cost:.6}",
        TIMEOUT.as_secs()
    );
}

#[cfg(test)]
mod tests {
    use noema_ir::{EffectPolicy, HealthSpec, IntentSir, ProposalId, RestartPolicy, SirVersion};

    use super::*;

    fn expected_intent() -> IntentSir {
        IntentSir {
            sir_version: SirVersion::V0,
            proposal_id: ProposalId::from("live-eval-test"),
            base_generation: GenerationId::INITIAL,
            mutations: vec![Mutation::CreateWorkload {
                id: WorkloadId::from("hello"),
                artifact: ArtifactRef::from("builtin:noema-test-workload"),
                desired: DesiredWorkloadState::Running,
                health: HealthSpec::Process,
                restart_policy: RestartPolicy::OnFailure,
            }],
            constraints: Vec::new(),
            effect_policy: EffectPolicy {
                maximum_effect: EffectClass::LocallyReversible,
                allow_irreversible: false,
            },
        }
    }

    #[test]
    fn candidate_acceptance_is_narrower_than_general_sir_validation() {
        assert!(verify_candidate(&expected_intent()).is_ok());

        let mut extra = expected_intent();
        extra.mutations.push(Mutation::SetDesiredState {
            workload: WorkloadId::from("hello"),
            state: DesiredWorkloadState::Stopped,
        });
        assert!(verify_candidate(&extra).is_err());
    }

    #[test]
    fn conservative_cost_must_fit_the_operator_budget() {
        let pricing = Pricing {
            input_per_million: 2.0,
            output_per_million: 8.0,
            maximum_usd: 0.02,
        };
        let cost = pricing.maximum_cost(1_000, 2_000);
        assert!((cost - 0.018).abs() < f64::EPSILON);
        assert!(pricing.enforce_budget(cost).is_ok());

        let too_small = Pricing {
            maximum_usd: 0.01,
            ..pricing
        };
        assert!(too_small.enforce_budget(cost).is_err());
    }
}
