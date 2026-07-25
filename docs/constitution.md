# Noema Constitution

Status: Normative draft 0.1

This document defines the invariants that implementations must preserve even
when a user, model, workload, or external input is wrong. Product behavior may
evolve; these boundaries must only change through an explicit specification
revision.

## 1. Authority boundary

The cloud model is a proposal author, not an executor and not an observer. It
may write Intent SIR. It may not write Execution IR, Evidence IR, observed
state, or the current-generation pointer.

## 2. State, not commands

Intent SIR describes desired system objects and constraints. It must not
contain a shell program, an arbitrary executable payload, or an escape hatch
that is semantically equivalent to unrestricted command execution.

## 3. Local deterministic control

Validation, planning, invariant enforcement, execution, observation, commit,
and rollback are local deterministic operations. Loss of cloud connectivity
must not stop already committed workloads from converging toward their desired
state.

## 4. Validation before effects

A proposal that fails syntax, schema, semantic, generation, or invariant
validation produces no execution plan and no system side effect. Validation
must be a pure operation over the proposal and a versioned state snapshot.

## 5. Versioned reality

Every committed mutation creates a new monotonically identified generation.
Execution occurs against a candidate generation. The current-generation
pointer changes only after required checks succeed.

## 6. Facts require evidence

Observed state can only be written by trusted local observers. Every observed
fact records its source. Model hypotheses and derived conclusions remain
distinct from observed facts.

## 7. Explicit effects

Plans classify effects as read-only, locally reversible, compensatable, or
irreversible. Irreversible effects must be declared and must occur only after
all applicable reversible validation has succeeded.

## 8. Recoverability

No ordinary proposal may remove the last bootable, internally consistent
generation. A failed candidate must not replace a healthy current generation.

## 9. Evidence continuity

Every accepted proposal receives a stable identifier. Planning, execution,
observation, rejection, commit, and rollback records remain causally linked to
that identifier and to the generation on which the proposal was based.

## 10. Compatibility isolation

Shells and traditional Linux utilities may exist in a legacy workload or a
recovery environment. They are not part of the normal host control path and
must not bypass Noema state ownership.

## 11. Information boundary

Secret material is not model context. The gateway may expose a reference or a
limited operation that uses a secret locally, but it must not expose the
secret value to the cloud model.

## 12. Host safety during development

Until a release is explicitly promoted for hardware testing, Noema system
effects are restricted to the simulation backend, the dedicated Docker test
environment, and disposable QEMU virtual machines.
