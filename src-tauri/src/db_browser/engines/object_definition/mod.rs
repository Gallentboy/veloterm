//! object_definition 的引擎分发（各引擎实现为自由函数，由 engines/mod.rs 分派）。

pub(crate) mod dameng;
pub(crate) mod mysql;
pub(crate) mod odbc;
pub(crate) mod oracle;
pub(crate) mod postgres;
