//! Durable libSQL-backed [`PredicateStateBackend`] (durable-backend PR 3/4).
//!
//! This crate is the libSQL sibling of the in-memory backend that ships in
//! `ironclaw_hooks`. The framework crate (`ironclaw_hooks`) owns the public
//! [`PredicateStateBackend`] trait and its supporting types; this crate
//! depends on it and provides a concrete durable implementation. The
//! dependency direction is backend-crate → framework-crate, so database
//! dependencies (`libsql`) never leak into the framework crate. This mirrors
//! the intended layout for the Postgres sibling (durable-backend PR 2/4),
//! which lives in its own `ironclaw_hooks_postgres` crate for the same reason.
//!
//! [`PredicateStateBackend`]: ironclaw_hooks::predicate_state::PredicateStateBackend
//!
//! # Public surface
//!
//! - [`LibSqlPredicateStateBackend`] — the durable backend impl.
//!
//! Everything else (schema, scope hashing, transaction helpers) is internal.
//!
//! See [`backend`] for the implementation notes (schema, atomicity,
//! fail-closed cap enforcement, clock basis).

mod backend;
mod hashing;
mod schema;

pub use backend::LibSqlPredicateStateBackend;
