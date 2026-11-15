# mt-formula

The formula language of computed fields (formula, lookup and roll-up fields): parser, printer,
type checker, bounded evaluator, and the dependency graph that decides what is recomputed when
something changes. Product design doc 17 domain D6; parity list doc 19 §1.2.

Pure and sans-IO. It builds for `wasm32-unknown-unknown`: the browser computes a cell with the
same code as the server, so a value never changes when the server's answer arrives. "Today",
"now", time zones, working days, parameters and reference labels are inputs (`Env`), never read
from the machine.

## Modules

| Module | Purpose |
|---|---|
| `parser` | `parse(text, schema)` → `Expr`. Field names are resolved to field ids, so renaming a field never breaks a formula. Errors carry a character position |
| `ast` | `Expr` (the stored form, JSON), the function table (`Function`, 81 functions in 6 groups), `Expr::to_text(names)` prints with the current field names and minimal parentheses |
| `types` | `check(expr, schema)` → `Type`, and `check_for_field(expr, schema, data_type)`: what is refused when a formula is saved |
| `value` | `Val`, the values during evaluation, and the rules for combining kinds |
| `eval` | `evaluate`, `evaluate_with` (sees results of computed fields evaluated earlier), `evaluate_to` (to the cell value of a field type). `Record` and `Env` are what the caller supplies |
| `deps` | `dependencies`, `analyse` (design-time cost warnings), `Graph`: cycle detection, evaluation order, `affected` (what to recompute after a change, also across links), `compute` (one record, in order) |
| `schema` | `Schema`: field ids, names and types of the formula's table and of linked tables |
| `error` | `FormulaError` (the text is not acceptable) and `ErrorValue` (the result for one record: `#DIV/0!`, `#VALUE!`, `#REF!`, `#NUM!`, `#CURRENCY!`, `#N/A`, `#LIMIT!`, `#CYCLE!`) |

## The language

```text
{Price} * {Qty} * (1 - {Discount})          fields in braces; + - * / & = <> < <= > >= and postfix %
SUM({Lines}.{Amount})                        {Link}.{Field}: the values of linked records (a list)
IF({Qty} = 0, "n/a", {Total} / {Qty})        text in double quotes ("" for a quote), TRUE / FALSE
COUNTIF({Lines}.{Qty}, CurrentValue > 1)     CurrentValue: the item, inside a list condition
SUMIF({Lines}.{Qty}, ">1", {Lines}.{Amount}) or a condition written as text
```

| Group | Functions |
|---|---|
| Logic | `IF IFS SWITCH AND OR NOT XOR IFERROR IFBLANK ISBLANK ISERROR ISNUMBER ISTEXT BLANK` |
| Numbers | `SUM AVERAGE MIN MAX MEDIAN COUNT COUNTA ABS SIGN ROUND ROUNDUP ROUNDDOWN INT MOD POWER CEILING FLOOR` |
| Text | `CONCAT` (`CONCATENATE`) `TEXTJOIN LEN LEFT RIGHT MID UPPER LOWER TRIM SUBSTITUTE REPLACE FIND SEARCH REPT VALUE TEXT EXACT CONTAINS` |
| Dates | `TODAY NOW DATE YEAR MONTH DAY HOUR MINUTE WEEKDAY ISOWEEKNUM DATEDIF DATEADD EDATE EOMONTH DAYS NETWORKDAYS WORKDAY DURATION TO_HOURS TO_DAYS` |
| Lists | `COUNTIF SUMIF AVERAGEIF FILTER UNIQUE FIRST LAST INDEX` |
| Reference data | `LABEL CODE ATTR PARAM` |

A lookup field is `{Link}.{Field}`; a roll-up is an aggregate over it (`SUM`, `COUNTIF`, `MAX`,
`TEXTJOIN`, …). Both are ordinary formulas.

## Semantics (each is asserted by tests)

1. **Exact numbers.** Decimals, never floats: `0.1 + 0.2 = 0.3`, `ROUND(1.005, 2) = 1.01`.
   Results that do not fit are `#NUM!`, not wrong numbers.
2. **Money keeps its currency.** Amount × number is an amount, amount ÷ amount is a number,
   amount + number is refused, two currencies never mix (`#CURRENCY!`). Amounts are rounded
   half-even to the currency's minor units once, when the result is stored.
3. **Dates.** Date + days = date; date − date = days; instant − instant = duration; instant ±
   duration = instant. Day boundaries, weekdays of instants and `DATEADD` on instants follow the
   viewer's time zone through `Env::start_of_day`; working days come from `Env::is_workday`.
4. **Empty values.** Empty counts as `0`, `""` and `FALSE` in arithmetic, joining and logic, as
   in spreadsheets, with two deliberate differences:
   * an empty date, time, amount, duration or reference **never satisfies an ordering**
     (`{Due} < TODAY()` is false without a due date: "no date" is not "overdue");
   * **calculating with an empty date gives an empty result**: `{Due} - {Start}` and
     `{Start} + 14` are empty while there is no start, and date functions return nothing for a
     missing date. The evaluator asks the type checker whether an empty operand is a date,
     which is why evaluation takes the `Schema`.
