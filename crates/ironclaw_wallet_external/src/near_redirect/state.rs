//! The gate-bound `state` parameter for the NEAR wallet redirect (threat #20).
//!
//! When IronClaw redirects the user to the NEAR wallet it embeds a `state`
//! parameter. The wallet echoes it back on the callback. To defeat redirect /
//! deep-link interception — an attacker pasting a callback for a *different*
//! gate, or replaying a callback against a different request — the `state` is
//! deterministically derived from the gate-identifying context plus the bound
//! [`ApprovedTxHash`], MAC'd with a server-side secret. The verifier
//! re-derives it from the authoritative context and requires an exact,
//! constant-time match; nothing in the callback is trusted to *carry* the
//! binding, only to echo it.
//!
//! The MAC is HMAC-SHA256 over the canonical context tuple, built on the `sha2`
//! primitive the crate already depends on (no new crypto dependency).

use sha2::{Digest, Sha256};

use ironclaw_signing_provider::{ApprovedTxHash, SigningContext};

/// Domain separation tag so this MAC can never collide with another use of the
/// same secret.
const STATE_DOMAIN: &[u8] = b"ironclaw.near_redirect.state.v1";

/// The decoded view of a state parameter (currently just the opaque MAC hex).
///
/// Kept as a named type so the wire shape can grow (e.g. an explicit nonce)
/// without churning call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearRedirectState {
    /// Lowercase-hex HMAC-SHA256 binding the gate context + approved hash.
    pub mac_hex: String,
}

/// Derive the gate-bound `state` parameter string.
///
/// Deterministic in `(secret, context, approved_tx_hash)`: the same gate always
/// produces the same state, and any change to who/which-run/which-gate/which-
/// chain/which-account/which-hash changes it.
pub fn derive_state(
    secret: &[u8],
    context: &SigningContext,
    approved_tx_hash: &ApprovedTxHash,
) -> String {
    let mac = hmac_sha256(secret, &state_message(context, approved_tx_hash));
    hex_encode(&mac)
}

/// Re-derive and constant-time compare the gate-bound state against the echoed
/// `candidate`. Returns `true` iff they match.
pub fn verify_state(
    secret: &[u8],
    context: &SigningContext,
    approved_tx_hash: &ApprovedTxHash,
    candidate: &str,
) -> bool {
    let expected = derive_state(secret, context, approved_tx_hash);
    constant_time_eq(expected.as_bytes(), candidate.as_bytes())
}

/// Encode a [`NearRedirectState`] to its wire string.
pub fn encode_state(state: &NearRedirectState) -> String {
    state.mac_hex.clone()
}

/// Decode a wire string into a [`NearRedirectState`]. Validation of the binding
/// happens in [`verify_state`]; this is only the shape parse.
pub fn decode_state(s: &str) -> NearRedirectState {
    NearRedirectState {
        mac_hex: s.to_string(),
    }
}

/// Build the canonical, length-prefixed message the MAC is computed over. Each
/// field is length-prefixed so no two distinct field tuples can collide.
fn state_message(context: &SigningContext, approved_tx_hash: &ApprovedTxHash) -> Vec<u8> {
    let mut msg = Vec::new();
    push_field(&mut msg, STATE_DOMAIN);
    push_field(&mut msg, context.tenant.as_str().as_bytes());
    push_field(&mut msg, context.user.as_str().as_bytes());
    push_field(&mut msg, context.run_id.as_str().as_bytes());
    push_field(&mut msg, context.gate_ref.as_str().as_bytes());
    push_field(&mut msg, context.chain_id.as_str().as_bytes());
    push_field(&mut msg, context.key_or_account_id.as_str().as_bytes());
    push_field(&mut msg, approved_tx_hash.as_bytes());
    msg
}

/// Length-prefix (8-byte big-endian) then append a field.
fn push_field(buf: &mut Vec<u8>, field: &[u8]) {
    buf.extend_from_slice(&(field.len() as u64).to_be_bytes());
    buf.extend_from_slice(field);
}

/// HMAC-SHA256 (RFC 2104) over `sha2::Sha256`. Implemented inline to avoid
/// pulling an `hmac` crate for this single use.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block_key = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        block_key[..32].copy_from_slice(&digest);
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= block_key[i];
        opad[i] ^= block_key[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();

    let mut mac = [0u8; 32];
    mac.copy_from_slice(&out);
    mac
}

/// Constant-time byte-slice equality (length-independent short-circuit only on
/// length, which is not secret here — the state is fixed-length hex).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };

    fn ctx(account: &str, gate: &str) -> SigningContext {
        SigningContext {
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new(gate),
            chain_id: ChainId::new("near:mainnet"),
            key_or_account_id: KeyOrAccountId::new(account),
        }
    }

    #[test]
    fn hmac_sha256_matches_rfc_test_vector() {
        // RFC 4231 test case 1: key = 0x0b * 20, data = "Hi There".
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex_encode(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn derive_is_deterministic() {
        let c = ctx("alice.near", "gate:1");
        let h = ApprovedTxHash::from_bytes([7u8; 32]);
        assert_eq!(
            derive_state(b"secret", &c, &h),
            derive_state(b"secret", &c, &h)
        );
    }

    #[test]
    fn verify_accepts_matching_state() {
        let c = ctx("alice.near", "gate:1");
        let h = ApprovedTxHash::from_bytes([7u8; 32]);
        let s = derive_state(b"secret", &c, &h);
        assert!(verify_state(b"secret", &c, &h, &s));
    }

    #[test]
    fn verify_rejects_different_gate() {
        let c1 = ctx("alice.near", "gate:1");
        let c2 = ctx("alice.near", "gate:2");
        let h = ApprovedTxHash::from_bytes([7u8; 32]);
        let s = derive_state(b"secret", &c1, &h);
        // A callback echoing gate:1's state must not validate against gate:2.
        assert!(!verify_state(b"secret", &c2, &h, &s));
    }

    #[test]
    fn verify_rejects_different_hash() {
        let c = ctx("alice.near", "gate:1");
        let h1 = ApprovedTxHash::from_bytes([7u8; 32]);
        let h2 = ApprovedTxHash::from_bytes([8u8; 32]);
        let s = derive_state(b"secret", &c, &h1);
        assert!(!verify_state(b"secret", &c, &h2, &s));
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let c = ctx("alice.near", "gate:1");
        let h = ApprovedTxHash::from_bytes([7u8; 32]);
        let s = derive_state(b"secret-a", &c, &h);
        assert!(!verify_state(b"secret-b", &c, &h, &s));
    }

    #[test]
    fn state_round_trips_through_wire_string() {
        let st = decode_state("deadbeef");
        assert_eq!(encode_state(&st), "deadbeef");
    }
}
