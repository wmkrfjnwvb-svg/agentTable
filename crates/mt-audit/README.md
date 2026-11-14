# mt-audit

Trace and audit primitives: the parts that let the platform answer "what happened, who did it, on
whose behalf, did it reach everyone, and is the record still the one we wrote?".

Pure and **sans-IO**: no clocks, sockets, threads or randomness. Time, span ids and keys are inputs;
storage is a trait. It builds for `wasm32-unknown-unknown`, so the browser can verify a chain and
track delivery with the same code as the server.

## Modules

| Module | Purpose |
|---|---|
| `trace` | W3C Trace Context (`traceparent`): strict parsing, formatting, child-span derivation. One trace id follows an operation from the browser through the server, rules, workflows and outbound calls |
| `delivery` | Delivery states of an operation, like message ticks in a chat app: `created → sent → received → validated → committed(seq) / rejected → broadcast → applied(by n clients)`. `step` is the pure transition function; `DeliveryLedger` is the idempotent server-side table; `SeqTracker` finds gaps in the sequence a client has seen |
| `canonical` | Canonical JSON (a strict subset of RFC 8785): the one byte form of audit details that is hashed. Floats are rejected |
| `chain` | Hash-chained audit log per tenant. Each entry records actor **and** initiator (dual identity), action, object, details, time and trace id, and commits to the previous entry's hash. `AuditStore` is the storage trait; `InMemoryAuditStore` is the reference implementation |
| `checkpoint` | Signed statements "chain X had hash H at sequence N". HMAC-SHA256 is built in; `CheckpointSigner` / `CheckpointVerifier` let a KMS or an asymmetric key replace it |
| `verify` | Verification that names the kind and position of tampering: modified, broken link, removed range, reordered, duplicated, forked, checkpoint mismatch, truncated, bad checkpoint signature |
| `privacy` | Optional minimisation: values of classified fields are stored in the audit as keyed digests, so the chain never has to be rewritten for a deletion request while "changed / unchanged" stays visible |
| `process` | `ProcessContract` (terminal events, reopen events, follow-up rules with deadlines, maximum duration, stuck threshold) and `CompletenessVerifier`, which reads the event journal incrementally and classifies every process instance as complete, in progress, late or broken |
| `exception` | `ExceptionDescriptor` (unassigned, overdue, failed action, unmapped value, missing submission, quality, AI quality, stuck process, missing step, undelivered operation, audit integrity) and its status machine `open → acknowledged → in progress → resolved / dismissed → reopen` |

## Invariants (asserted by tests)

1. **Any change to stored history is detected and located.** Flipping any single byte of any field
   of any entry, and removing, swapping, duplicating or forking entries, is reported with the right
   kind and position (property tests over random chains).
2. **A consistently rewritten suffix is caught by the first checkpoint that covers it**, and the
   verdict names the range in which the forgery lies. The verdict does not depend on the order in
   which a store returns its checkpoints.
3. **The hash preimage is injective**: every variable-length field is length-prefixed and every
   domain (genesis, entry, checkpoint, value digest) has its own label, so entries cannot be spliced
   between tenants or reinterpreted.
4. **Appending is linear under concurrency.** A writer that loses the race gets a conflict and
   prepares again on the new head; simulated interleavings of many writers always produce one valid
   chain with every entry exactly once.
5. **Delivery events are idempotent and illegal states are unreachable.** Replaying any event is a
   no-op; an event that contradicts the state is an error that changes nothing; a sequence number
   belongs to one operation (model test against the transition table, random runs).
6. **The completeness verifier is incremental and replay-safe**: feeding the journal in any
   batching, with overlaps, or after a serialise / deserialise round trip gives the same reports
   (property test).
7. **Canonical JSON is a fixed point**: `canonical(parse(canonical(v))) == canonical(v)`.
8. **Keys never print** (`Debug` is redacted) and signatures are compared in constant time.

## What a hash chain cannot do (stated plainly)

- It cannot show that the **unsigned tail** was rewritten or cut off; only a checkpoint or an
  externally remembered head can. Checkpoint often, and export checkpoints outside the database.
- An attacker who can delete entries **and** every checkpoint leaves no trace in the store itself.
- HMAC checkpoints prove integrity only to holders of the key. For third-party verifiability
  implement `CheckpointSigner` with an asymmetric key.
- `Privacy::HashClassified` hides values from people without the tenant key; it is pseudonymisation,
  not anonymisation.

## Relation to other crates

- `mt-core` owns provenance on cells (`CellMeta`, `Actor`); this crate owns the tenant-wide log.
  Both carry the same dual identity (actor + initiator).
- `ClientOpId` and `TraceId` here are plain 128-bit values; `mt-core`'s typed `OpId` / `TraceId`
  convert to and from them with `as_u128` / `from_u128`.
- `mt-store` implements `AuditStore` on SQLite, MySQL and PostgreSQL; `mt-lifecycle` and
  `mt-workflow` emit the `ProcessEvent`s that `CompletenessVerifier` consumes.

## Tests

| File | What | Count |
|---|---|---|
| `tests/chain.rs` | Append, store contract, checkpoints, verification, every tamper kind, privacy, canonical JSON, simulated concurrent appends | 27 |
| `tests/delivery.rs` | Transition table model test, ledger, sequence tracker, random runs | 15 |
| `tests/process.rs` | Contracts, completeness classification, exceptions, incremental and resumed ingestion | 15 |
| `tests/properties.rs` | Tamper detection and localisation, canonical JSON round trip | 4 |
| `tests/trace.rs` | `traceparent` parsing and formatting, span derivation | 12 |

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-audit
cargo fmt -p mt-audit --check
cargo clippy -p mt-audit --all-targets -- -D warnings
cargo test -p mt-audit
cargo build -p mt-audit --target wasm32-unknown-unknown
cargo llvm-cov -p mt-audit --summary-only
```
