# mt-query

What a view shows: which records, in which order, in which groups, with which totals — and the
in-memory evaluator that defines exactly what that means.

Pure and sans-IO. It builds for `wasm32-unknown-unknown`: the browser filters, sorts, groups and
totals the records it holds with the same code the server uses, so a view never "jumps" when the
server's answer arrives.

## Modules

| Module | Purpose |
|---|---|
| `query` | `Query` = filter (`mt_core::condition::Condition`) + quick `Search` + `GroupKey`s (up to 5 levels, optional day / week / month / quarter / year buckets for dates) + `SortKey`s (direction, where empties go) + `Aggregate`s. `Query::check` verifies it against the table's field types |
| `eval` | `run(query, rows, ctx, env)` → `QueryResult { rows, totals, groups }`; `matches` and `compare` for placing one changed record into a loaded result; `day_of` (the viewer's civil day of an instant). `Row` is one record; `Env` supplies field types, order substitutes (a reference item's position, a person's name) and display texts for search |
| `aggregate` | 13 functions: count empty / filled / unique, percent empty / filled, count / percent checked, sum, average, median, min, max, range. `summarize` for the selection status bar |
| `view` | `View`: kind (grid, list, kanban, calendar, timeline, gallery) with the fields each kind needs, column layout, frozen columns, row height, colour rules, access mode (shared, personal, protected, locked) |

## Semantics (each is asserted by tests)

1. **Order is total.** Records are ordered by group keys, then sort keys, then record id. The
   same query on the same data gives the same sequence everywhere, whatever order the records
   were loaded in. Paging and incremental updates rely on this.
2. **Empties.** By default empty counts as the smallest value (first ascending, last
   descending); a sort key can force empties first or last. Blank text and empty lists are empty.
3. **Groups partition the result.** Every record is in exactly one group per level; a list value
   (several tags, several people) is one key as a whole, whatever the order of its items. So the
   leaves of the group tree, in order, are the result's rows, and counts and sums of sub-groups
   add up to their parent's (property-tested).
4. **Groups never interleave**, even when two different groups have the same display position.
5. **Search** needs every word to occur in at least one of the listed fields, case-insensitively;
   references and people are searched by their labels, never by raw ids; a blank search matches
   everything.
6. **Aggregates are exact** and refuse to mislead: empty is not zero; money in more than one
   currency has no total (`MixedCurrencies`); overflow is reported; averages of money round
   half-even to the currency's minor units; the sum of no numbers is 0, the sum of no money is
   nothing (there is no currency to state it in).
7. **Time zones live in the `Context`.** Bucketing a date-time by day, week or month uses the
   viewer's day boundaries; the same instant can fall into April for one viewer and May for
   another, and both are right.
8. **Views never store data.** A view is a query, a layout and an access mode.

## Not in this crate

- **The SQL compiler.** It lives with the storage layer (wave 3) behind the same `Query`, and is
  tested for equality with `run` on SQLite, MySQL and PostgreSQL. Until then `run` is the only
  evaluator; it is meant for the records a client holds and for tests, not for a million rows.
- Capabilities ("may edit views", "view manager") are decided by `mt-perm`; `View::can_save` only
  combines them with the view's access mode.
- Locale-aware collation: text is ordered as the database sort keys order it (ASCII case folded).

## Tests

| File | What | Count |
|---|---|---|
| `tests/query.rs` | Filter, sort directions and empties, multi-key sort, reference order through `Env`, search, one and two levels of groups, every bucket size, date-time buckets across time zones, list keys, non-interleaving groups, paging, checks, storage round trip | 20 |
| `tests/aggregates.rs` | Every function on every applicable type, money rounding per currency, mixed currencies, overflow, dates, the function / type matrix, selection summary | 10 |
| `tests/views.rs` | Every view kind, layout and colour rules, refusals, who sees and who may change a view | 4 |
| `tests/properties.rs` | Result = sorted selection independent of input order; `compare` is a total order; groups partition the result and subtotals add up; pages tile the result; aggregates are mutually consistent; storage round trip; thread-safety; 50 000 rows | 9 |

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-query
cargo fmt -p mt-query --check
cargo clippy -p mt-query --all-targets -- -D warnings
cargo test -p mt-query
cargo build -p mt-query --target wasm32-unknown-unknown
cargo llvm-cov -p mt-query --summary-only
```
