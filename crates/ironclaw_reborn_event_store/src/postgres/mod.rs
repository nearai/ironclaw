//! PostgreSQL-backed Reborn durable store modules.
//!
//! Each module implements the backend for one durable store using a
//! `deadpool_postgres::Pool`. All DDL is managed through the incremental
//! migration table in [`crate::postgres::migrations`].

pub mod gate_resolution;
pub mod migrations;

pub use gate_resolution::{PostgresGateResolutionStore, run_postgres_gate_migrations};
