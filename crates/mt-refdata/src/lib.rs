//! Reference data and settings (product design doc 17 §5 C3–C4, doc 07 §2–3).
//!
//! * [`model`] — reference sets, items and tenant overlays as stored.
//! * [`effective`] — the validated, indexed set one tenant sees; tree queries, selection
//!   rules, labels, search and crosswalks; the option resolver for `mt-core` validation.
//! * [`change`] — changes to a set and how they route to the tenant overlay of system sets.
//! * [`ledger`] — versioned history with optional four-eyes approval and "as of" browsing.
//! * [`impact`] — what a change touches, before it is made.
//! * [`params`] — the typed, scoped parameter registry (system → tenant → base → user) with
//!   an explanation of where each value comes from.
//! * [`builtin`] — the built-in system sets.
//!
//! The crate is pure: no I/O, no clock, no randomness. It builds for
//! `wasm32-unknown-unknown`, so the browser validates and renders reference values with the
//! same code as the server.
#![forbid(unsafe_code)]

pub mod builtin;
pub mod change;
pub mod effective;
pub mod error;
pub mod impact;
pub mod ledger;
pub mod model;
pub mod params;

pub use change::{Change, ItemPatch, State};
pub use effective::{EffectiveSet, NoParameters, Origin, ParameterOptions, RefData, Resolver};
pub use error::RefDataError;
pub use impact::{impact, Impact, ItemImpact, NoUsage, Reference, ReferenceKind, Usage};
pub use ledger::{Ledger, Proposal, ProposalStatus, Revision};
pub use model::{Aliases, ChangePolicy, Overlay, RefItem, RefSet, Relabel, SetScope, Structure};
pub use params::{
    ParamContext, ParamDef, ParamKind, Registry, Resolved, Scope, ScopeLevel, SecretRef, Setting,
    Source, StoredChange, StoredRegistry,
};
