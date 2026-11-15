//! The query of a view: which records, in which order, in which groups, with which totals.
//!
//! A [`Query`] is data. It is checked against the table's field types ([`Query::check`]) and
//! evaluated either in memory ([`crate::run`], which defines the semantics) or by the SQL
//! compiler of the storage layer, which must return the same answer.

use crate::aggregate::Aggregate;
use crate::error::QueryError;
use mt_core::condition::Condition;
use mt_core::field::DataType;
use mt_core::id::FieldId;
use mt_core::limits::MAX_GROUP_LEVELS;
use serde::{Deserialize, Serialize};

/// Maximum number of sort keys.
pub const MAX_SORT_KEYS: usize = 10;
/// Maximum number of aggregates in one query.
pub const MAX_AGGREGATES: usize = 200;
/// Maximum number of characters in a search text.
pub const MAX_SEARCH_CHARS: usize = 200;
/// Maximum number of words in a search text.
pub const MAX_SEARCH_WORDS: usize = 10;

/// Ascending or descending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Smallest first.
    #[default]
    Asc,
    /// Largest first.
    Desc,
}

/// Where records without a value go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Empties {
    /// Empty counts as the smallest value: first when ascending, last when descending.
    #[default]
    Natural,
    /// Always before records with a value.
    First,
    /// Always after records with a value.
    Last,
}

/// One level of ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    /// The field to order by.
    pub field: FieldId,
    /// The direction.
    #[serde(default)]
    pub direction: Direction,
    /// Where empty values go.
    #[serde(default)]
    pub empties: Empties,
}

impl SortKey {
    /// Ascending by `field`, empties first.
    #[must_use]
    pub const fn asc(field: FieldId) -> Self {
        SortKey {
            field,
            direction: Direction::Asc,
            empties: Empties::Natural,
        }
    }

    /// Descending by `field`, empties last.
    #[must_use]
    pub const fn desc(field: FieldId) -> Self {
        SortKey {
            field,
            direction: Direction::Desc,
            empties: Empties::Natural,
        }
    }
}

/// How date and date-time values are collected into groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateBucket {
    /// One group per day.
    Day,
    /// One group per week (starting on the context's first day of the week).
    Week,
    /// One group per calendar month.
    Month,
    /// One group per calendar quarter.
    Quarter,
    /// One group per calendar year.
    Year,
}

/// One level of grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupKey {
    /// The field to group by.
    pub field: FieldId,
    /// The order of the groups.
    #[serde(default)]
    pub direction: Direction,
    /// For date and date-time fields: the size of a group. Without it, each distinct value
    /// is a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<DateBucket>,
}

impl GroupKey {
    /// One group per distinct value of `field`, in ascending order.
    #[must_use]
    pub const fn by(field: FieldId) -> Self {
        GroupKey {
            field,
            direction: Direction::Asc,
            bucket: None,
        }
    }
}

/// Quick search: every word must occur in at least one of the fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Search {
    /// What the person typed.
    pub text: String,
    /// The fields to look in (normally the view's visible fields).
    pub fields: Vec<FieldId>,
}

/// What a view asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Query {
    /// Which records to include (all, if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Condition>,
    /// Quick search on top of the filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Search>,
    /// Grouping levels, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group: Vec<GroupKey>,
    /// Ordering within the innermost group, most significant first. Ties are broken by
    /// record id, so the order is always total.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<SortKey>,
    /// Totals to compute for the whole result and for every group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aggregates: Vec<Aggregate>,
}

impl Query {
    /// Checks the query against the field types of its table.
    ///
    /// # Errors
    /// [`QueryError::Invalid`], [`QueryError::Limit`], or the condition's own error.
    pub fn check(&self, type_of: &dyn Fn(FieldId) -> Option<DataType>) -> Result<(), QueryError> {
        let known = |what: &'static str, field: FieldId| {
            type_of(field).ok_or_else(|| {
                QueryError::invalid(what, format!("the field {field} is not in the table"))
            })
        };
        if let Some(filter) = &self.filter {
            filter.check(type_of)?;
        }
        if let Some(search) = &self.search {
            if search.text.chars().count() > MAX_SEARCH_CHARS {
                return Err(QueryError::Limit("search text too long"));
            }
            if search.text.split_whitespace().count() > MAX_SEARCH_WORDS {
                return Err(QueryError::Limit("too many search words"));
            }
            if search.fields.is_empty() {
                return Err(QueryError::invalid("search", "names no field to look in"));
            }
            for field in &search.fields {
                known("search", *field)?;
            }
        }
        if self.group.len() > MAX_GROUP_LEVELS {
            return Err(QueryError::Limit("too many group levels"));
        }
        for (index, key) in self.group.iter().enumerate() {
            let data_type = known("group", key.field)?;
            if self.group.iter().take(index).any(|k| k.field == key.field) {
                return Err(QueryError::invalid(
                    "group",
                    format!("the field {} is grouped twice", key.field),
                ));
            }
            if matches!(
                data_type,
                DataType::Json | DataType::LongText | DataType::Attachment
            ) {
                return Err(QueryError::invalid(
                    "group",
                    format!("a {data_type} field cannot be grouped"),
                ));
            }
            if key.bucket.is_some() && !matches!(data_type, DataType::Date | DataType::DateTime) {
                return Err(QueryError::invalid(
                    "group",
                    format!("a {data_type} field cannot be grouped by day, week, month, quarter or year"),
                ));
            }
        }
        if self.sort.len() > MAX_SORT_KEYS {
            return Err(QueryError::Limit("too many sort keys"));
        }
        for (index, key) in self.sort.iter().enumerate() {
            let data_type = known("sort", key.field)?;
            if self.sort.iter().take(index).any(|k| k.field == key.field) {
                return Err(QueryError::invalid(
                    "sort",
                    format!("the field {} is sorted twice", key.field),
                ));
            }
            if matches!(data_type, DataType::Json | DataType::Attachment) {
                return Err(QueryError::invalid(
                    "sort",
                    format!("a {data_type} field cannot be sorted"),
                ));
            }
        }
        if self.aggregates.len() > MAX_AGGREGATES {
            return Err(QueryError::Limit("too many aggregates"));
        }
        for aggregate in &self.aggregates {
            let data_type = known("aggregate", aggregate.field)?;
            if !aggregate.function.applies_to(data_type) {
                return Err(QueryError::invalid(
                    "aggregate",
                    format!(
                        "`{}` cannot be computed for a {data_type} field",
                        aggregate.function
                    ),
                ));
            }
        }
        Ok(())
    }
}
