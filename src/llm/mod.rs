//! LLM 模块 - 大模型接口
//!
//! MiniMax API Client, using Transport layer for HTTP/SSE.
//! All LLM interaction goes through ApiClient trait, not the old LLM trait.

pub mod client;
pub mod factory;

pub use client::LlmClient;
pub use factory::ClientFactory;
