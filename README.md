# Noema Linux

Noema Linux is an experimental Linux userspace whose primary control interface
is desired state rather than shell commands.

The cloud model proposes a versioned System Intent Representation (SIR). A
local, deterministic Rust control plane validates the proposal, compiles it
into an execution plan, observes the result, and either commits a new system
generation or preserves the previous one.

## Status

Noema has completed its M3 container-backed process loop. It is not yet an
operating system image and must not be used to manage a host machine.

The Rust workspace now contains a deterministic planner, transactional state
core, virtual and real-process execution backends, and reconciler. Real
processes are exercised only inside the Docker laboratory or explicit tests:

- Intent SIR: model-authored desired-state proposals.
- Execution IR: locally-authored deterministic plans.
- Evidence IR: locally-authored observations and outcomes.
- State generations: isolated candidates with commit, abort, and causal events.
- Simulation: virtual Workloads with crash, timeout, and health-failure injection.
- Reconciliation: commit/rollback plus recovery after observed runtime drift.
- Process execution: built-in Rust Workloads, HTTP health probes, SIGTERM, and reaping.
- Persistence: validated generation and event snapshots with atomic replacement.

See [plan.md](plan.md), [the constitution](docs/constitution.md), and the
[SIR v0 specification](specs/sir-v0.md).

## Development

```bash
cargo xtask check
docker compose -f docker/compose.yaml up --build \
  --abort-on-container-exit --exit-code-from m3-scenario
docker compose -f docker/compose.yaml down --volumes --remove-orphans
```

Docker is the daily integration environment. QEMU is reserved for behavior
that requires a real boot, PID 1, system cgroups, or generation rollback.
