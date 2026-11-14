# Logic audit — wave 1 (`mt-core`, `mt-cluster`, `mt-audit`)

Audit date: 2026-11-14. Question asked of every module: *is the behaviour right for the business and
sound in its reasoning* — independent of whether it matches the specification text. Focus: money and
time correctness, identity and provenance, ordering and uniqueness, concurrency hazards, failure
handling, and security of anything that crosses a trust boundary.

Method:

- `mt-cluster`: every source file read line by line; each state transition traced by hand; then
  tests written to pin the conclusions (hand-driven scenarios, model tests, simulations).
- `mt-audit`: `chain`, `checkpoint`, `verify` and `process` read line by line; `delivery`,
  `exception`, `privacy`, `trace`, `canonical` reviewed against their transition tables and
  contracts and against their tests.
- `mt-core`: reviewed while it was written; conclusions pinned by 14 property tests (ordering,
  round trips, conversion matrix) in addition to unit tests. `numbering` re-read for this audit.

## 1. Findings

Severity: **High** = wrong business result possible; **Medium** = wrong or surprising result in a
realistic situation, or an avoidable availability cost; **Low** = hardening; **Info** = behaviour
that is right but must be known by later waves.

| ID | Sev. | Module | Finding | Resolution |
|---|---|---|---|---|
| L1-01 | High | `mt-core::numbering` | A pattern with a month but no year (`INV-{MM}-{seq}`) restarted its counter each month while the text showed only the month, so May 2025 and May 2026 produced the same numbers. The same for `{DD}` without `{MM}` | **Fixed.** Such patterns are rejected: the text must show the whole period the counter restarts in |
| L1-02 | High | `mt-core::numbering` | With one "current period + next value" per sequence, a back-dated record (dated in an earlier period) would restart that period at 1 and reuse numbers | **Fixed** by the API shape: `allocate` takes the counter *of the date's period*; storage keeps one counter per period key. Test covers back-dating |
| L1-03 | Medium | `mt-cluster::membership` | A probe was tied to a stable node id. If the node restarted while a probe of its old instance was pending, the timeout marked the *new* instance suspect although it had never been pinged — needless suspicion and gossip right after every restart | **Fixed.** A probe is tied to one instance and dropped when that instance is replaced or declared gone |
| L1-04 | Medium | `mt-cluster::scheduler` | After an idempotent re-claim, the scheduler assumed the lease expired at `now + ttl`; the stored expiry was earlier. It would renew too late and could lose the lease for no reason | **Fixed.** The scheduler tracks the stored expiry |
| L1-05 | Medium | `mt-cluster::scheduler` | `advance` moves the fire cursor before the caller fires the window. A holder that crashes in between never fires it: at-most-once, which is wrong for SLA timers and scheduled jobs | **Contract documented; must be honoured in wave 3.** The store inserts the due jobs into the durable queue in the same transaction as the cursor move. Tracked as a wave-3 acceptance test: "crash between advance and fire loses nothing" |
| L1-06 | Low | `mt-cluster::auth`, `mt-audit::checkpoint` | Secret keys were stored as bytes in types that derived `PartialEq`, so any `==` on them would compare key material in non-constant time | **Fixed.** Both hold a keyed HMAC state and are no longer comparable. Residual: `mt-audit::Privacy::HashClassified` still holds its key as bytes inside a comparable configuration value; it is only ever compared with locally held configuration, never with outside input. Accepted; revisit if that changes |
| L1-07 | Low | `mt-audit::verify` | The verdict depended on the order in which the store returned checkpoints. A shuffled list could not hide a forgery, but it blurred *where* it was (a later checkpoint was blamed and the last trustworthy position fell back to the start) | **Fixed.** Checkpoints are sorted before verification; test added |
| L1-08 | Low | `mt-cluster::codec` | `encode` had a size check that no valid message could ever trigger (dead code that suggested a possibility that does not exist) | **Fixed.** Replaced by a compile-time assertion that the largest valid message fits a frame, plus a test that encodes that largest message |
| L1-09 | Info | `mt-cluster::membership` | `has_majority` is false for the survivor of a two-node cluster (1 active vs 1 dead is not a strict majority) and stays false until the tombstone expires | **By design** (it is a brake against ownership ping-pong, not a safety mechanism). Wave 4 decision recorded: the server must not gate ownership on it for clusters of fewer than three nodes |
| L1-10 | Info | `mt-cluster::auth` | The handshake authenticates both peers and resists replay, but does not bind to the transport. Without TLS an on-path attacker can relay a handshake and then alter traffic | **Documented.** Wave 4: cluster links run over TLS; plaintext only on loopback |
| L1-11 | Info | `mt-cluster::fencing` | `Fencer::acquire` takes ownership from whoever holds it. That is correct only when routing decided this node should own the resource | **Documented** on the function; the routing rule "the current holder keeps the resource while it is an active member" (`RoutingTable::route`) is tested, including the no-stealing case |
| L1-12 | Info | `mt-audit::chain` | Entry times are not required to increase: two nodes with slightly different clocks may append in either order. Order is defined by `seq`, not by time | **By design.** Consumers must sort by `seq`. Noted for the trace explorer |
| L1-13 | Low | `mt-audit::chain` | The in-memory store cannot check that entry 1 links to the genesis hash (it does not know the chain id). Verification catches a wrong first link | **Tracked for wave 3:** the SQL store knows the tenant and must refuse a first entry that does not link to genesis |
| L1-14 | Info | `mt-audit` / `mt-core` | Both crates have a 128-bit operation id and trace id type (`mt-audit` is deliberately free of dependencies) | **Documented:** conversion through `as_u128` / `from_u128`; `mt-ops` (wave 3) is the single place that converts |

