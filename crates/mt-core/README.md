# mt-core

Pure foundation types of the platform. No I/O, no clock, no randomness (time and random bytes are
inputs), so the same code runs natively and in WebAssembly and gives identical results.

## Modules

| Module | Purpose |
|---|---|
| `id` | Typed IDs `<prefix>_<26 chars>` (ULID-style: 48-bit time + 80-bit random, lowercase Crockford base32). One Rust type per object kind |
| `decimal`, `money`, `time` | Exact decimals (never floats), ISO 4217 amounts with minor-unit rules, `Date`, `Timestamp` (UTC ms), `DurationMs` |
| `value` | `Value`: one variant per data type plus `Null`; canonical serde form (decimals as strings) |
| `field` | `FieldDef` = data type × widget × options source × display format × validation × governance; `FieldDef::check` verifies the facets fit. Options come from an inline list, a reference set (subtree, level, leaves only, **cascade** from another field), a parameter, or a table. Localised names (`display_name(locale)`), optional AI skill binding, definition version |
| `validate` | `validate_value` returns every problem as data (`ValidationIssue`); `validate_cross_field`; `validate_cascade`; `check_writable`. `OptionResolver` is the hook to reference data and parameters |
| `coerce` | `parse_input`: locale-neutral reading of typed or imported text; never guesses (a lone decimal comma is rejected) |
| `convert` | `convertibility` matrix, `convert` (with a per-value `lossy` flag), `preview_conversion` for field type changes |
| `format` | Standard validators: email, URL, E.164, postal codes (US/JP/KR/DE/IN), IBAN, BIC, ABA, EIN, DE VAT ID, JP corporate and invoice numbers, KR business number, GSTIN, PAN, ISO codes, UUID |
| `pattern`, `numbering` | Regex-free anchored text patterns (linear time); auto-numbering patterns such as `PO-{YYYY}-{seq:5}` with `allocate` (one counter per period) |
| `limits` | The v1 product limits (rows, fields, views, group levels, filter conditions, editors, attachment size) |
| `sortkey` | `sort_key`: bytes whose bytewise order equals `compare_values`, so every database sorts identically |
| `provenance` | `CellMeta`: source (human, import, data source, automation, formula, AI), dual identity (`Actor`), `ReviewState` |
| `state` | `StateCategory`: the eight lifecycle state categories |

## Invariants (checked by tests)

- `sort_key(a) < sort_key(b)` exactly when `compare_values(a, b)` is `Less`, for any two values (property test, 20 000 cases per run when `PROPTEST_CASES` is raised).
- `parse_input(type, value.to_string())` returns the value for every type except free text.
- Conversions marked `Lossless` convert every value without loss and convert back to the original.
- Empty values (null, blank text, empty lists) are treated the same everywhere: they fail `required`, convert to null, and normalise to null.
- An AI change always names an initiator; only AI values can be suggestions; AI cannot review its own suggestion.
- Percent is stored as a fraction (25 % = `0.25`); typed and displayed as percent points.
- A numbering pattern must show the whole period its counter restarts in (`{MM}` needs a year, `{DD}` needs month and year), so two periods can never produce the same number; a back-dated record continues its own period's counter.
- A cascading field is re-checked whenever its parent changes, also for values already on the record; an AI field always has a data type and options like any other field.
- Definitions written by an older version (without `names`, `ai_skill`, `version`, `level`, `cascade_from`) still load.

## Known limits

- Text sort keys fold ASCII case only. Locale-aware collation is applied by the server on top.
- `format` validators check structure and published checksums. EIN, PAN and the German VAT ID are format-only checks.
- Uniqueness and reference-set membership need data outside this crate (`OptionResolver` is the hook).

## Checks

```bash
cargo test -p mt-core
```

```bash
cargo build -p mt-core --target wasm32-unknown-unknown
```
