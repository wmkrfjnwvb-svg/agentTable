//! The three SQL dialects. Every statement the store runs is built here from fixed text and
//! identifiers made only of validated ids (`t_tbl_…`, `f_fld_…`), never from names people
//! type; values always go through bind parameters.

use crate::error::{StoreError, StoreResult};

/// A database engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dialect {
    /// SQLite: a file per tenant.
    Sqlite,
    /// MySQL 8: a database per tenant.
    MySql,
    /// PostgreSQL: a database per tenant.
    Postgres,
}

/// Longest identifier the store makes (PostgreSQL allows 63 bytes).
pub const MAX_IDENT: usize = 63;

/// Checks an identifier: lowercase ASCII letters, digits and `_`, starting with a letter.
///
/// # Errors
/// [`StoreError::Invalid`].
pub fn check_ident(ident: &str) -> StoreResult<()> {
    let mut chars = ident.chars();
    let ok = ident.len() <= MAX_IDENT
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(StoreError::Invalid(format!(
            "`{ident}` is not a storage name"
        )))
    }
}

impl Dialect {
    /// The dialect of a database URL (`sqlite:`, `mysql:`, `postgres:` / `postgresql:`).
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for another scheme.
    pub fn of_url(url: &str) -> StoreResult<Dialect> {
        let scheme = url.split(':').next().unwrap_or_default();
        match scheme {
            "sqlite" => Ok(Dialect::Sqlite),
            "mysql" | "mariadb" => Ok(Dialect::MySql),
            "postgres" | "postgresql" => Ok(Dialect::Postgres),
            other => Err(StoreError::Invalid(format!(
                "`{other}` is not a supported database (sqlite, mysql, postgres)"
            ))),
        }
    }

    /// The engine's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Dialect::Sqlite => "SQLite",
            Dialect::MySql => "MySQL",
            Dialect::Postgres => "PostgreSQL",
        }
    }

    /// A quoted identifier.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for a name that is not a storage name.
    pub fn quote(self, ident: &str) -> StoreResult<String> {
        check_ident(ident)?;
        Ok(match self {
            Dialect::MySql => format!("`{ident}`"),
            Dialect::Sqlite | Dialect::Postgres => format!("\"{ident}\""),
        })
    }

    /// The `n`th bind placeholder (from 1).
    #[must_use]
    pub fn param(self, n: usize) -> String {
        match self {
            Dialect::Postgres => format!("${n}"),
            Dialect::Sqlite | Dialect::MySql => "?".to_owned(),
        }
    }

    /// `count` placeholders from `start`, comma-separated.
    #[must_use]
    pub fn params(self, start: usize, count: usize) -> String {
        (start..start.saturating_add(count))
            .map(|n| self.param(n))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The column type for long text (values, JSON).
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Dialect::MySql => "LONGTEXT",
            Dialect::Sqlite | Dialect::Postgres => "TEXT",
        }
    }

    /// The column type for keys and short names (indexable everywhere).
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Dialect::MySql => "VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin",
            Dialect::Sqlite | Dialect::Postgres => "VARCHAR(255)",
        }
    }

    /// The column type for bytes (sort keys).
    #[must_use]
    pub const fn blob(self) -> &'static str {
        match self {
            Dialect::MySql => "LONGBLOB",
            Dialect::Sqlite => "BLOB",
            Dialect::Postgres => "BYTEA",
        }
    }

    /// The column type for 64-bit integers.
    #[must_use]
    pub const fn int(self) -> &'static str {
        match self {
            Dialect::Sqlite => "INTEGER",
            Dialect::MySql | Dialect::Postgres => "BIGINT",
        }
    }

    /// Text after `CREATE TABLE … (…)`.
    #[must_use]
    pub const fn table_options(self) -> &'static str {
        match self {
            Dialect::MySql => " ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            Dialect::Sqlite | Dialect::Postgres => "",
        }
    }

    /// Locks the selected rows until the transaction ends, where the engine has row locks
    /// (SQLite has a single writer instead).
    #[must_use]
    pub const fn for_update(self) -> &'static str {
        match self {
            Dialect::Sqlite => "",
            Dialect::MySql | Dialect::Postgres => " FOR UPDATE",
        }
    }

    /// Like [`Dialect::for_update`], skipping rows another transaction holds (job claims).
    #[must_use]
    pub const fn for_update_skip_locked(self) -> &'static str {
        match self {
            Dialect::Sqlite => "",
            Dialect::MySql | Dialect::Postgres => " FOR UPDATE SKIP LOCKED",
        }
    }
}
