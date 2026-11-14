# mt-cluster

Clustering for the single binary: every node runs the same program, there is no leader and no
external coordinator. Nodes find each other by gossip, agree on who owns what by hashing, and are
kept from writing over each other by fencing epochs stored in the database they already share.

The crate is **sans-IO**: no sockets, clocks, threads or random sources. Time, random seeds and
nonces are inputs, and storage is a trait. The server wires it to TCP/TLS and the tenant database;
the simulator wires it to a virtual network and a virtual clock. It builds for
`wasm32-unknown-unknown`.

## Design in one page

| Concern | Mechanism | Industry reference |
|---|---|---|
| Who is in the cluster | SWIM failure detector with suspicion, incarnations and anti-entropy sync (`membership`) | SWIM paper; HashiCorp memberlist / Serf; Cassandra gossip |
| Who should own a base or a shard | Rendezvous (highest-random-weight) hashing with integer weights (`placement`) | Rendezvous hashing (Thaler & Ravishankar); used by Riak, Twitter, GitHub load balancer |
| Who may actually write | Fencing epoch per resource, raised by compare-and-set in the shared store; every write carries the token (`fencing`) | Kleppmann, "How to do distributed locking"; ZooKeeper zxid fencing; Kafka controller epoch |
| Timers, SLA clocks, cron | Per-shard lease with a term and a fire cursor moved by compare-and-set (`scheduler`) | Kubernetes Lease objects; Quartz clustered scheduler; Temporal shard ownership |
| Who may connect | Three-message mutual HMAC-SHA256 handshake with nonces (`auth`) | SIGMA-style challenge-response; Erlang cookie done properly |
| Wire format | postcard frames with a version byte and hard limits checked on both sides (`codec`) | Length-limited framing as in memberlist |
| Proof that it works | Deterministic simulation: seeded network with delay, loss, duplication, partitions, crashes and restarts (`sim`) | FoundationDB simulation; TigerBeetle VOPR; Jepsen-style invariants |

**Safety never depends on membership.** Views can disagree during a partition, so two nodes can
both believe they own a base. That is harmless: only the node whose epoch is the stored one gets
its writes accepted, because the store checks the token inside the write transaction. Membership
and placement only decide who *tries*.

## Public API

| Item | What it does |
|---|---|
| `Membership::new(cfg, me, seed)` / `handle(now_ms, Input) -> Vec<Output>` | The membership state machine. Inputs: `Tick`, `Message`, `Join`, `Leave`. Outputs: messages to send and `Event`s (`MemberUp`, `Suspect`, `Down`, `Left`, `Renewed`) |
| `Membership::active_nodes`, `placement`, `has_majority`, `member`, `members` | The local view |
| `Placement::new(nodes)`, `owner(key)`, `ranked(key)`, `moved_keys(keys, before, after)` | Deterministic ownership; `ranked` is the fail-over order |
| `RoutingTable::route(resource, record, held_epoch) -> Route` | `Local { must_acquire }`, `Forward { node }` or `Unavailable`; `fan_out` splits subscriber nodes into local, remote and gone |
| `Fencer::acquire / release / token / on_fenced / handoff_candidates / begin_drain / reset` | Node-side ownership of epochs |
| `check_token(store, token)` | The check a store runs inside every guarded write |
| `EpochStore`, `LeaseStore` | Compare-and-set storage traits (implemented over SQL in `mt-store`); `MemoryStore` for tests |
| `claim / renew / release / advance` and `ShardScheduler::tick / fire / reset` | Shard leases and fire windows |
| `ClusterSecret::new`, `Handshake::initiator / responder / handle / peer` | Peer authentication |
| `encode`, `decode`, `codec::validate` | Wire frames |
| `sim::{SimCluster, SimNet, NetConfig}` (feature `sim`) | The deterministic simulator |

## Invariants (each one is asserted by tests)

1. **Single writer.** For any `(resource, epoch)` at most one node instance ever has a write
   accepted, and accepted epochs never decrease. Checked by a model test, an 8-thread race test
   and the fault-injection simulation.
2. **No timer fires twice.** Fire windows of a shard tile the timeline: each window starts where
   the previous one ended, across lease terms and holders. Checked by a model test and the
   simulation.
3. **Convergence.** After faults stop, every running node's active set equals the set of running
   nodes. The view depends only on the set of facts received, not on order, grouping or repetition
   (property test).
