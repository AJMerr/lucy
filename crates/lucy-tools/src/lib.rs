//! Lucy's own tools, served over MCP so the core treats them exactly like external servers:
//! same safety labels, same permission rules, same confirmations (ADR 0004).

pub mod builtin;
mod html;
pub mod paths;

pub use paths::PrivatePaths;
