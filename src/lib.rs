//! RustyClaw library crate — exposes the SDK module for integration tests and embedding.
//!
//! Modules that depend on the TUI (`tui`, `session`, `spawn`) and compile-time
//! binary-only env vars (`deeplink`) are excluded — they only compile as part
//! of the `rustyclaw` binary.

/// Claude Code version this build is based on.
pub const BUILT_AGAINST_CLAUDE_VERSION: &str = "2.1.92";

pub mod api;
pub mod commands;
pub mod compact;
pub mod config;
pub mod cost;
pub mod distro;
pub mod hooks;
pub mod mcp;
pub mod permissions;
pub mod query_engine;
pub mod rag;
pub mod router;
pub mod sandbox;
pub mod settings;
pub mod skills;
pub mod tools;
pub mod voice;

pub mod sdk;
