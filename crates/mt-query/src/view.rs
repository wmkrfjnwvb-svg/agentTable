//! View definitions: a saved way of looking at a table. A view never stores data; it stores
//! a [`Query`], a layout and who may change them.
//!
//! | Kind | Needs | Shows |
//! |---|---|---|
//! | grid | — | rows and columns |
//! | list | — | one line per record |
//! | kanban | a single-choice, checkbox or people field to stack by | one column per value |
//! | calendar | a start date (and optionally an end date) | records on days |
//! | timeline | a start and an end date | bars over time |
//! | gallery | optionally an attachment field for the cover | cards |
//!
//! # Who may change a view
//!
//! | Access | Seen by | Configuration changed by |
//! |---|---|---|
//! | shared | everyone with access to the table | anyone who may edit views |
//! | personal | its owner only | its owner |
//! | protected | everyone | its owner and view managers; others may adjust it for themselves without saving |
//! | locked | everyone | nobody until a view manager unlocks it |
//!
//! "May edit views" and "view manager" are capabilities decided by the access engine; this
//! module only combines them with the view's own access mode.

use crate::error::QueryError;
use crate::query::Query;
use mt_core::condition::Condition;
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId, TableId, ViewId};
use serde::{Deserialize, Serialize};

/// Maximum number of characters in a view name.
pub const MAX_VIEW_NAME_CHARS: usize = 255;
/// Maximum number of colour rules in one view.
pub const MAX_COLOUR_RULES: usize = 50;
/// Maximum number of characters in a colour token.
pub const MAX_COLOUR_CHARS: usize = 32;
/// Widest column, in pixels.
pub const MAX_COLUMN_WIDTH: u16 = 2_000;
/// Narrowest column, in pixels.
pub const MIN_COLUMN_WIDTH: u16 = 40;

/// The kind of a view and the fields that kind needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewKind {
    /// Rows and columns.
    Grid,
    /// One line per record.
    List,
    /// One column per value of a field.
    Kanban {
        /// The field whose values are the columns.
        stack_by: FieldId,
    },
    /// Records on the days of a calendar.
    Calendar {
        /// The day (or instant) a record starts.
        start: FieldId,
        /// The day (or instant) it ends, for records that span several days.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<FieldId>,
    },
    /// Bars over time.
    Timeline {
        /// Where a bar starts.
        start: FieldId,
        /// Where it ends.
        end: FieldId,
    },
    /// Cards.
    Gallery {
        /// The attachment field that provides the card's picture.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cover: Option<FieldId>,
    },
}

/// Who sees a view and who may change its configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewAccess {
    /// Seen by everyone; changed by anyone who may edit views.
    #[default]
    Shared,
    /// Seen and changed by its owner only.
    Personal,
    /// Seen by everyone; saved changes only by its owner and view managers.
    Protected,
    /// Seen by everyone; changed by nobody until unlocked.
    Locked,
}

/// The height of a grid row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowHeight {
    /// One line.
    #[default]
    Short,
    /// Two lines.
    Medium,
    /// Four lines.
    Tall,
    /// Six lines.
    ExtraTall,
}

/// How one field appears in a view. The order of the settings is the order of the columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSetting {
    /// The field.
    pub field: FieldId,
    /// Whether it is shown.
    #[serde(default = "shown")]
    pub visible: bool,
    /// Column width in pixels (the default width, if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
}

const fn shown() -> bool {
    true
}

/// What a colour rule paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum ColourTarget {
    /// The whole row or card.
    Record,
    /// One cell.
    Field {
        /// The field.
        field: FieldId,
    },
}

/// "Paint it like this when the record matches." Rules are tried in order; the first match
/// per target wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColourRule {
    /// When the rule applies.
    pub when: Condition,
    /// What it paints.
    pub target: ColourTarget,
    /// A colour token of the theme (`"red"`, `"amber"`), not a raw colour, so that light and
    /// dark themes both stay readable.
    pub colour: String,
}

/// A saved view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// Stable id.
    pub id: ViewId,
    /// The table it looks at.
    pub table: TableId,
    /// Display name.
    pub name: String,
    /// The kind and its fields.
    pub kind: ViewKind,
    /// Which records, in which order and groups, with which totals.
    #[serde(default)]
    pub query: Query,
    /// The fields in column order with their visibility and width. Fields of the table that
    /// are not listed are hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldSetting>,
    /// How many of the leading visible columns stay in place when scrolling sideways.
    #[serde(default)]
    pub frozen: u16,
    /// Row height in a grid.
    #[serde(default)]
    pub row_height: RowHeight,
    /// Conditional colours.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colours: Vec<ColourRule>,
    /// Who sees it and who may change it.
    #[serde(default)]
    pub access: ViewAccess,
    /// Who created it (the owner of a personal or protected view).
    pub owner: PrincipalId,
}

impl View {
    /// A shared grid view of a table showing everything.
    pub fn grid(id: ViewId, table: TableId, name: impl Into<String>, owner: PrincipalId) -> Self {
        View {
            id,
            table,
            name: name.into(),
            kind: ViewKind::Grid,
            query: Query::default(),
            fields: Vec::new(),
            frozen: 0,
            row_height: RowHeight::Short,
            colours: Vec::new(),
            access: ViewAccess::Shared,
            owner,
        }
    }

