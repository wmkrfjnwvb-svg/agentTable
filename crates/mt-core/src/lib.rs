//! mt-core
#![forbid(unsafe_code)]
pub mod decimal;
pub mod error;
pub mod id;
pub mod time;
pub mod field { 
    //! tmp
    /// tmp
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DataType { /// a
        Bool, /// b
        Date }
    impl core::fmt::Display for DataType { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str(match self { DataType::Bool => "bool", DataType::Date => "date" }) } }
}
