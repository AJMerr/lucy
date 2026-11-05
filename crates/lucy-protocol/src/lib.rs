//! Types shared by the Lucy core and every front end (CLI, sprite, remote clients).
//!
//! Anything that crosses the IPC boundary lives here so all sides agree on the wire format.

pub mod rpc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// How much freedom Lucy has to run tools without asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Chat plus tools; risky actions require confirmation.
    #[default]
    Assist,
    /// Autonomous multi-step tasks, governed by an [`AgentLevel`].
    Agent,
}

/// Agent-mode freedom levels. See `docs/adr/0004-modes-and-permissions.md`.
///
/// Serialized as its number (0–3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum AgentLevel {
    /// Confirm every tool call.
    Supervised = 0,
    /// Run freely, but confirm irreversible or outbound actions.
    #[default]
    Guarded = 1,
    /// Run freely inside a configured workspace; confirm outside it.
    Workspace = 2,
    /// Never confirm; only step and cost caps apply.
    Unrestricted = 3,
}

impl TryFrom<u8> for AgentLevel {
    type Error = String;

    fn try_from(level: u8) -> Result<Self, Self::Error> {
        match level {
            0 => Ok(Self::Supervised),
            1 => Ok(Self::Guarded),
            2 => Ok(Self::Workspace),
            3 => Ok(Self::Unrestricted),
            other => Err(format!("agent level must be 0–3, got {other}")),
        }
    }
}

impl From<AgentLevel> for u8 {
    fn from(level: AgentLevel) -> Self {
        level as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_through_json() {
        let msg = Message::new(Role::User, "wake up, Lucy");
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"wake up, Lucy"}"#);
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), msg);
    }

    #[test]
    fn default_agent_level_is_guarded() {
        assert_eq!(AgentLevel::default(), AgentLevel::Guarded);
    }
}
