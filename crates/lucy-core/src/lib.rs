//! Lucy's core service.
//!
//! Owns everything stateful: provider profiles, chat sessions (SQLite), the persona, MCP tool
//! servers, mode/permission enforcement, and the IPC listeners front ends connect to.
//! See `docs/ROADMAP.md` for what lands in each milestone.

pub mod config;
pub mod persona;
pub mod provider;
pub mod secrets;
pub mod sessions;

pub use lucy_protocol as protocol;
pub use lucy_providers as providers;
