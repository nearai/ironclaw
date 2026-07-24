//! Reborn product-auth production services.
//!
//! Auth-owned contracts, flow/account stores, refresh helpers, OAuth engine
//! helpers, continuations, cleanup, and fakes live here. HTTP route serving and
//! product-specific prompt rendering stay in product/host crates.

pub mod api;
pub mod credentials;
pub mod durable;
pub mod oauth;
