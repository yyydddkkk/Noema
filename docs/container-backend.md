# M3 container and process backend

M3 moves the M2 control loop from virtual Workloads to real Linux child
processes. Docker is the disposable system boundary around this experiment;
the Process backend itself uses typed Rust APIs and never controls Docker.

## Execution transaction

`ExecutionBackend` defines explicit begin, commit, and rollback operations.
The Simulation backend stages a cloned runtime state. The Process backend
records prepared objects, running process membership, and failed observations.
On rollback it terminates newly started children and restores the prior
membership before the state candidate is abandoned.

Only four built-in artifact references are accepted in M3. They select the
normal Rust test Workload or deterministic crash, startup-timeout, and
unhealthy behaviors. Artifact text is never evaluated as a path, command, or
Shell program.

Child processes receive SIGTERM and are polled and reaped. SIGKILL is used only
after a one-second graceful-stop deadline. HTTP probes connect only to the
loopback address and use the validated port, path, and timeout from `HealthSpec`.

## Persistence

The generation store writes current state, allocator positions, and causal
events to a temporary JSON file, flushes it, and atomically renames it over the
active snapshot. Loading rejects non-monotonic generation allocators and event
sequences. JSON is the M3 prototype format, not a final storage decision.

The Docker scenario saves this snapshot in the explicitly named
`noema-m3-state` volume. A second container loads generation 2, observes that
the real process runtime is absent, restarts the desired Workload, and commits
generation 3.

## Container boundary

The Compose service has a read-only root filesystem, no network, no Docker
socket, no Linux capabilities, `no-new-privileges`, a 64-PID limit, and small
tmpfs mounts for `/run` and `/tmp`. It runs as UID/GID 65534. The final image
contains only the release scenario binary, the release test Workload, and the
Debian runtime files; Rust tooling and source remain in the builder stage.
