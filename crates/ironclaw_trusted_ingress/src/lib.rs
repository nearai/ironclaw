//! Host-only trusted ingress authority tokens for IronClaw Reborn.
//!
//! This crate does not parse product payloads and does not own conversation
//! binding. It exists so trusted ingress constructors can require a host-owned
//! authority object instead of exposing public helper functions that any
//! adapter-facing crate can call by convention.
#![warn(unreachable_pub)]

/// Host authority for scheduled trigger ingress.
///
/// Possession of this value means the caller is host/composition-owned and is
/// minting a trusted trigger request from durable trigger state, not from a
/// product adapter payload. Workspace architecture tests restrict which crates
/// may depend on this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTrustedTriggerIngress {
    _private: (),
}

impl HostTrustedTriggerIngress {
    /// Mint host-owned trigger ingress authority at the composition root.
    #[cfg(feature = "composition-root")]
    pub fn new_for_composition_root() -> Self {
        Self { _private: () }
    }

    #[cfg(test)]
    fn new_for_tests() -> Self {
        Self { _private: () }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_ingress_authority_is_zero_sized() {
        let authority = HostTrustedTriggerIngress::new_for_tests();

        assert_eq!(core::mem::size_of_val(&authority), 0);
    }
}
