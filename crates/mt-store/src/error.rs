//! Storage errors.

use thiserror::Error;

/// What can go wrong in storage.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error("database: {0}")]
    Db(String),
    /// A name, URL or value is not acceptable.
    #[error("invalid: {0}")]
    Invalid(String),
    /// There is no such thing.
    #[error("{0} does not exist")]
    NotFound(String),
    /// The thing exists already.
    #[error("{0} exists already")]
    Exists(String),
    /// The writer's fencing epoch is no longer current: another node owns the resource.
    #[error("fenced: the epoch is now {current}")]
    Fenced {
        /// The current epoch.
        current: u64,
    },
    /// The write does not follow what is stored (a sequence gap or a repeat).
    #[error("conflict: {0}")]
    Conflict(String),
    /// A job's lease was lost (expired and taken by another worker, or finished).
    #[error("the lease of job {0} is no longer held")]
    LeaseLost(String),
    /// The stored data cannot be read back.
    #[error("corrupt: {0}")]
    Corrupt(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        StoreError::Db(error.to_string())
    }
}

/// A storage result.
pub type StoreResult<T> = Result<T, StoreError>;
