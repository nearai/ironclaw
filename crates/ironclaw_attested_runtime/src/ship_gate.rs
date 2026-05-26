//! The `CUSTODIAL_MAINNET_ENABLED` ship-gate (threat #18).
//!
//! Mirrors the `HOOKS_THIRD_PARTY_ENABLED` env-gate pattern: a dangerous
//! capability (here, custodial *mainnet* / real-value signing with keys
//! IronClaw holds) is refused unless an operator explicitly opts in **and** a
//! secure-custody HSM/KMS backend is wired. The opt-in flag alone is never
//! sufficient — a hot key in process memory can only ever sign testnet / dev
//! value (compromised-host hot-key threat).
//!
//! This wraps the lower-level [`ironclaw_chain_signing::ShipGate`] (which
//! encodes the actual allow/deny logic and the mainnet-vs-testnet
//! classification) and adds the env-driven construction so the composition
//! layer reads one flag and hands a built gate to the custodial signer.

use ironclaw_chain_signing::{HsmKmsBackend, ShipGate};

/// The environment variable that opts a deployment into custodial mainnet
/// signing. Necessary but NOT sufficient: secure custody is still required.
pub const CUSTODIAL_MAINNET_ENABLED_ENV: &str = "CUSTODIAL_MAINNET_ENABLED";

/// The composition-layer custodial-mainnet ship-gate.
///
/// Reads the `CUSTODIAL_MAINNET_ENABLED` opt-in and builds the chain-signing
/// [`ShipGate`] from it plus the wired KMS backend (if any).
pub struct CustodialMainnetShipGate {
    opt_in: bool,
}

impl CustodialMainnetShipGate {
    /// Build from an explicit opt-in flag (used by tests / callers that own the
    /// config).
    pub fn new(opt_in: bool) -> Self {
        Self { opt_in }
    }

    /// Build by reading the `CUSTODIAL_MAINNET_ENABLED` env var. Anything other
    /// than a truthy value (`1`, `true`, `yes`, case-insensitive) is treated as
    /// opted-out — fail-closed default.
    pub fn from_env() -> Self {
        Self {
            opt_in: parse_opt_in(std::env::var(CUSTODIAL_MAINNET_ENABLED_ENV).ok().as_deref()),
        }
    }

    /// Whether the operator opted into mainnet custodial signing.
    pub fn mainnet_opt_in(&self) -> bool {
        self.opt_in
    }

    /// Build the lower-level chain-signing [`ShipGate`] for the custodial
    /// signer, binding the operator opt-in to the wired KMS backend.
    ///
    /// A `None` backend (no KMS) or a hot-key backend
    /// (`is_secure_custody() == false`) cannot satisfy the mainnet requirement,
    /// regardless of the opt-in (threat #18). Testnet / dev signing is always
    /// allowed.
    pub fn build_chain_ship_gate(&self, kms: Option<&dyn HsmKmsBackend>) -> ShipGate {
        ShipGate::new(self.opt_in, kms)
    }
}

/// Parse the `CUSTODIAL_MAINNET_ENABLED` value into an opt-in flag. Anything
/// other than a truthy value (`1`, `true`, `yes`, case-insensitive, trimmed) —
/// including an absent var — is opted-out (fail-closed default).
fn parse_opt_in(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_truthy_values_opt_in() {
        for v in ["1", "true", "TRUE", "Yes", " yes ", "tRuE"] {
            assert!(
                parse_opt_in(Some(v)),
                "value {v:?} must be treated as opted-in"
            );
        }
    }

    #[test]
    fn from_env_falsy_and_unset_default_to_opted_out() {
        for v in ["0", "false", "no", "", "nope", "2", "  "] {
            assert!(
                !parse_opt_in(Some(v)),
                "value {v:?} must be treated as opted-out (fail-closed)"
            );
        }
        // An absent var also fails closed.
        assert!(!parse_opt_in(None), "unset must default to opted-out");
    }

    /// The public `from_env` constructor wires `parse_opt_in`; with the var
    /// absent (the harness does not set it) it must report opted-out.
    #[test]
    fn from_env_constructor_defaults_opted_out_when_absent() {
        // We do not mutate the process env (the crate forbids unsafe); we only
        // assert the safe default, which holds whenever the var is unset.
        if std::env::var(CUSTODIAL_MAINNET_ENABLED_ENV).is_err() {
            assert!(!CustodialMainnetShipGate::from_env().mainnet_opt_in());
        }
    }
}
