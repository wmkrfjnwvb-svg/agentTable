//! Queries and views (product design doc 17 §5 C1, domain D5).
//!
//! * [`query`] — what a view asks for: filter, quick search, grouping, ordering, totals.
//! * [`eval`] — the in-memory evaluator, which defines the semantics.
//! * [`aggregate`] — footer totals, group subtotals and the selection summary.
//! * [`view`] — view definitions: kind (grid, kanban, calendar, timeline, gallery, list),
//!   field layout, colour rules and who may change them.
//!
//! The filter language itself is `mt_core::condition`. The SQL compiler lives with the
//! storage layer and is tested against [`eval::run`] on every supported database.
//!
//! The crate is pure: no I/O, no clock, no randomness. It builds for
//! `wasm32-unknown-unknown`, so the browser sorts, groups and totals the records it holds
//! with exactly the code the server uses.
#![forbid(unsafe_code)]

pub mod aggregate;
pub mod error;
pub mod eval;
pub mod query;
pub mod view;

pub use aggregate::{
    aggregate, summarize, Aggregate, AggregateFn, AggregateProblem, AggregateValue, NumericSummary,
    SelectionSummary,
};
pub use error::QueryError;
pub use eval::{compare, day_of, matches, run, Env, Group, QueryResult, Row};
pub use query::{DateBucket, Direction, Empties, GroupKey, Query, Search, SortKey};
pub use view::{ColourRule, ColourTarget, FieldSetting, RowHeight, View, ViewAccess, ViewKind};
