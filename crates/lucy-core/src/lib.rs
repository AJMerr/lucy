//! Lucy's core service.
//!
//! Owns everything stateful: provider profiles, chat sessions (SQLite), the persona, MCP tool
//! servers, mode/permission enforcement, and the IPC listeners front ends connect to.
//! See `docs/ROADMAP.md` for what lands in each milestone.

pub mod config;
pub mod generate;
pub mod mcp;
pub mod oauth;
pub mod persona;
pub mod pictures;
pub mod policy;
pub mod provider;
pub mod secrets;
pub mod server;
pub mod sessions;
pub mod setup;
pub mod tools;

pub use lucy_protocol as protocol;
pub use lucy_providers as providers;
