//! Core type definitions for the engine.
//!
//! All data structures live here. No async, no I/O — just types and
//! validation logic.

pub mod capability;
pub mod conversation;
pub mod error;
pub mod event;
pub mod memory;
pub mod message;
pub mod project;
pub mod provenance;
pub mod step;
pub mod thread;
