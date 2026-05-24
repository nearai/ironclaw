//! First-party userland extension implementations for IronClaw.
//!
//! This crate owns concrete implementation behavior. Host runtime and
//! composition own declaration, authorization, accounting, lifecycle, and
//! loop-facing adapter wiring.
#![forbid(unsafe_code)]

pub mod coding;
pub mod skills;
