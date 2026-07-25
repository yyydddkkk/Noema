use std::{
    env,
    error::Error,
    net::TcpListener,
    path::{Path, PathBuf},
    process::ExitCode,
};

use noema_executor::ProcessBackend;
use noema_ir::{
    ArtifactRef, Constraint, DesiredWorkloadState, EffectClass, EffectPolicy, GenerationId,
    HealthSpec, IntentSir, Mutation, ProposalId, RestartPolicy, SirVersion, WorkloadId,
};
use noema_reconciler::{Reconciler, TransactionStatus};
use noema_state::{GenerationStore, MemoryGenerationStore};

const NORMAL: &str = "builtin:noema-test-workload";
const UNHEALTHY: &str = "builtin:noema-test-workload:unhealthy";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("M3_FAILED {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let executable = env::var("NOEMA_TEST_WORKLOAD")?;
    let state_path = PathBuf::from(env::var("NOEMA_STATE_PATH")?);
    if state_path.exists() {
        return recover_persisted(&executable, &state_path);
    }

    let workload = WorkloadId::from("hello");
    let mut reconciler = Reconciler::with_backend(ProcessBackend::new(&executable));
    let started = reconciler.submit(&create_intent(NORMAL, unused_port()?, "docker-start"))?;
    require(
        started.status == TransactionStatus::Committed,
        "normal Workload was not committed",
    )?;
    let first_pid = reconciler
        .backend()
        .process_id(&workload)
        .ok_or("normal Workload has no process")?;

    reconciler.backend_mut().force_crash(&workload)?;
    let recovered = reconciler
        .reconcile_once()?
        .ok_or("crashed Workload produced no recovery Evidence")?;
    require(
        recovered.new_generation == Some(GenerationId(2)),
        "recovery did not commit generation 2",
    )?;
    let second_pid = reconciler
        .backend()
        .process_id(&workload)
        .ok_or("recovered Workload has no process")?;
    require(
        first_pid != second_pid,
        "recovery did not replace the process",
    )?;

    let mut failing = Reconciler::with_backend(ProcessBackend::new(executable));
    let rejected = failing.submit(&create_intent(
        UNHEALTHY,
        unused_port()?,
        "docker-unhealthy",
    ))?;
    require(
        rejected.status == TransactionStatus::RolledBack,
        "unhealthy Workload was not rolled back",
    )?;
    require(
        failing.backend().process_id(&workload).is_none(),
        "rolled-back Workload process is still present",
    )?;
    reconciler.store().save(&state_path)?;

    println!(
        "M3_OK start_generation=1 recovery_generation=2 rollback_generation=none old_pid={first_pid} new_pid={second_pid} state={}",
        state_path.display()
    );
    Ok(())
}

fn recover_persisted(executable: &str, state_path: &Path) -> Result<(), Box<dyn Error>> {
    let store = MemoryGenerationStore::load(state_path)?;
    let workload = WorkloadId::from("hello");
    let old_generation = store.current().generation();
    let mut reconciler = Reconciler::with_store(ProcessBackend::new(executable), store);
    let evidence = reconciler
        .reconcile_once()?
        .ok_or("persisted running Workload produced no recovery transaction")?;
    require(
        evidence.old_generation == old_generation,
        "recovery used the wrong persisted generation",
    )?;
    let new_generation = evidence
        .new_generation
        .ok_or("recovery did not commit a generation")?;
    require(
        reconciler.backend().process_id(&workload).is_some(),
        "persisted Workload was not restarted",
    )?;
    reconciler.store().save(state_path)?;
    println!(
        "M3_RECOVERED old_generation={old_generation} new_generation={new_generation} state={}",
        state_path.display()
    );
    Ok(())
}

fn create_intent(artifact: &str, port: u16, proposal: &str) -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from(proposal),
        base_generation: GenerationId::INITIAL,
        mutations: vec![Mutation::CreateWorkload {
            id: WorkloadId::from("hello"),
            artifact: ArtifactRef::from(artifact),
            desired: DesiredWorkloadState::Running,
            health: HealthSpec::Http {
                port,
                path: "/health".to_owned(),
                timeout_ms: 500,
            },
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

fn unused_port() -> Result<u16, Box<dyn Error>> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn require(condition: bool, message: &'static str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
