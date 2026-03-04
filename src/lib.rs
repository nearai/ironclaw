//! NEAR AI Agentic Worker Framework
//!
//! An LLM-powered autonomous agent that operates on the NEAR AI marketplace.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────────┐
//! │                              User Interaction Layer                              │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐                         │
//! │  │   CLI    │  │  Slack   │  │ Telegram │  │   HTTP   │                         │
//! │  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘                         │
//! │       └─────────────┴────────────┬┴─────────────┘                               │
//! └──────────────────────────────────┼──────────────────────────────────────────────┘
//!                                    ▼
//! ┌──────────────────────────────────────────────────────────────────────────────────┐
//! │                              Main Agent Loop                                      │
//! │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                      │
//! │  │ Message Router │──│  LLM Reasoning │──│ Action Executor│                      │
//! │  └────────────────┘  └───────┬────────┘  └───────┬────────┘                      │
//! │         ▲                    │                   │                               │
//! │         │         ┌──────────┴───────────────────┴──────────┐                    │
//! │         │         ▼                                         ▼                    │
//! │  ┌──────┴─────────────┐                         ┌───────────────────────┐        │
//! │  │   Safety Layer     │                         │    Self-Repair        │        │
//! │  │ - Input sanitizer  │                         │ - Stuck job detection │        │
//! │  │ - Injection defense│                         │ - Tool fixer          │        │
//! │  └────────────────────┘                         └───────────────────────┘        │
//! └──────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **Multi-channel interaction** - CLI, Slack, Telegram, HTTP webhooks
//! - **Parallel job execution** - Run multiple jobs with isolated contexts
//! - **Pluggable tools** - MCP, 3rd party services, dynamic tools
//! - **Self-repair** - Detect and fix stuck jobs and broken tools
//! - **Prompt injection defense** - Sanitize all external data
//! - **Continuous learning** - Improve estimates from historical data

pub mod agent;
pub mod app;
pub mod benchmark;
pub mod boot_screen;
pub mod bootstrap;
pub mod channels;
pub mod cli;
pub mod config;
pub mod context;
pub mod db;
pub mod error;
pub mod estimation;
pub mod evaluation;
pub mod extensions;
pub mod history;
pub mod hooks;
pub mod llm;
pub mod observability;
pub mod orchestrator;
pub mod pairing;
pub mod registry;
pub mod safety;
pub mod sandbox;
pub mod secrets;
pub mod service;
pub mod settings;
pub mod setup;
pub mod skills;
pub mod tools;
pub mod tracing_fmt;
pub mod tunnel;
pub mod util;
pub mod worker;
pub mod workspace;

#[cfg(test)]
pub mod testing;

pub use config::Config;
pub use error::{Error, Result};

/// Re-export commonly used types.
pub mod prelude {
    pub use crate::channels::{Channel, IncomingMessage, MessageStream};
    pub use crate::config::Config;
    pub use crate::context::{JobContext, JobState};
    pub use crate::error::{Error, Result};
    pub use crate::llm::LlmProvider;
    pub use crate::safety::{SanitizedOutput, Sanitizer};
    pub use crate::tools::{Tool, ToolOutput, ToolRegistry};
    pub use crate::workspace::{MemoryDocument, Workspace};
}