Found and fixed earlier in the wave, listed for completeness:

| ID | Sev. | Module | Finding | Resolution |
|---|---|---|---|---|
| L1-15 | High | `mt-core::coerce`, `convert` | The decimal library rounds silently when a product needs more than 28 decimal places; a percent typed with many decimals lost digits without notice | Fixed: such input is rejected (`points_to_fraction`) |
| L1-16 | Medium | `mt-core::convert` | An empty list converted to JSON became `[]`, while every other empty value became null: "empty" meant two things | Fixed: every empty value converts to null first (found by a property test) |
| L1-17 | Low | `mt-core::convert` | Text → JSON → text did not return the original text | Fixed: a JSON string converts to its content |
| L1-18 | Low | `mt-core::pattern` | Error position was off by one when a pattern ended inside `{…}` | Fixed |

## 2. Reasoning checks that passed

**Money and numbers.** No floats anywhere (denied by lint). Money carries its currency and is rounded
only by an explicit rounding mode to the currency's minor units (JPY 0, USD 2, KWD 3). Percent is a
fraction internally, so `25 %` of `200` is `0.25 × 200` without a hidden division. Comparison of money
in different currencies never claims an order by amount (currency first).

**Time.** All instants are UTC milliseconds; dates are civil dates without a zone; "the record's
date" is decided by the caller (base time zone), never by reading a clock. Durations cannot go
negative through arithmetic (checked operations return errors).

**Ordering.** `sort_key` bytes order exactly like `compare_values` for any two values (property
test), which is what makes the three databases sort identically. Nulls first, then by type rank.

**Identity and provenance.** An AI-produced value always names the human or rule that initiated it;
only AI values can be suggestions; a suggestion cannot be accepted by the producer that made it; a
review cannot predate the value. The audit entry carries the same two identities.

**Cluster safety argument.** Two nodes can believe they own a base (views differ during a
partition). Only one can hold the stored `(epoch, owner)`; the store checks it in the write
transaction; the epoch only grows; a release clears the owner without lowering the epoch. Therefore
no interleaving lets two instances write under one epoch. The same argument with `(term, holder,
cursor)` shows fire windows cannot overlap. Both were tested three ways: model test with random
operation sequences and store failures, a real 8-thread race, and whole-cluster simulations with
partitions, crashes and restarts.

**Cluster liveness argument.** Suspicion is refuted by a version bump that only the suspected node
can make; a node told it is dead takes a new incarnation and rejoins; anti-entropy also contacts
dead and departed members, so a healed partition is always noticed. Tested by simulation
(convergence after faults heal; bounded detection time; no false deaths on a lossless network).

**Audit soundness.** The hash preimage is injective (length prefixes, domain labels), the genesis
hash binds the tenant, and checkpoints bind chain id, position, time, algorithm and key id. What a
hash chain cannot prove (an unsigned tail, deletion of all checkpoints) is stated in the README
rather than implied away.

**Process completeness.** "Broken" outranks "late"; a required follow-up satisfies all open
obligations of its rule; reopening makes an instance unfinished again; ingestion is replay-safe.
These match how a support or procurement team would judge the same situations.

## 3. Items handed to later waves

| Item | Wave | Check to write there |
|---|---|---|
| Outbox in the same transaction as the fire-cursor move (L1-05) | 3 | Crash between `advance` and firing loses no timer and fires none twice |
| `check_token` inside every guarded write transaction | 3 | A write with a stale token is rejected on all three databases under concurrency |
| SQL audit store refuses a first entry not linked to genesis (L1-13) | 3 | Direct test |
| Sequence counters: one row per (sequence, period), allocated in the record's transaction | 3 | Concurrent creates on three databases give unique, gap-free numbers |
| Cluster links over TLS; no ownership gating on `has_majority` below three nodes (L1-09, L1-10) | 4 | Configuration tests |
| Incarnation = start time in ms or a persisted counter | 4 | Restart test in the server |
| Condition type shared by validation, visibility, filters, rules (R1-07) | 2 | One evaluator; SQL and in-memory results agree (property test) |
