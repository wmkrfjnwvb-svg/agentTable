# mt-perm

Access control (product design doc 17: domain D12, capability C5, process P9): who may do
what, where, to which records and fields — and why.

One engine answers every question about access, for the UI, the API, exports, search,
notifications, dashboards and AI alike. It is pure (no I/O, no clock): the same code enforces
on the server and gives hints in the browser (WebAssembly).

## Modules

| Module | Purpose |
|---|---|
| `capability` | `Action` — the built-in actions (record read / create / update / delete / comment / export / import, classified data, unmasking, approval, schema, views, automations, skills, agents, sharing, access, members, reference data, settings, templates, audit) — and `Capability::Custom` keys a base adds for its own buttons, field groups and operations (`expense.pay`). No job titles: capabilities name what can be done. |
| `grant` | `Grant`: a principal (person, guest, agent, API app or group) receives a capability or a `CapabilitySet` on a `Resource` (tenant, workspace, base, table), narrowed for record actions by a `Scope` (all, own, team, a condition, a reference subtree, listed records), a condition (an amount limit), a `FieldAccess` (only / except fields) and a validity period; it records who gave it, when and why. `SodRule`: two capabilities that one person may never hold (`Hold`) or never both use on one record (`SameRecord`). |
| `policy` | `Policy` holds grants, sets and rules. `decide` returns a `Decision`: allowed or not, the **grant chain** that allows it, and for every grant that came close the `Reason` it did not apply — that is *explain*; *preview as user* is asking for another actor. `row_filter` gives the records an actor may act on (lists never return what `decide` would refuse). `field_permissions` gives each field's visibility (visible, masked, hidden) and editability, from classification clearance, masks, field groups and generated fields. `violations` lists grants that break a rule after the fact. |
| `share` | `ShareLink`: anonymous, narrow, expiring access to one table (optionally one record or view), never to classified or protected fields, never unmasked, never administration. |
| `review` | `AccessReview`: a state machine over a snapshot of grants; nobody reviews their own access; revocations need a note; completing returns the grants to revoke. |

The caller answers what the engine cannot know: `Directory` (who is in which group, the kind
of each principal), `Facts` (which fields make a principal an owner, reference trees) and
`RecordFacts` (a record's values).

## Rules (each is asserted by tests)

1. **No deny rules.** Access is the sum of what was granted; that is what makes every
   decision explainable. What must never be combined is a separation-of-duties rule.
2. **Separation of duties.** A grant that would let anyone (directly or through a group)
   hold both capabilities of a `Hold` rule is refused; a `SameRecord` rule refuses the second
   capability on a record where the actor already used the first ("nobody approves their own
   request").
3. **Dual identity.** An agent acting for a person may do only what *both* may: grants are
   checked for each, rows and fields are intersected. An agent never exceeds the person who
   started it, nor its own grants.
4. **Guests never administer**, and share links never reveal classified fields or unmask.
5. **Limits are conditions on the record**: a grant "approve up to 10,000" does not apply to
   a request of 12,000, and the refusal says so.
6. **Revoked and expired grants stay on record** and never apply; validity is checked at
   the instant of the request, which is an input.
7. **Field visibility is monotone in clearance**: more clearance never hides a field; masks
   apply unless `data.unmask` is held.

## Tests

| File | What | Count |
|---|---|---|
| `tests/policy.rs` | Scopes, conditions and limits, field access, classification and masks, field-group capabilities, sets, validity, revocation, SoD (hold and same record), dual identity, row filters, explanations | 21 |
| `tests/model.rs` | The building blocks one by one: built-in and custom capabilities, places containing each other, field lists, grants and their validity, malformed grants, sets and rules | 9 |
| `tests/share_review.rs` | Share links (modes, expiry, narrowing); access reviews; **model-based test of the review state machine** against a reference (512 random sequences of verdicts, completions and cancellations; refused steps change nothing) | 7 |
| `tests/properties.rs` | Row filters agree with decisions for every record; agents never exceed the person; more grants never take access away; storage round trip; one policy answers many threads | 5 |

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-perm
cargo fmt -p mt-perm --check
cargo clippy -p mt-perm --all-targets -- -D warnings
cargo test -p mt-perm
cargo build -p mt-perm --target wasm32-unknown-unknown
cargo llvm-cov -p mt-perm --summary-only
```
