//! Reborn automation cluster — the product automations facade and the
//! trigger-poller that backs it. Grouped behind one internal module; the
//! crate root re-exports the same public items so the public API is unchanged.

pub(crate) mod facade;
pub(crate) mod trigger_poller;
pub(crate) mod trigger_poller_trusted_submit;
