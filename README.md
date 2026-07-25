# Noema Linux

Noema Linux is an experimental Linux userspace whose primary control interface
is desired state rather than shell commands.

The cloud model proposes a versioned System Intent Representation (SIR). A
local, deterministic Rust control plane validates the proposal, compiles it
into an execution plan, observes the result, and either commits a new system
generation or preserves the previous one.

## Status

Noema has completed its M2 in-memory simulation loop. It is not yet an
operating system image and must not be used to manage a host machine.

The Rust workspace now contains a deterministic planner, transactional state
core, virtual execution backend, and reconciler. Together they exercise these
boundaries without spawning processes or changing the host:

- Intent SIR: model-authored desired-state proposals.
- Execution IR: locally-authored deterministic plans.
- Evidence IR: locally-authored observations and outcomes.
- State generations: isolated candidates with commit, abort, and causal events.
- Simulation: virtual Workloads with crash, timeout, and health-failure injection.
- Reconciliation: commit/rollback plus recovery after observed runtime drift.

See [plan.md](plan.md), [the constitution](docs/constitution.md), and the
[SIR v0 specification](specs/sir-v0.md).

## Development

```bash
cargo xtask check
```

Docker is the daily integration environment. QEMU is reserved for behavior
that requires a real boot, PID 1, system cgroups, or generation rollback.
