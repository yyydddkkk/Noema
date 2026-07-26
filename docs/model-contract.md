# Noema model contract v0

The model boundary is a versioned data protocol, not a natural-language shell
adapter. A cloud model receives one complete `ContractRequest` and may return
one `ContractReply` containing Intent SIR. It never receives an executor,
terminal, tool, state-store handle, or capability to author observations.

```text
typed WorldState + bounded causal event references + objective
  -> ContractBuilder
  -> ContractRequest + generated ContractReply JSON Schema
  -> ModelProvider (untrusted remote computation)
  -> opaque bytes
  -> Gateway
  -> validated Intent SIR only
```

## Request boundary

`noema-contract` explicitly maps trusted local state into a read-only model
view. It does not serialize `WorldState` wholesale. The view contains only:

- Workload identity and artifact reference;
- desired and observed state as separate typed fields;
- typed health and restart policies;
- the generation in which the object last changed;
- at most eight causal event references per visible Workload.

Transaction identifiers, candidate lifecycle events, raw logs, process output,
filesystem contents, and arbitrary kernel state are absent. URL-style artifact
references containing userinfo are rejected before disclosure. Callers must
still treat objective text and ordinary artifact names as cloud-disclosed data
and must not put secrets in them. The request limits objective, Workload count,
evidence count, serialized request size, and reply size. Its reply schema is
generated from the same Rust `ContractReply` and `IntentSir` types used by the
decoder.

The objective is data inside the contract. It cannot add capabilities or alter
the rule/capability fields. Prompt text is not itself a security boundary; all
provider output remains untrusted until local decoding and validation finish.

## Reply admission order

`noema-protocol::Gateway` applies checks in this order, before any planner,
state transaction, or executor can observe the result:

1. provider/transport success;
2. reply byte limit before JSON parsing;
3. strict JSON decoding with unknown fields denied;
4. exact `request_id` binding;
5. exact `base_generation` binding;
6. pure Intent SIR semantic validation.

The gateway returns only `IntentSir`. It deliberately owns no generation store,
planner, reconciler, or execution backend, so rejection has no state side
effects by construction.

## Provider boundary

`ModelProvider` is replaceable and intentionally narrow: complete contract in,
opaque response bytes out. `DeterministicMockProvider` is a protocol test double
and performs no local inference.

The first opt-in cloud adapter uses OpenAI's Responses API and is compiled only
when `noema-protocol` enables its `openai` Cargo feature. It:

- accepts the API key and model identifier only through explicit construction;
- fixes the destination to the official HTTPS Responses endpoint;
- sends no tools and sets `store` to `false`;
- supplies the generated reply schema as strict Structured Outputs;
- bounds the HTTP response body;
- rejects incomplete responses, refusals, missing text, and ambiguous multiple
  text outputs;
- returns extracted bytes to the same provider-independent Gateway checks.

Constructing the provider does not perform a request. No default test reads an
API key or accesses the network. Production deployments should pin a tested
model snapshot and run model compatibility evaluations before changing it.

OpenAI API shape was checked against the official
[Structured Outputs guide](https://developers.openai.com/api/docs/guides/structured-outputs)
and [text generation guide](https://developers.openai.com/api/docs/guides/text).

## What this does not prove

Passing schema and protocol tests proves the trust boundary and deterministic
admission behavior. It does not prove that a particular untrained cloud model
understands every future Noema objective. That is an opt-in compatibility
evaluation with bounded requests, tokens, cost, timeout, and an explicit data
disclosure decision.
