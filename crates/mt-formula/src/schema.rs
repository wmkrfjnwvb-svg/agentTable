//! What a formula needs to know about the tables it refers to.

use mt_core::field::DataType;
use mt_core::id::FieldId;

/// The fields a formula can refer to: those of its own table and, through link fields,
/// those of linked tables. Implemented by the schema catalogue.
pub trait Schema {
    /// The field of the formula's table with this name.
    fn field_by_name(&self, name: &str) -> Option<FieldId>;

    /// The data type of a field of the formula's table.
    fn field_type(&self, field: FieldId) -> Option<DataType>;

    /// The field with this name in the table the link field `link` points to.
    fn linked_field_by_name(&self, link: FieldId, name: &str) -> Option<FieldId>;

    /// The data type of a field of the table the link field `link` points to.
    fn linked_field_type(&self, link: FieldId, field: FieldId) -> Option<DataType>;
}
