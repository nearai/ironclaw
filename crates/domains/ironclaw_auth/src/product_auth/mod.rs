//! Reborn product-auth production services — **the second of this crate's two
//! engines** (PROPOSAL §6.4.8).
//!
//! Auth-owned contracts, flow/account stores, refresh helpers, OAuth engine
//! helpers, continuations, cleanup, and fakes live here. HTTP route serving and
//! product-specific prompt rendering stay in product/host crates.
//!
//! # Module charter
//!
//! **Owns:** the *durable, product-facing* half of auth — the lifecycle a user
//! and a product surface can observe. Starting, resuming, listing and expiring
//! auth flows; credential accounts and their selection, recovery and refresh
//! serialization; secure interactions and manual-token submission; the
//! ownership-aware cleanup lifecycle; the filesystem-backed stores behind all
//! of it; and the OAuth turn-gate that pauses a run until a flow completes.
//!
//! **Never contains:** a vendor handshake. Constructing an authorize URL,
//! validating scopes against a recipe ceiling, exchanging or refreshing a
//! token against a vendor endpoint, and RFC 7591 dynamic client registration
//! are [`crate::engine`]'s, without exception.
//!
//! **The severance is the point, and it is enforced.** This module must not
//! name `engine`, and `engine` must not name this module — measured at zero
//! references in both directions and pinned by
//! `tests/module_charter.rs::the_two_engines_do_not_name_each_other`. The two
//! engines meet only through the shared vocabulary re-exported from the crate
//! root (`credential`, `provider`, `oauth`, `scope`, `ids`, `error`), which is
//! why that vocabulary is a **third** owner in `AGENTS.md`'s sub-owner map
//! rather than being charged to either engine.

pub mod api;
pub mod credentials;
pub mod durable;
pub mod oauth;
