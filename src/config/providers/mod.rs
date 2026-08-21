//! Provider implementations
//!
//! Each concrete Provider gets its own file. All 12 built-in Providers
//! (Chat-Completions-based + Anthropic) are implemented.

pub mod anthropic;
pub mod bytedance;
pub mod custom;
pub mod deepseek;
pub mod google;
pub mod kimi;
pub mod local;
pub mod minimax;
pub mod openai;
pub mod openrouter;
pub mod qwen;
pub mod zhipu;

pub use anthropic::AnthropicProvider;
pub use bytedance::ByteDanceProvider;
pub use custom::CustomProvider;
pub use deepseek::DeepSeekProvider;
pub use google::GoogleProvider;
pub use kimi::KimiProvider;
pub use local::LocalProvider;
pub use minimax::MiniMaxProvider;
pub use openai::OpenAIProvider;
pub use openrouter::OpenRouterProvider;
pub use qwen::QwenProvider;
pub use zhipu::ZhipuProvider;