    /// The visible fields in column order.
    pub fn visible_fields(&self) -> impl Iterator<Item = FieldId> + '_ {
        self.fields
            .iter()
            .filter(|setting| setting.visible)
            .map(|setting| setting.field)
    }

    /// Whether `principal` sees the view at all (given access to the table).
    #[must_use]
    pub fn is_listed_for(&self, principal: PrincipalId) -> bool {
        match self.access {
            ViewAccess::Personal => self.owner == principal,
            ViewAccess::Shared | ViewAccess::Protected | ViewAccess::Locked => true,
        }
    }

    /// Whether `principal` may save changes to the view's configuration. `may_edit_views`
    /// and `is_view_manager` are capabilities the access engine decided for the table.
    #[must_use]
    pub fn can_save(
        &self,
        principal: PrincipalId,
        may_edit_views: bool,
        is_view_manager: bool,
    ) -> bool {
        match self.access {
            ViewAccess::Shared => may_edit_views || is_view_manager,
            ViewAccess::Personal => self.owner == principal,
            ViewAccess::Protected => self.owner == principal || is_view_manager,
            ViewAccess::Locked => false,
        }
    }

    /// Whether `principal` may change the access mode (lock, unlock, protect, share).
    #[must_use]
    pub fn can_change_access(&self, principal: PrincipalId, is_view_manager: bool) -> bool {
        match self.access {
            ViewAccess::Personal | ViewAccess::Protected => {
                self.owner == principal || is_view_manager
            }
            ViewAccess::Shared | ViewAccess::Locked => is_view_manager,
        }
    }

    /// Checks the view against the field types of its table.
    ///
    /// # Errors
    /// [`QueryError::Invalid`], [`QueryError::Limit`], or an error of the query or of a
    /// colour rule's condition.
    pub fn check(&self, type_of: &dyn Fn(FieldId) -> Option<DataType>) -> Result<(), QueryError> {
        let invalid = |reason: String| QueryError::invalid("view", reason);
        if self.name.trim().is_empty()
            || self.name.chars().count() > MAX_VIEW_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid(format!(
                "the name must be 1 to {MAX_VIEW_NAME_CHARS} characters without control characters"
            )));
        }
        self.query.check(type_of)?;

        let typed = |role: &str, field: FieldId, allowed: &[DataType]| match type_of(field) {
            None => Err(invalid(format!(
                "the {role} field {field} is not in the table"
            ))),
            Some(data_type) if !allowed.contains(&data_type) => Err(invalid(format!(
                "a {data_type} field cannot be the {role} of this kind of view"
            ))),
            Some(_) => Ok(()),
        };
        let dates = [DataType::Date, DataType::DateTime];
        match self.kind {
            ViewKind::Grid | ViewKind::List | ViewKind::Gallery { cover: None } => {}
            ViewKind::Kanban { stack_by } => typed(
                "stacking field",
                stack_by,
                &[DataType::Reference, DataType::Bool, DataType::Principal],
            )?,
            ViewKind::Calendar { start, end } => {
                typed("start date", start, &dates)?;
                if let Some(end) = end {
                    typed("end date", end, &dates)?;
                    if end == start {
                        return Err(invalid("start and end are the same field".to_owned()));
                    }
                }
            }
            ViewKind::Timeline { start, end } => {
                typed("start date", start, &dates)?;
                typed("end date", end, &dates)?;
                if end == start {
                    return Err(invalid("start and end are the same field".to_owned()));
                }
            }
            ViewKind::Gallery { cover: Some(cover) } => {
                typed("cover", cover, &[DataType::Attachment])?;
            }
        }

        for (index, setting) in self.fields.iter().enumerate() {
            if type_of(setting.field).is_none() {
                return Err(invalid(format!(
                    "the field {} is not in the table",
                    setting.field
                )));
            }
            if self
                .fields
                .iter()
                .take(index)
                .any(|s| s.field == setting.field)
            {
                return Err(invalid(format!(
                    "the field {} is listed twice",
                    setting.field
                )));
            }
            if setting
                .width
                .is_some_and(|w| !(MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(&w))
            {
                return Err(invalid(format!(
                    "a column width must be between {MIN_COLUMN_WIDTH} and {MAX_COLUMN_WIDTH} pixels"
                )));
            }
        }
        if usize::from(self.frozen) > self.visible_fields().count() {
            return Err(invalid(
                "more columns are frozen than are visible".to_owned(),
            ));
        }

        if self.colours.len() > MAX_COLOUR_RULES {
            return Err(QueryError::Limit("too many colour rules"));
        }
        for rule in &self.colours {
            rule.when.check(type_of)?;
            let token_ok = !rule.colour.is_empty()
                && rule.colour.len() <= MAX_COLOUR_CHARS
                && rule
                    .colour
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !token_ok {
                return Err(invalid(
                    "a colour is a theme token such as `red` or `amber-2`".to_owned(),
                ));
            }
            if let ColourTarget::Field { field } = rule.target {
                if type_of(field).is_none() {
                    return Err(invalid(format!(
                        "the coloured field {field} is not in the table"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The colour of a target for one record: the first matching rule's colour.
    #[must_use]
    pub fn colour_of<'a>(
        &self,
        target: ColourTarget,
        value_of: &dyn Fn(FieldId) -> Option<&'a mt_core::value::Value>,
        ctx: &dyn mt_core::condition::Context,
    ) -> Option<&str> {
        self.colours
            .iter()
            .find(|rule| rule.target == target && rule.when.evaluate(value_of, ctx))
            .map(|rule| rule.colour.as_str())
    }
}
