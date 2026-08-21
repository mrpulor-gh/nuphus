//! Annotation — relationship annotation system
//!
//! Replaces old `memory::annotation` + `memory::annotation_store`,
//! As an independent product module, stored in the user data directory.
//!
//! Features:
//! - keyword -> {description, file paths, tags, group} relationship mapping
//! - Leader CRUD via tool calls
//! - Built-in preset annotations shipped with distribution
//! - Word boundary matching + priority sorted injection into Leader prompt

pub mod presets;
pub mod store;
pub mod types;
