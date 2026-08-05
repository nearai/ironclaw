//! Remote transport for Trace Commons.
//!
//! `claim` owns credentials and the HTTP substrate; `profile` and `account`
//! are the two product APIs built on it; `client` owns the shared outbound
//! client and its timeout. Nothing here decides queue state — that is
//! `submission`'s job.

mod account;
mod claim;
mod client;
mod profile;

pub use account::*;
pub use claim::*;
pub(crate) use client::*;
pub use profile::*;
