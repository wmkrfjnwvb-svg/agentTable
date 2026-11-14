//! Pure foundation types of the platform.
//!
//! `mt-core` has no I/O, no clock and no randomness: time and random bytes are inputs. It
//! compiles natively (server, future desktop client) and to WebAssembly (browser), so the same
//! rules run everywhere.
//!
//! | Module | Contents |
//! |---|---|
//! | [`id`] | typed, prefixed, time-ordered IDs |
//! | [`decimal`], [`money`], [`time`] | exact numbers, currency amounts, dates, instants, durations |
//! | [`value`] | the cell value model and its canonical serde form |
//! | [`field`] | field definitions: type, widget, options, format, validation, governance |
//! | [`validate`] | checking a value against a field definition |
//! | [`coerce`] | reading user input |
//! | [`convert`] | converting values between data types |
//! | [`format`] | standard format validators (email, IBAN, tax IDs, postal codes…) |
//! | [`pattern`], [`numbering`] | regex-free text patterns and auto-numbering patterns |
//! | [`sortkey`] | order-preserving sort keys |
//! | [`provenance`] | where a value came from: source, dual identity, review state |
//! | [`state`] | lifecycle state categories |
//! | [`codes`] | embedded ISO code lists |
#![forbid(unsafe_code)]

pub mod codes;
pub mod coerce;
pub mod convert;
pub mod decimal;
pub mod error;
pub mod field;
pub mod format;
pub mod id;
pub mod limits;
pub mod money;
pub mod numbering;
pub mod pattern;
pub mod provenance;
pub mod sortkey;
pub mod state;
pub mod time;
pub mod validate;
pub mod value;

pub use error::CoreError;
