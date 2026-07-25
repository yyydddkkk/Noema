use std::cell::Cell;

use noema_ir::{
    ArtifactRef, Constraint, DesiredWorkloadState, EffectClass, EffectPolicy, GenerationId,
    HealthSpec, IntentSir, Mutation, ProposalId, RestartPolicy, SirVersion, ValidationCode,
    WorkloadId, validate_intent,
};

fn valid_intent() -> IntentSir {
    IntentSir {
        sir_version: SirVersion::V0,
        proposal_id: ProposalId::from("proposal-hello-1"),
        base_generation: GenerationId::INITIAL,
        mutations: vec![Mutation::CreateWorkload {
            id: WorkloadId::from("hello"),
            artifact: ArtifactRef::from("builtin:noema-test-workload"),
            desired: DesiredWorkloadState::Running,
            health: HealthSpec::Http {
                port: 8080,
                path: "/health".to_owned(),
                timeout_ms: 1_000,
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

#[test]
fn valid_v0_intent_passes_validation() {
    assert_eq!(validate_intent(&valid_intent()), Ok(()));
}

#[test]
fn valid_intent_round_trips_through_json() {
    let intent = valid_intent();
    let json = serde_json::to_string_pretty(&intent).expect("serialize valid intent");
    let decoded: IntentSir = serde_json::from_str(&json).expect("deserialize valid intent");
    assert_eq!(decoded, intent);
}

#[test]
fn unknown_json_fields_are_rejected() {
    let json = r#"
    {
      "sir_version": 0,
      "proposal_id": "proposal-1",
      "base_generation": 0,
      "mutations": [],
      "constraints": [],
      "effect_policy": {
        "maximum_effect": "locally_reversible",
        "allow_irreversible": false
      },
      "shell": "rm -rf /"
    }
    "#;

    let error = serde_json::from_str::<IntentSir>(json).expect_err("unknown field must fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn unknown_mutation_fields_are_rejected() {
    let json = r#"
    {
      "sir_version": 0,
      "proposal_id": "proposal-1",
      "base_generation": 0,
      "mutations": [
        {
          "type": "remove_workload",
          "workload": "hello",
          "command": "shutdown now"
        }
      ],
      "constraints": [],
      "effect_policy": {
        "maximum_effect": "locally_reversible",
        "allow_irreversible": false
      }
    }
    "#;

    let error = serde_json::from_str::<IntentSir>(json).expect_err("unknown field must fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn create_workload_reports_the_exact_invalid_id_path_once() {
    let mut intent = valid_intent();
    let Mutation::CreateWorkload { id, .. } = &mut intent.mutations[0] else {
        panic!("fixture must create a workload");
    };
    *id = WorkloadId::from("bad workload id");

    let errors = validate_intent(&intent).expect_err("invalid id must fail");
    let id_errors: Vec<_> = errors
        .iter()
        .filter(|error| error.code == ValidationCode::InvalidIdentifier)
        .collect();
    assert_eq!(id_errors.len(), 1);
    assert_eq!(id_errors[0].path, "/mutations/0/id");
}

#[test]
fn validation_collects_independent_errors() {
    let intent = IntentSir {
        sir_version: SirVersion(99),
        proposal_id: ProposalId::from("bad proposal id"),
        base_generation: GenerationId::INITIAL,
        mutations: Vec::new(),
        constraints: Vec::new(),
        effect_policy: EffectPolicy {
            maximum_effect: EffectClass::Irreversible,
            allow_irreversible: false,
        },
    };

    let errors = validate_intent(&intent).expect_err("intent must fail");
    let codes: Vec<_> = errors.iter().map(|error| error.code).collect();
    assert!(codes.contains(&ValidationCode::UnsupportedVersion));
    assert!(codes.contains(&ValidationCode::InvalidIdentifier));
    assert!(codes.contains(&ValidationCode::EmptyMutations));
    assert!(codes.contains(&ValidationCode::InconsistentEffectPolicy));
}

#[test]
fn invalid_intent_never_reaches_effect_boundary() {
    let mut intent = valid_intent();
    intent.mutations.push(Mutation::RemoveWorkload {
        workload: WorkloadId::from("hello"),
    });
    let effect_count = Cell::new(0_u32);

    if validate_intent(&intent).is_ok() {
        effect_count.set(effect_count.get() + 1);
    }

    assert_eq!(effect_count.get(), 0);
}

#[test]
fn intent_schema_can_be_generated() {
    let schema = schemars::schema_for!(IntentSir);
    let json = serde_json::to_value(schema).expect("serialize schema");
    assert_eq!(json["title"], "IntentSir");
    assert!(json["properties"]["mutations"].is_object());
}
