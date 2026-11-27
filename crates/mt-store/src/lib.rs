//! `mt-store`: storage on SQLite, MySQL and PostgreSQL.
//!
//! * [`Store`]: the control database (the tenants of an installation) and one database per
//!   tenant — a file on SQLite, a database on MySQL and PostgreSQL — migrated on open.
//! * Each tenant database holds documents (configuration), the operation journal, the
//!   physical tables of the user's tables (`t_<table>` with `f_`, `sk_`, `cv_` columns per
//!   field), ownership epochs (fencing), scheduler leases and a job queue with leases.
//! * The same code serves the three engines through one dialect layer; every statement is
//!   tested on all three.
#![forbid(unsafe_code)]

pub mod dialect;
pub mod documents;
pub mod error;
pub mod fencing;
pub mod jobs;
pub mod records;
pub mod row;
pub mod schema;
pub mod store;

pub use dialect::Dialect;
pub use documents::Document;
pub use error::{StoreError, StoreResult};
pub use jobs::{Job, JobState, Lease, NewJob};
pub use records::Records;
pub use store::{int, uint, SharedStore, Store, StoreConfig, Tenant, TenantInfo};
