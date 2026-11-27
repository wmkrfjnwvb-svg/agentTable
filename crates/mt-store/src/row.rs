//! Reading columns the same way on every engine. MySQL sends long text with a BLOB wire
//! type, so text is read as a string or, failing that, as UTF-8 bytes.

use crate::error::{StoreError, StoreResult};
use sqlx::any::AnyRow;
use sqlx::Row;

/// A text column.
///
/// # Errors
/// [`StoreError::Corrupt`] when it is neither text nor UTF-8 bytes.
pub fn text(row: &AnyRow, column: &str) -> StoreResult<String> {
    opt_text(row, column)?.ok_or_else(|| StoreError::Corrupt(format!("`{column}` is empty")))
}

/// A nullable text column.
///
/// # Errors
/// [`StoreError::Corrupt`] when it is neither text nor UTF-8 bytes.
pub fn opt_text(row: &AnyRow, column: &str) -> StoreResult<Option<String>> {
    if let Ok(value) = row.try_get::<Option<String>, _>(column) {
        return Ok(value);
    }
    let bytes: Option<Vec<u8>> = row
        .try_get(column)
        .map_err(|e| StoreError::Corrupt(format!("`{column}`: {e}")))?;
    bytes
        .map(|b| {
            String::from_utf8(b)
                .map_err(|_| StoreError::Corrupt(format!("`{column}` is not UTF-8")))
        })
        .transpose()
}

/// An integer column.
///
/// # Errors
/// [`StoreError::Corrupt`].
pub fn int(row: &AnyRow, column: &str) -> StoreResult<i64> {
    opt_int(row, column)?.ok_or_else(|| StoreError::Corrupt(format!("`{column}` is empty")))
}

/// A nullable integer column.
///
/// # Errors
/// [`StoreError::Corrupt`].
pub fn opt_int(row: &AnyRow, column: &str) -> StoreResult<Option<i64>> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(|e| StoreError::Corrupt(format!("`{column}`: {e}")))
}
