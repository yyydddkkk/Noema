# M2 simulation architecture

M2 proves Noema's control semantics without making Linux system calls. It is a
deterministic laboratory for the control plane, not a production executor.

## Boundaries

The cloud-authored input ends at Intent SIR. `noema-planner` validates that
input against an immutable state snapshot and emits Execution IR containing
only typed actions. It has no I/O or mutable state.

`noema-reconciler` allocates two candidate snapshots for a valid plan:

- a candidate world-state generation;
- a clone of the committed simulation runtime.

`noema-executor` applies typed runtime actions only to the candidate runtime.
If execution or an invariant fails, both candidates are abandoned. If every
check passes, the state generation and matching runtime snapshot are committed
together and Evidence IR records the observations and transitions.

## Fault model

The simulator supports deterministic crash-on-start, start-timeout, and
health-check-failure injection. It can also crash an already committed virtual
Workload so the Reconciler must observe drift and restore the desired state.

Fault results are local observations. They cannot be authored through Intent
SIR and they do not directly change desired state.

## Important generation rule

Candidate generation identifiers are allocated by the state store and are not
reused after rollback. Execution IR therefore requests creation or commit of a
candidate without predicting its numeric identifier. Evidence IR records the
identifier actually allocated and committed.
