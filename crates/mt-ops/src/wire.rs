//! Values on the wire. In JSON a value keeps its normal tagged form; in the binary codec it
//! travels as its canonical JSON text, because a value may hold arbitrary JSON, which a
//! non-self-describing format cannot decode.

use mt_core::id::FieldId;
use mt_core::value::Value;
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

struct Wire<'a>(&'a Value);

impl Serialize for Wire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            self.0.serialize(serializer)
        } else {
            let text = serde_json::to_string(self.0).map_err(S::Error::custom)?;
            serializer.serialize_str(&text)
        }
    }
}

struct Owned(Value);

impl<'de> Deserialize<'de> for Owned {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            Value::deserialize(deserializer).map(Owned)
        } else {
            let text = String::deserialize(deserializer)?;
            serde_json::from_str(&text)
                .map(Owned)
                .map_err(D::Error::custom)
        }
    }
}

/// One value.
pub mod value {
    use super::{Owned, Wire};
    use mt_core::value::Value;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serializes a value.
    ///
    /// # Errors
    /// The serializer's.
    pub fn serialize<S: Serializer>(value: &Value, serializer: S) -> Result<S::Ok, S::Error> {
        Wire(value).serialize(serializer)
    }

    /// Deserializes a value.
    ///
    /// # Errors
    /// The deserializer's.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
        Owned::deserialize(deserializer).map(|owned| owned.0)
    }
}

/// An optional value.
pub mod option {
    use super::{Owned, Wire};
    use mt_core::value::Value;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serializes an optional value.
    ///
    /// # Errors
    /// The serializer's.
    pub fn serialize<S: Serializer>(
        value: &Option<Value>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_ref().map(Wire).serialize(serializer)
    }

    /// Deserializes an optional value.
    ///
    /// # Errors
    /// The deserializer's.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Value>, D::Error> {
        Option::<Owned>::deserialize(deserializer).map(|owned| owned.map(|o| o.0))
    }
}

/// Cells by field.
pub mod cells {
    use super::{BTreeMap, FieldId, Owned, Wire};
    use mt_core::value::Value;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serializes cells.
    ///
    /// # Errors
    /// The serializer's.
    pub fn serialize<S: Serializer>(
        cells: &BTreeMap<FieldId, Value>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(cells.iter().map(|(field, value)| (field, Wire(value))))
    }

    /// Deserializes cells.
    ///
    /// # Errors
    /// The deserializer's.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<FieldId, Value>, D::Error> {
        BTreeMap::<FieldId, Owned>::deserialize(deserializer)
            .map(|map| map.into_iter().map(|(k, v)| (k, v.0)).collect())
    }
}
