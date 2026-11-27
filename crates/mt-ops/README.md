# mt-ops

The operation model of a base: what clients send, how the base decides it, and how clients
keep a copy that converges. Pure (no clocks, sockets or randomness); builds for
`wasm32-unknown-unknown` so the web app uses the same code.

## Public API

| Item | What it is |
|---|---|
| `Op` | One change: `Create` (record with values), `Set` (one cell, with the cell version seen), `Transition` (one cell, with the record version seen), `Delete` (with the record version seen), `Restore` |
| `Batch` | A client's operations, applied all or none: `id: ClientOpId`, `base`, `ops` |
| `ClientOpId` | `{client, seq}` — the client's own increasing number for a batch |
| `BaseState` | Records with record and cell versions, the base sequence, and the batches recently decided. `step(batch) -> Decision`, `replay(accepted)`, `apply_local(batch)`, `from_records`, `record`, `records`, `outcome` |
| `Decision` | `{outcome, repeated}` |
| `Outcome` | `Accepted {id, seq, effects}` or `Rejected {id, op, reason}` |
| `Effect` | `Created`, `Set {before, after, overwritten}`, `Deleted`, `Restored` |
| `Reason` | Why a batch was refused, with `text()` |
| `Replica` | A client's copy: `submit(ops) -> Batch`, `receive(outcome)`, `view()`, `confirmed()`, `pending()` |
| `codec` | `encode_client` / `decode_client`, `encode_server` / `decode_server` for `ClientMessage` (`Submit`, `CatchUp`) and `ServerMessage` (`Outcome`, `Reload`): `[version][postcard]` frames |

## Invariants (each tested)

* **All or nothing.** A batch applies completely with the next sequence number, or nothing of
  it applies and the sequence does not move.
* **No gaps, no repeats.** Accepted batches are numbered 1, 2, 3, … per base. A batch received
  again (same `ClientOpId`) gets the outcome it got the first time and changes nothing; a
  refusal is remembered the same way. An older number that is no longer remembered is
  refused (`TooOld`), never applied. The window holds the last 4,096 decisions.
* **Conflicts per cell.** A `Set` on a cell version the client had not seen applies (the
  last writer wins) and its effect names the overwritten value, so the client can be told.
  The client's own earlier change in the same batch is not a conflict. Deliberate changes —
  `Transition`, `Delete` — are refused on a stale record version instead.
* **Versions.** A cell's version is the sequence of the batch that set it (0 = never set); a
  record's version is the sequence of its last change. Setting the value a cell already has
  is no change. An empty value removes the cell.
* **Replay.** Accepted effects replayed in order on a copy give the same records (the
  journal, followers, clients); a replay that does not fit changes nothing.
* **Convergence.** A replica shows the confirmed state with its pending batches applied on
  top (rebased on every change from the server; batches that no longer apply drop out of
  the view). When every batch is answered, every replica equals the server — under
  concurrent edits, resends and reconnects with catch-up.
* **Codec.** Values travel as canonical JSON text inside postcard (a value may hold arbitrary
  JSON). Decoding never panics; it refuses empty, oversized (> 4 MiB), unknown-version,
  truncated and trailing-byte frames and messages over the limits (500 operations, 400
  cells per record).

## Tests

| File | Category | What |
|---|---|---|
| `tests/state.rs` | standard, edge, invalid | Versions and sequences, conflicts and overwrites, deliberate changes, repeats, atomicity, the bounded window, sequence exhaustion, replay and refused replays, JSON form |
| `tests/model.rs` | model-based | 512 random runs of batches from three clients (with repeats and stale numbers) against a naive reference: same decisions, same records, same sequence, replay equal, no gaps |
| `tests/replica.rs` | standard, concurrency | Optimistic view, rebase, refusals, gaps; seeded simulation (300 seeds × 400 steps, 3 long runs of 5,000) of clients editing together with resends and reconnects: every replica ends equal to the server, no batch applied twice |
| `tests/codec.rs` | property, invalid | Round trips of every message in binary and JSON (all value shapes), arbitrary bytes, truncated and extended frames, every limit |

Cross-platform: `cargo build -p mt-ops --target wasm32-unknown-unknown`.
