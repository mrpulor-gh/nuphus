//! Config commands — split by responsibility.
//!
//! - `toml_ops`: low-level TOML CRUD (`get_config_path`, `update_config_toml`, ...)
//! - `llm`: LLM provider/model configuration commands + helpers
//! - `preferences`: user language & project dir preferences
//! - `capabilities`: capabilities, refinement threshold, tool permissions
//!
//! All public items are re-exported at this level so external callers can
//! continue using `crate::commands::config::<name>` (preserves the original
//! single-file API surface).

pub mod capabilities;
pub mod handoff;
pub mod llm;
pub mod preferences;
pub mod relation;
pub mod team;
pub mod toml_ops;

// Re-export so `crate::commands::config::configure_llm`, `get_config_path`,
// `model_supports_vision`, `load_llm_config_from_disk` etc. still resolve
// exactly like they did when everything lived in a single `config.rs`.
pub use self::capabilities::*;
pub use self::handoff::*;
pub use self::llm::*;
pub use self::preferences::*;
pub use self::relation::*;
pub use self::team::*;
pub use self::toml_ops::*;
