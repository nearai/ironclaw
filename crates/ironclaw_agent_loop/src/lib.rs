//! Agent-loop framework state and strategy contracts for IronClaw Reborn.
//!
//! This crate owns the framework layer above `ironclaw_turns`. The master
//! architecture is `docs/reborn/agent-loop-skeleton.md`; workstream briefs live
//! under `docs/reborn/agent-loop-briefs/`.

pub mod state;
pub(crate) mod strategies;
