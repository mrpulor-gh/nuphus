//! MCP (Model Context Protocol) client for Nuphus.
//!
//! Provides stdio-based JSON-RPC transport to external MCP servers,
//! with lazy-start connection pool and tools/list + tools/call support.

pub mod client;
pub mod config;
pub mod dual;