4. **Bounded detection.** A crashed node is declared dead by every other node within
   `probe_interval × members + suspect_timeout` plus dissemination time (simulation).
5. **No false deaths on a healthy network.** With delay and duplication but no loss, no running
   node is ever declared dead (simulation).
6. **Minimal movement.** Removing a node moves only its keys; adding a node moves keys only to it;
   raising a weight only attracts keys (property tests). A restart moves no key to another node.
7. **Events are a faithful change feed.** Replaying `MemberUp` / `Down` / `Left` reproduces the
   active set exactly (property test).
8. **A node's identity only moves forward**: incarnation and version never decrease, whatever it
   is told (property test).
9. **The decoder never panics** and never returns a message that breaks a limit (fuzz tests);
   every valid message fits a frame (compile-time assertion plus a test of the largest message).
10. **Authentication.** A peer without the secret cannot complete a handshake; tampering with any
    field, replaying a recorded handshake to a fresh nonce, or connecting to oneself fails; after
    any error the handshake stays failed.
11. **Determinism.** The same seed gives the same run, byte for byte (replay test). Hash values
    and the wire layout are pinned by golden tests, because nodes of different versions must agree
    on them during a rolling upgrade.

## Operating rules for the caller

- **Incarnations must grow across restarts.** Use the start time in milliseconds (the simulator
  does) or a persisted counter. A node that restarts with an old incarnation is corrected by the
  cluster (it jumps past the highest one anyone knows), but until then its facts are ignored.
- **`check_token` belongs in the write transaction.** Checking first and writing later is a race.
- **Fire windows are at-most-once on their own.** `advance` moves the cursor before the caller
  acts. The store must insert the due jobs into a durable queue in the same transaction as the
  cursor move; workers then run them with their own idempotency keys.
- **Lease time should be the store's clock** (database time). Node clocks that disagree cost
  availability but cannot cause a double fire, because firing is fenced by term and cursor.
- **On `Event::Renewed` or a restart, call `Fencer::reset` and `ShardScheduler::reset`.**
  Everything held by the old identity may have been taken over.
- **Drain before leaving**: `Fencer::begin_drain`, release everything in `held()`, tick the
  scheduler with a placement that excludes this node, then `Input::Leave`.
- **`has_majority` is a brake, not a lock.** Use it to stop a minority node from grabbing epochs
  back and forth during a partition; correctness comes from fencing either way.

## Known limits

- The handshake authenticates peers but does not encrypt or bind to the transport. Run cluster
  links over TLS (or a private network); without it an on-path attacker can relay a handshake and
  then tamper with traffic.
- One shared secret for the whole cluster: any node that holds it can announce facts about other
  nodes. Rotating it needs a rolling restart (dual-secret acceptance is not implemented).
- The secret is not wiped from memory on drop.
- Views larger than 1024 members are synchronised in random windows of 1024 per exchange, which
  converges more slowly. The intended scale is tens of nodes.
- `has_majority` counts dead members until `dead_retention_ms` passes, so after losing more than
  half of the nodes for good, the survivors report no majority until the tombstones expire.
- There is no adaptive failure detector (Lifeguard / phi-accrual). Timeouts are fixed
  configuration; on a slow network raise `probe_timeout_ms` and `suspect_timeout_ms`.

## Tests

| File | What | Count |
|---|---|---|
| `tests/protocol.rs` | Node ids, hash golden vectors, codec round trips, limits, fuzzing, handshake | 23 |
| `tests/ownership.rs` | Rng, placement, routing, store, fencing (incl. thread race and model test), leases and scheduler (incl. model test) | 52 |
| `tests/membership.rs` | Hand-driven scenarios for every branch of the state machine, merge table, three properties | 34 |
| `tests/simulation.rs` | Six whole-cluster simulations over many seeds, three tests of the harness itself | 9 |

Replay a failing simulation seed with `MT_SIM_SEED=<seed>`; run more seeds with `MT_SIM_SEEDS=<count>`.

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-cluster
cargo fmt -p mt-cluster --check
cargo clippy -p mt-cluster --all-targets -- -D warnings
cargo test -p mt-cluster
cargo build -p mt-cluster --target wasm32-unknown-unknown
cargo llvm-cov -p mt-cluster --summary-only
```
