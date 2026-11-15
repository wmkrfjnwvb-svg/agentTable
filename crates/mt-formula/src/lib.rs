//! The formula language (product design doc 17 domain D6, doc 19 §1.2).
//!
//! * [`parser`] — text to tree, with positions in error messages.
//! * [`ast`] — the tree, the function table, and printing with current field names.
//! * [`value`] — the values formulas calculate with and the rules for combining them.
//! * [`types`] — the type checker: definition-time errors and the type of the result.
//! * [`eval`] — the bounded evaluator.
//! * [`deps`] — what a formula reads, cycle detection, evaluation order and what to
//!   recompute after a change.
//! * [`schema`] — what a formula needs to know about the tables it refers to.
//! * [`error`] — problems with a formula's text, and the error values a formula can
//!   evaluate to.
//!
//! The crate is pure: no I/O, no clock, no randomness. "Today", working days, parameters
//! and reference labels come from the caller, so a formula gives the same result on the
//! server, in the browser (WebAssembly) and in a test.
#![forbid(unsafe_code)]

pub mod ast;
pub mod deps;
pub mod error;
pub mod eval;
mod functions;
pub mod parser;
pub mod schema;
pub mod types;
pub mod value;

pub use ast::{
    BinaryOp, DateFn, Expr, Function, ListFn, LogicFn, LookupFn, Names, NumberFn, TextFn, UnaryOp,
};
pub use deps::{
    analyse, dependencies, Affected, Analysis, ComputedField, Dependencies, Graph, Remote, Warning,
};
pub use error::{ErrorValue, FormulaError};
pub use eval::{evaluate, evaluate_to, evaluate_with, Computed, Env, Record};
pub use parser::parse;
pub use schema::Schema;
pub use types::{check, check_for_field, Type};
pub use value::{Comparison, Val};
