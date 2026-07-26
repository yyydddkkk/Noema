# Live cloud-model compatibility evaluation

The M4 live evaluation is a deliberately separate binary. It is not compiled
by default and is never part of normal unit, Docker, or host-management paths.
Its only success condition is narrow: an explicitly chosen cloud-model snapshot
must turn the fixed, empty-world Noema Contract into exactly one valid Workload
creation Intent, which the local simulation reconciler must commit.

## Fixed boundary

One invocation of `--live` has these non-configurable limits:

- exactly one HTTPS request and no retries;
- the official OpenAI Responses endpoint only;
- no tools and no remote conversation state;
- one fixed objective and an initially empty world view;
- 2,048 maximum output tokens;
- 30-second total HTTP timeout;
- the normal Contract request/reply byte limits;
- strict Gateway and additional scenario-specific mutation checks;
- Simulation backend only, with no host or container side effects.

The model identifier has no default. Use a snapshot that has been reviewed for
the evaluation rather than a moving alias.

## Dry run

Supply the selected model's current non-cached input and output prices in USD
per million tokens, plus an operator budget. Noema does not embed a price table
because prices can change independently of this repository.

```bash
export NOEMA_OPENAI_MODEL='<reviewed-model-snapshot>'
export NOEMA_OPENAI_INPUT_USD_PER_MILLION='<current-input-price>'
export NOEMA_OPENAI_OUTPUT_USD_PER_MILLION='<current-output-price>'
export NOEMA_LIVE_EVAL_MAX_USD='<maximum-one-run-budget>'

cargo run -p noema-live-eval --features live-eval -- --dry-run
```

Dry-run constructs the exact provider request body but does not read
`OPENAI_API_KEY` and does not send anything. For a deliberately conservative
preflight estimate, the encoded request byte count plus a 4,096-token framing
safety margin is treated as the maximum input-token count. The configured full
input price and maximum output-token count produce the displayed cost bound.

This is a client-side safety check, not a substitute for an account-level spend
limit or the provider's authoritative usage record.

## One live request

Provide the API key through the process environment or a secret manager. Do not
put it in a command argument, repository file, shell script, Docker image, or
CI configuration. After reviewing dry-run output, set the exact acknowledgement:

```bash
export OPENAI_API_KEY='<secret supplied outside the repository>'
export NOEMA_LIVE_EVAL='ONE_REQUEST_TO_OPENAI'

cargo run -p noema-live-eval --features live-eval -- --live
```

Before transport, the runner recomputes and enforces the configured cost bound.
After transport, it requires the ordinary Gateway checks and then additionally
requires exactly one `CreateWorkload` mutation for:

```text
id:       hello
artifact: builtin:noema-test-workload
desired:  running
effect:   locally_reversible
```

Only then is the Intent passed to an in-memory Simulation reconciler. Success is
reported as `LIVE_EVAL_OK`; failures never print the API key or raw model reply.
