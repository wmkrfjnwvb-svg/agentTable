# mt-refdata

Reference data and settings: the governed lists of permitted values that classify everything
else (regions, cost centres, return reasons, ISO countries), and the typed parameters that
configure a tenant. A "data dictionary" is simply the flat case of a reference set.

Pure and sans-IO: no clock, no storage, no randomness. It builds for `wasm32-unknown-unknown`,
so the browser offers and validates the same options as the server.

## Modules

| Module | Purpose |
|---|---|
| `model` | `RefSet` (code, names, flat or tree, scope system / tenant / base, custom attributes defined with the field framework, change policy), `RefItem` (stable id, immutable code, names per locale, typed value, parent, sort, enabled, validity period, attribute values, aliases per external system), `Overlay` (one tenant's changes to a system set) |
| `effective` | `EffectiveSet::build` merges items and overlay, verifies the whole set and indexes it: tree queries (`path`, `ancestors`, `descendants`, `is_within`, `depth`, `is_leaf`, `walk`), selection (`is_active`, `is_selectable`, `options`), labels and `path_label` (`Asia / Japan / Kanto / Tokyo`), `search`, crosswalks (`crosswalk` inbound, `alias` outbound). `RefData` is a tenant's catalogue; `RefData::resolver(day, parameters)` is the `OptionResolver` that `mt-core` validation asks |
| `change` | `Change` (add, update, move, enable / disable, set aliases, remove) and `State::apply`, which routes changes to the tenant overlay for system sets and verifies the result |
| `ledger` | `Ledger`: versioned history with optimistic concurrency, optional four-eyes approval (`propose` / `approve` / `reject` / `withdraw`), `as_of(version)`, `version_at(time)`, `restore` (replays and verifies stored history) |
| `impact` | `impact(set, change, usage)`: which items a change touches and how much they are used; removing an item that is still in use is blocked |
| `params` | `Registry`: typed parameters with defaults and overrides at tenant, base and user level; `resolve` + `explain` ("is 24: set for the tenant by … at …"); secrets as write-only handles; feature flags; history; choice parameters that feed field options |
| `builtin` | System sets: countries (ISO 3166-1), currencies (ISO 4217, with minor units), languages (ISO 639-1), sex (ISO/IEC 5218), fiscal period types. Ids derive from codes, so they are identical on every installation |

## Rules (each is asserted by tests)

1. **Cells store ids, never labels.** Relabelling, translating or re-sorting an item never
   touches a record. Codes are immutable: no change can alter them.
2. **A built set is always valid**: unique ids, unique codes (ASCII case ignored), parents exist,
   no cycles, depth within the set's limit, no parents in a flat set, one item per external
   value and system.
3. **Selection.** An item can be newly selected on a day when it *and every ancestor* is enabled
   and valid on that day, and it fits the field's scope (subtree, level, leaves only). Disabling
   an item never removes it from existing records.
4. **Tenant overlay.** Effective set = system items ∪ tenant additions − tenant-disabled, with
   tenant labels and aliases. Tenants cannot change a system item's code, parent, value or
   validity, cannot move or remove it, and cannot add a code or alias that collides with it.
5. **Changes are atomic.** A commit applies all of its changes or none; a failed operation leaves
   the ledger exactly as it was (model-tested with random operation sequences).
6. **History is reproducible.** Replaying the revisions on the starting state gives the current
   state; every past version reads back as it was; a history with a gap or a revision that does
   not apply does not load.
7. **Four eyes.** Under the approval policy nothing changes without a second person; the author
   can neither approve nor reject their own proposal. A proposal is applied to the state at
   approval time; if it no longer fits it stays pending instead of applying half-way.
8. **Parameters.** The most specific allowed override wins (user → base → tenant → default);
   every value is validated like a field; redefinition cannot invalidate existing overrides;
   secrets are never returned, printed or explained.

## Decisions worth knowing

- **Validity is inclusive** on both ends (`valid_to` is the last valid day).
- **Leaf** is structural (no children at all), so "leaves only" does not change when a child is
  disabled.
- **Crosswalk lookups are exact.** `"jp"` does not match the alias `"JP"`; integrations declare
  what they send.
- **Removal is the exception.** `Change::Remove` needs an item without children, and
  `impact` blocks it while records or other definitions still use the item. Disable instead.
- **Revision times are informational.** Versions order the history; `version_at` counts the
  leading revisions at or before an instant.

## Not in this crate

- Storage (the SQL closure table, caching per tenant) — `mt-store`.
- Who may use, view or manage a set, per subtree — `mt-perm`.
- Display names for countries, currencies and languages per locale, ISO 3166-2 subdivisions,
  IANA time zones and units of measure — delivered as data packs with the templates.
- JSON Schema validation of JSON parameters and the cron parameter type — with `mt-rules`.

## Tests

| File | What | Count |
|---|---|---|
| `tests/sets.rs` | Definitions, tree rules, selection by date and scope, labels, search, crosswalks, overlays, catalogue, resolver driving `mt-core` validation, built-in sets | 21 |
| `tests/ledger.rs` | Every change on tenant and system sets, refusals without side effects, version conflicts, proposals, history and restore, impact, **model-based test of the ledger** | 16 |
| `tests/params.rs` | Resolution and explanation, flags, secrets, refusals, redefinition, limits, choice parameters feeding field options | 9 |
| `tests/properties.rs` | Tree queries agree on random forests, selection consistency, storage round trip, order independence, overlay rules, atomicity; thread-safety and racing writers; size limits | 11 |

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-refdata
cargo fmt -p mt-refdata --check
cargo clippy -p mt-refdata --all-targets -- -D warnings
cargo test -p mt-refdata
cargo build -p mt-refdata --target wasm32-unknown-unknown
cargo llvm-cov -p mt-refdata --summary-only
```
