//! Call expression modules.

pub mod array_ops;
pub(crate) mod bool_methods;
mod cascade;
pub(crate) mod datetime_methods;
pub(crate) mod decimal_methods;
pub(crate) mod float_methods;
mod function;
mod hash_methods;
pub(crate) mod int_methods;
mod method;
pub mod method_registry;
pub(crate) mod null_methods;
mod pipeline;
mod query_builder_methods;
pub mod string_methods;
pub mod user_methods;
