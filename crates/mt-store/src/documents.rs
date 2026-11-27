//! Configuration documents: JSON bodies by kind and id, with an optimistic version so two
//! writers cannot silently overwrite each other.

use crate::dialect::check_ident;
use crate::error::{StoreError, StoreResult};
use crate::store::{int, uint, Tenant};

/// A stored document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Its kind (`workspace`, `view`, …).
    pub kind: String,
    /// Its id within the kind.
    pub id: String,
    /// Raised by one on every write (1 after the first).
    pub version: u64,
    /// The body (JSON).
    pub body: String,
    /// When it was last written (ms).
    pub updated_at: i64,
}

fn check_id(id: &str) -> StoreResult<()> {
    if id.is_empty() || id.len() > 255 || !id.chars().all(|c| c.is_ascii_graphic()) {
        return Err(StoreError::Invalid(format!("`{id}` is not a document id")));
    }
    Ok(())
}

fn document_of(row: &sqlx::any::AnyRow) -> StoreResult<Document> {
    Ok(Document {
        kind: crate::row::text(row, "kind")?,
        id: crate::row::text(row, "id")?,
        version: uint(crate::row::int(row, "version")?)?,
        body: crate::row::text(row, "body")?,
        updated_at: crate::row::int(row, "updated_at")?,
    })
}

impl Tenant {
    /// A document.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for a bad kind or id; the database's errors.
    pub async fn document(&self, kind: &str, id: &str) -> StoreResult<Option<Document>> {
        check_ident(kind)?;
        check_id(id)?;
        let d = self.dialect;
        let row = sqlx::query(&format!(
            "SELECT kind, id, version, body, updated_at FROM _documents WHERE kind = {} AND id = {}",
            d.param(1),
            d.param(2)
        ))
        .bind(kind.to_owned())
        .bind(id.to_owned())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(document_of).transpose()
    }

    /// Every document of a kind, by id.
    ///
    /// # Errors
    /// As [`Tenant::document`].
    pub async fn documents(&self, kind: &str) -> StoreResult<Vec<Document>> {
        check_ident(kind)?;
        let rows = sqlx::query(&format!(
            "SELECT kind, id, version, body, updated_at FROM _documents WHERE kind = {} ORDER BY id",
            self.dialect.param(1)
        ))
        .bind(kind.to_owned())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(document_of).collect()
    }

    /// Writes a document over the version the writer read (0 for a new one); returns the
    /// new version.
    ///
    /// # Errors
    /// [`StoreError::Conflict`] when the stored version is another (someone wrote first, or
    /// the document exists already); [`StoreError::Invalid`]; the database's errors.
    pub async fn put_document(
        &self,
        kind: &str,
        id: &str,
        body: &str,
        expected: u64,
        now_ms: i64,
    ) -> StoreResult<u64> {
        check_ident(kind)?;
        check_id(id)?;
        let d = self.dialect;
        let next = expected
            .checked_add(1)
            .ok_or_else(|| StoreError::Invalid("version overflow".to_owned()))?;
        if expected == 0 {
            let inserted = sqlx::query(&format!(
                "INSERT INTO _documents (kind, id, version, body, updated_at) VALUES ({})",
                d.params(1, 5)
            ))
            .bind(kind.to_owned())
            .bind(id.to_owned())
            .bind(1_i64)
            .bind(body.to_owned())
            .bind(now_ms)
            .execute(&self.pool)
            .await;
            return match inserted {
                Ok(_) => Ok(1),
                Err(error) => match self.document(kind, id).await? {
                    Some(existing) => Err(StoreError::Conflict(format!(
                        "{kind} {id} exists at version {}",
                        existing.version
                    ))),
                    None => Err(error.into()),
                },
            };
        }
        let updated = sqlx::query(&format!(
            "UPDATE _documents SET version = {}, body = {}, updated_at = {} WHERE kind = {} AND id = {} AND version = {}",
            d.param(1),
            d.param(2),
            d.param(3),
            d.param(4),
            d.param(5),
            d.param(6)
        ))
        .bind(int(next)?)
        .bind(body.to_owned())
        .bind(now_ms)
        .bind(kind.to_owned())
        .bind(id.to_owned())
        .bind(int(expected)?)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            return Ok(next);
        }
        let current = self.document(kind, id).await?.map_or(0, |doc| doc.version);
        Err(StoreError::Conflict(format!(
            "{kind} {id} is at version {current}, not {expected}"
        )))
    }

    /// Deletes a document at the version the writer read.
    ///
    /// # Errors
    /// [`StoreError::Conflict`] for another version or a missing document.
    pub async fn delete_document(&self, kind: &str, id: &str, expected: u64) -> StoreResult<()> {
        check_ident(kind)?;
        check_id(id)?;
        let d = self.dialect;
        let deleted = sqlx::query(&format!(
            "DELETE FROM _documents WHERE kind = {} AND id = {} AND version = {}",
            d.param(1),
            d.param(2),
            d.param(3)
        ))
        .bind(kind.to_owned())
        .bind(id.to_owned())
        .bind(int(expected)?)
        .execute(&self.pool)
        .await?;
        if deleted.rows_affected() == 1 {
            Ok(())
        } else {
            Err(StoreError::Conflict(format!(
                "{kind} {id} is not at version {expected}"
            )))
        }
    }
}
