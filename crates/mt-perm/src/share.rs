//! Share links: access for whoever holds the link, without an account.
//!
//! A link is deliberately narrow. It opens one table (optionally one record or one view of
//! it) in one [`ShareMode`], for a limited time, and it never reveals classified or
//! capability-protected fields, never unmasks, and never administers. It is evaluated on
//! its own, not through grants: there is no principal behind it.

use crate::capability::{Action, Capability};
use crate::error::PermError;
use crate::grant::{FieldAccess, Inactive, Resource};
use crate::policy::{FieldMeta, FieldPermission, Reason, Visibility};
use mt_core::field::Classification;
use mt_core::id::{PrincipalId, RecordId, ShareLinkId, ViewId};
use mt_core::time::Timestamp;
use serde::{Deserialize, Serialize};

/// What the holder of a link may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareMode {
    /// See the records.
    View,
    /// See and comment.
    Comment,
    /// See, comment and change.
    Edit,
    /// Add a record through a form, without seeing any.
    Submit,
}

impl ShareMode {
    /// The actions the mode allows.
    #[must_use]
    pub const fn actions(self) -> &'static [Action] {
        match self {
            ShareMode::View => &[Action::RecordRead],
            ShareMode::Comment => &[Action::RecordRead, Action::RecordComment],
            ShareMode::Edit => &[
                Action::RecordRead,
                Action::RecordComment,
                Action::RecordUpdate,
            ],
            ShareMode::Submit => &[Action::RecordCreate],
        }
    }
}

/// A link that gives its holder narrow access to a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareLink {
    /// Stable ID.
    pub id: ShareLinkId,
    /// The table the link opens.
    pub on: Resource,
    /// The one record it opens, if it is a link to a record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RecordId>,
    /// The view whose filter and layout apply (the caller applies the view's filter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewId>,
    /// What the holder may do.
    pub mode: ShareMode,
    /// Which fields the holder gets.
    #[serde(default)]
    pub fields: FieldAccess,
    /// Who created the link.
    pub created_by: PrincipalId,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it stops working; never, if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// When it was revoked, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<Timestamp>,
}

impl ShareLink {
    /// Verifies the link: it opens a table, a submit link names no record, it includes at
    /// least one field, and it does not expire before it was created.
    ///
    /// # Errors
    /// [`PermError::Invalid`].
    pub fn check(&self) -> Result<(), PermError> {
        if self.on.table().is_none() {
            return Err(PermError::invalid("a share link opens one table"));
        }
        if self.mode == ShareMode::Submit && self.record.is_some() {
            return Err(PermError::invalid(
                "a link for submitting new records cannot name a record",
            ));
        }
        if self.fields.is_none() {
            return Err(PermError::invalid("the link includes no field at all"));
        }
        if self
            .expires_at
            .is_some_and(|expires| expires <= self.created_at)
        {
            return Err(PermError::invalid("the link expires before it starts"));
        }
        Ok(())
    }

    /// Whether the link works at `now`.
    ///
    /// # Errors
    /// Why it does not.
    pub fn status(&self, now: Timestamp) -> Result<(), Inactive> {
        if self.revoked_at.is_some_and(|at| at <= now) {
            return Err(Inactive::Revoked);
        }
        if now < self.created_at {
            return Err(Inactive::NotYetValid);
        }
        if self.expires_at.is_some_and(|expires| now >= expires) {
            return Err(Inactive::Expired);
        }
        Ok(())
    }

    /// Whether the holder of the link may do something to a record of a table (or, without
    /// a record, to the table: listing, or submitting a new record).
    ///
    /// # Errors
    /// Why not.
    pub fn allows(
        &self,
        capability: &Capability,
        on: &Resource,
        record: Option<RecordId>,
        now: Timestamp,
    ) -> Result<(), Reason> {
        self.status(now).map_err(|why| Reason::Inactive { why })?;
        if *on != self.on {
            return Err(Reason::Elsewhere);
        }
        let allowed = self
            .mode
            .actions()
            .iter()
            .any(|action| Capability::Builtin(*action) == *capability);
        if !allowed {
            return Err(Reason::NoGrant);
        }
        match (self.record, record) {
            // A link to one record opens that record only, and no list.
            (Some(linked), Some(asked)) if linked == asked => Ok(()),
            (Some(_), Some(_) | None) => Err(Reason::OutOfScope),
            (None, _) => Ok(()),
        }
    }

    /// What the holder may do with a field. Classified fields (anything above internal)
    /// and fields protected by a field-group capability are never shown through a link;
    /// masked fields stay masked; generated values are never editable.
    #[must_use]
    pub fn field_permission(&self, field: &FieldMeta) -> FieldPermission {
        let open = matches!(
            field.classification,
            Classification::Public | Classification::Internal
        );
        let shown = self.fields.allows(field.id)
            && open
            && field.view_capability.is_none()
            && self.mode != ShareMode::Submit;
        let visibility = if !shown {
            Visibility::Hidden
        } else if field.masked {
            Visibility::Masked
        } else {
            Visibility::Visible
        };
        // A form shows its fields for filling in although no record can be read.
        let fillable = self.fields.allows(field.id)
            && open
            && field.view_capability.is_none()
            && !field.masked;
        let editable = !field.generated
            && field.edit_capability.is_none()
            && match self.mode {
                ShareMode::View | ShareMode::Comment => false,
                ShareMode::Edit => visibility == Visibility::Visible,
                ShareMode::Submit => fillable,
            };
        FieldPermission {
            visibility,
            editable,
        }
    }
}