5. **Kinds do not mix silently.** Comparing different kinds: `=` is false, an ordering is
   `#VALUE!`. Text compares without regard to case (`mt_core::condition::fold_text`).
6. **Errors are values.** They flow through operators and functions, the first one wins, and
   `IFERROR` / `ISERROR` catch them. A reference to a deleted field is `#REF!`.
7. **Only what is needed is evaluated.** `IF`, `IFS`, `SWITCH`, `AND`, `OR`, `IFERROR`,
   `IFBLANK` are lazy, so `IF({n} = 0, 0, {total} / {n})` never divides by zero.
8. **Evaluation is bounded.** At most 200 000 steps, 10 000 list items and 100 000 characters
   per cell; beyond that the cell shows `#LIMIT!`. Formulas are at most 10 000 characters, 2 000
   parts and 64 levels deep. Evaluation never panics and never loops, for any tree.
9. **The checker tells the truth.** A formula it accepts evaluates to the kind it announced, to
   nothing, or to an error value (property-tested). Values of unknown kind (`PARAM`, `ATTR`)
   fit anywhere. Alternatives of different kinds are fine inside a formula; for a computed
   field every possible result must fit the field (`IF(x, "n/a", 1)` is refused for a number
   field, accepted for a text field).
10. **Printing keeps the meaning.** `parse(print(tree))` evaluates like `tree`, and a parsed
    formula prints to text that reads back as the same tree (property-tested).
11. **No circles.** `Graph::build` refuses computed fields that depend on each other, also
    across tables through links, and names the circle. The evaluation order does not depend on
    the order of the definitions.
12. **Recompute exactly what changed.** `Graph::affected` returns the computed fields of the
    same record that read a changed field (transitively, in evaluation order) and, per linking
    table and link field, the fields of the records that link to it. Recomputing only those
    gives the same record as recomputing everything (property-tested).

## What the caller must do

* **Supply `Env`**: today and now in the viewer's or the base's time zone, the business
  calendar, parameter values and reference labels. The crate never reads a clock.
* **Supply `Record::linked_values`**: the values of a field in the linked records, at most
  10 000 (more gives `#LIMIT!`).
* **Follow `Affected::remote`**: after recomputing the listed fields of the linking records,
  call `affected` for that table with those fields to follow longer chains. Circles cannot
  occur (`build` refuses them), so this ends.
* **Recompute volatile formulas** (`Dependencies::volatile`: `TODAY`, `NOW`) when they are read
  and once a day; they do not change when a record changes.
* **Re-check formulas when a referenced field changes its type or is deleted**
  (`check_for_field`); until then the cell shows `#REF!` or `#VALUE!`.
* A parameter read through a computed key (`Dependencies::any_param`) depends on every
  parameter.

## Not in this crate

* Storing formulas in field definitions (the computed facet of `mt_core::field::FieldDef`) and
  scheduling recomputation: storage and operations layers (wave 3).
* Localised function names and number formats: the formula text is the same in every language;
  `TEXT` uses English month and weekday names.
* Regular expressions and user-defined functions.

## Tests

| File | What | Count |
|---|---|---|
| `tests/parser.rs` | Literals, references, precedence, signs and percent, every function by name, printing with current names, storage, syntax errors with positions, unknown names, argument counts, size limits | 9 |
| `tests/evaluate.rs` | Arithmetic on every kind, money, dates and durations, empty dates, joining, comparisons, empty values, logic and laziness, errors as values, results of earlier computed fields, conversion to cell values, hand-built trees, the evaluation bounds | 20 |
| `tests/functions.rs` | Every function: standard, edge and invalid arguments; `TEXT` patterns; working days; time zones; lists of linked values; conditions written as text; nested list conditions; reference data and parameters | 22 |
| `tests/types.rs` | The type of every kind of expression, unknown kinds, everything the checker refuses and its message, trees that did not come from the parser, results against field types, error values | 13 |
| `tests/graph.rs` | Dependencies, warnings, order, circles (also across links), invalid definitions, what a change affects, two links to one table, computing a record | 9 |
| `tests/properties.rs` | Printing keeps the meaning; evaluation is total and repeatable; the checker tells the truth (random trees, and 1 500 formulas well typed by construction); storage round trip; the parser survives any text; order and recompute against a model; thread-safety | 9 |

By category: standard 41, edge 10, invalid 19, anomalous 3, properties 8, concurrency 1;
cross-platform is the WebAssembly build. Line coverage 99.8 % (the one uncovered line of the library is a
defensive arm that exhaustive matching requires and no input reaches).

```bash
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-formula
cargo fmt -p mt-formula --check
cargo clippy -p mt-formula --all-targets -- -D warnings
cargo test -p mt-formula
cargo build -p mt-formula --target wasm32-unknown-unknown
cargo llvm-cov -p mt-formula --summary-only
```
