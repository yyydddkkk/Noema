# System Intent Representation v0

Status: Normative draft 0.1

SIR v0 defines the smallest model-authored proposal that can create and drive
one Noema Workload. It deliberately omits arbitrary file edits, packages,
users, secrets, network policy, and shell execution.

## 1. Protocol boundary

The model may submit only Intent SIR. Execution IR and Evidence IR use the same
Rust library for consistent identifiers and states, but they are emitted only
by trusted local components.

Unknown JSON fields are rejected. Enum variants use a `type` discriminator and
`snake_case` names. Identifiers must be non-empty, at most 128 bytes, and use
ASCII letters, digits, `.`, `_`, or `-`.

## 2. Intent envelope

```json
{
  "sir_version": 0,
  "proposal_id": "proposal-0001",
  "base_generation": 0,
  "mutations": [],
  "constraints": [],
  "effect_policy": {
    "maximum_effect": "locally_reversible",
    "allow_irreversible": false
  }
}
```

An intent must contain at least one mutation. `base_generation` is the exact
state snapshot against which the model reasoned. A stale generation is a
semantic rejection and must not be silently rebased.

## 3. Workload mutations

SIR v0 supports:

- `create_workload`: declare a new workload, artifact, desired state, health
  contract, and restart policy.
- `set_desired_state`: change an existing or same-proposal workload target.
- `remove_workload`: declare that a workload should be absent.

SIR v0 does not specify how an artifact becomes a process. That decision
belongs to the local planner and backend.

Example:

```json
{
  "sir_version": 0,
  "proposal_id": "proposal-hello-1",
  "base_generation": 0,
  "mutations": [
    {
      "type": "create_workload",
      "id": "hello",
      "artifact": "builtin:noema-test-workload",
      "desired": "running",
      "health": {
        "type": "http",
        "port": 8080,
        "path": "/health",
        "timeout_ms": 1000
      },
      "restart_policy": "on_failure"
    }
  ],
  "constraints": [
    {
      "type": "must_pass_health_check",
      "workload": "hello"
    },
    {
      "type": "rollback_on_failure"
    }
  ],
  "effect_policy": {
    "maximum_effect": "locally_reversible",
    "allow_irreversible": false
  }
}
```

## 4. Validation

Validation is pure and returns every independently detectable error in a
stable input order. Each error contains:

- a stable machine code;
- a JSON-pointer-like path;
- a human-readable diagnostic that is not used for program logic.

Required v0 checks include:

- supported SIR version;
- valid proposal, workload, transaction, and artifact identifiers;
- at least one mutation;
- unique workload creation within a proposal;
- valid HTTP port, path, and timeout;
- health constraints refer to a workload addressed by the proposal;
- irreversible permission is consistent with the maximum effect class.

Validation does not inspect live system state. Generation freshness, object
existence, artifact resolution, and resource feasibility are planner checks.

## 5. Effects

Effect classes are ordered:

```text
read_only < locally_reversible < compensatable < irreversible
```

SIR v0 Workload mutations require at most `locally_reversible`. The higher
classes exist in the wire model so later protocol revisions can evolve without
changing the intent envelope.

## 6. Compatibility

Within SIR major version 0, new enum variants and fields are not assumed to be
forward compatible because unknown fields are rejected. A gateway must obtain
the exact schema exposed by the connected Noema system.
