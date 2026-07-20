//! Reborn product-auth cluster.
//!
//! Groups the product-auth surface — public API/prompt types (`api`), OAuth
//! provider composition (`oauth`), durable flow/account state (`durable`),
//! WebUI route serving (`serve`), and runtime credential resolution/refresh
//! (`credentials`) — behind one internal module. The crate root re-exports the
//! same public items from here so the crate's public API is unchanged.

pub(crate) mod api;
pub(crate) mod credentials;
pub(crate) mod durable;
pub(crate) mod oauth;
pub(crate) mod serve;
