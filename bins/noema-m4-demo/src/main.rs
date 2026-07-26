use std::{error::Error, process::ExitCode};

use noema_contract::ContractBuilder;
use noema_ir::{GenerationId, WorkloadId};
use noema_protocol::{DeterministicMockProvider, Gateway, GatewayError};
use noema_reconciler::{Reconciler, TransactionStatus};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("M4_FAILED {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut reconciler = Reconciler::new();
    let request = ContractBuilder::default().build(
        "docker-m4-1",
        "Create and keep the built-in hello workload running",
        reconciler.current(),
        &[],
    )?;
    let provider =
        DeterministicMockProvider::create_workload("hello", "builtin:noema-test-workload");
    let intent = Gateway::new(provider).request_intent(&request)?;
    let outcome = reconciler.submit(&intent)?;
    require(
        outcome.status == TransactionStatus::Committed,
        "validated mock intent did not commit",
    )?;
    require(
        reconciler.current().generation() == GenerationId(1),
        "mock intent did not create generation 1",
    )?;
    require(
        reconciler
            .current()
            .workload(&WorkloadId::from("hello"))
            .is_some(),
        "mock intent did not create the Workload",
    )?;

    let escaped = br#"{"request_id":"docker-m4-2","shell":"id","intent":{}}"#;
    let mut rejecting = Gateway::new(DeterministicMockProvider::raw(escaped.to_vec()));
    require(
        matches!(
            rejecting.request_intent(&request),
            Err(GatewayError::InvalidJson(_))
        ),
        "gateway accepted an out-of-contract shell field",
    )?;
    require(
        reconciler.current().generation() == GenerationId(1),
        "rejected response changed the current generation",
    )?;

    println!(
        "M4_OK contract_version={} request_id={} generation=1 rejected=shell_field",
        request.contract_version.0, request.request_id
    );
    Ok(())
}

fn require(condition: bool, message: &'static str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
