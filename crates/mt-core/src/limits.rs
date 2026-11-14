//! Product limits (v1 targets, product design doc 17 §5 C1). They live in one place so that the
//! server, the browser and the API documentation always agree on them.

/// Active rows per table. Older rows move to the archive tier.
pub const MAX_ACTIVE_ROWS_PER_TABLE: u64 = 1_000_000;
/// Fields per table.
pub const MAX_FIELDS_PER_TABLE: usize = 400;
/// Views per table.
pub const MAX_VIEWS_PER_TABLE: usize = 200;
/// Nested group levels in a view.
pub const MAX_GROUP_LEVELS: usize = 5;
/// Conditions in one filter (counting nested ones).
pub const MAX_FILTER_CONDITIONS: usize = 100;
/// People editing one base at the same time.
pub const MAX_CONCURRENT_EDITORS_PER_BASE: usize = 200;
/// Size of one attachment in bytes (2 GiB).
pub const MAX_ATTACHMENT_BYTES: u64 = 2_147_483_648;
