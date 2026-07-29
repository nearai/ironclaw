//! Shared helpers for the live JSON-RPC broadcasters (EVM / Solana / NEAR).
//!
//! These run only under `feature = "broadcast-http"`. Every broadcaster is a
//! one-shot submitter of an already-signed payload — none re-signs, bumps a
//! nonce, or refreshes a blockhash. Re-broadcast requires a fresh approval,
//! enforced upstream by the signing-ledger broadcast-idempotency guard.

#![cfg(feature = "broadcast-http")]

use std::net::IpAddr;

use crate::error::ChainSigningError;

/// A JSON-RPC endpoint URL that has been validated against SSRF / network-policy
/// rules **at construction time**.
///
/// Raw config strings are never handed to a broadcaster. Each per-chain
/// broadcaster takes an `RpcEndpoint`, and the only way to build one is through
/// [`RpcEndpoint::parse`] (or [`RpcEndpoint::parse_with_allowlist`]), which
/// rejects:
///
/// * any scheme other than `http`/`https`,
/// * URL userinfo (`user:pass@host`), which can smuggle a target past naive
///   host checks,
/// * a literal-IP host inside a cloud-metadata / link-local / loopback /
///   unspecified / private / CGNAT range (IPv4 and IPv6, including
///   IPv4-mapped-IPv6), unless that exact host string is explicitly
///   allow-listed.
///
/// This is the config-time guard: it stops an operator (or a compromised config
/// source) from pointing a broadcaster at `http://169.254.169.254/…`,
/// `http://127.0.0.1`, `http://10.0.0.5`, `http://[::1]`, etc. DNS-level
/// rebinding defense (resolving a *hostname* to a private IP at request time)
/// is the responsibility of routing outbound HTTP through
/// `ironclaw_network::PolicyNetworkHttpEgress`; this newtype validates the
/// statically-supplied endpoint, which is the layer the broadcaster owns.
#[derive(Debug, Clone)]
pub struct RpcEndpoint {
    url: String,
}

impl RpcEndpoint {
    /// Parse and validate `raw` with no host allow-list (the strict default:
    /// every internal/metadata/loopback literal IP is rejected).
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, ChainSigningError> {
        Self::parse_with_allowlist(raw, &[])
    }

    /// Parse and validate `raw`, permitting a literal-IP host only when its host
    /// string appears verbatim in `allowlist` (e.g. a deliberately-configured
    /// private RPC node). The scheme and userinfo checks still apply.
    pub fn parse_with_allowlist(
        raw: impl AsRef<str>,
        allowlist: &[&str],
    ) -> Result<Self, ChainSigningError> {
        let raw = raw.as_ref();
        let reject = |reason: String| ChainSigningError::Broadcast {
            chain: "rpc-endpoint",
            reason,
        };

        let url = reqwest::Url::parse(raw)
            .map_err(|error| reject(format!("invalid RPC endpoint URL: {error}")))?;

        match url.scheme() {
            "http" | "https" => {}
            other => {
                return Err(reject(format!(
                    "unsupported RPC endpoint scheme {other:?} (only http/https)"
                )));
            }
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(reject(
                "RPC endpoint URL must not contain userinfo".to_string(),
            ));
        }

        let host = url
            .host_str()
            .filter(|host| !host.trim().is_empty())
            .ok_or_else(|| reject("RPC endpoint URL is missing a host".to_string()))?;

        // A bracketed/literal IP host that resolves to an internal range is
        // rejected unless explicitly allow-listed. Hostnames pass this gate (DNS
        // rebinding is handled by the policy egress transport at request time).
        if let Some(ip) = parse_host_ip(host) {
            let allow = allowlist
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(host));
            if !allow && is_blocked_ip(ip) {
                return Err(reject(format!(
                    "RPC endpoint host {host} is in a blocked internal/metadata range"
                )));
            }
        }

        Ok(Self {
            url: url.to_string(),
        })
    }

    /// The validated endpoint URL as a string.
    pub fn as_str(&self) -> &str {
        &self.url
    }
}

/// Parse a URL host component into an [`IpAddr`] if it is a literal IP. `url`
/// reports IPv6 hosts bracketed (`[::1]`), so strip a single pair of brackets
/// before attempting to parse.
fn parse_host_ip(host: &str) -> Option<IpAddr> {
    let candidate = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    candidate.parse::<IpAddr>().ok()
}

/// True when `ip` is in a range a broadcaster must never reach: cloud-metadata,
/// link-local, loopback, unspecified, private, multicast/broadcast,
/// documentation, CGNAT, or `0.0.0.0/8` — across IPv4, IPv6, and
/// IPv4-mapped-IPv6.
///
/// This mirrors `ironclaw_network`'s `is_private_or_loopback_ip` (which is
/// `pub(crate)` there and so cannot be reused directly); keeping the policy in
/// lockstep is covered by the unit tests below.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local() // 169.254.0.0/16 — includes 169.254.169.254 metadata
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || is_cgnat_v4(ip)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(mapped));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || is_unique_local_v6(ip)
                || is_unicast_link_local_v6(ip)
        }
    }
}

/// 100.64.0.0/10 carrier-grade NAT.
fn is_cgnat_v4(ip: std::net::Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && (64..=127).contains(&second)
}

/// fc00::/7 unique-local addresses (stable across Rust versions without the
/// unstable `Ipv6Addr::is_unique_local`).
fn is_unique_local_v6(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// fe80::/10 link-local unicast.
fn is_unicast_link_local_v6(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// Default per-request timeout for a one-shot broadcast submission. A signed
/// payload is submitted exactly once; without a deadline a hung or slow RPC
/// node would block the broadcast task indefinitely (request-hang DoS). The
/// timeout bounds a single submit/poll round-trip — it does NOT cause a
/// re-broadcast (the broadcasters never re-sign or resubmit; a timeout surfaces
/// as a `Broadcast` error and the ledger row is resolved out-of-band).
pub(crate) const RPC_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Build the rustls-backed reqwest client every broadcaster shares, with the
/// default one-shot [`RPC_REQUEST_TIMEOUT`] applied. `chain` tags any builder
/// error. Callers needing custom timeout/proxy/policy use `with_client` instead.
pub(crate) fn build_broadcast_client(
    chain: &'static str,
) -> Result<reqwest::Client, ChainSigningError> {
    reqwest::Client::builder()
        .timeout(RPC_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| ChainSigningError::Broadcast {
            chain,
            reason: format!("failed to build HTTP client: {error}"),
        })
}

/// Maximum JSON-RPC response body we will buffer. A submission RPC reply is a
/// hash / signature / small error object; a multi-megabyte body is hostile (or
/// a misconfigured endpoint) and must not be allowed to exhaust memory.
pub(crate) const MAX_RPC_RESPONSE_BYTES: usize = 64 * 1024;

/// The JSON-RPC request id we send. The node MUST echo it back; a mismatch
/// means the response was not for our request (proxy crosstalk, hostile node)
/// and is rejected rather than parsed for a `result`.
pub(crate) const RPC_REQUEST_ID: i64 = 1;

/// Read a JSON-RPC HTTP response defensively: enforce a body-size cap, parse as
/// JSON, reject a mismatched `id`, surface an `error` object, and reject a
/// `result` that is absent. Returns the `result` value's string form via the
/// supplied `extract`. `chain` tags any error.
///
/// This centralizes hostile-response handling for every per-chain broadcaster
/// so the size cap, id check, and error-object handling can't drift apart.
pub(crate) async fn read_jsonrpc_result(
    chain: &'static str,
    response: reqwest::Response,
) -> Result<serde_json::Value, ChainSigningError> {
    let reject = move |reason: String| ChainSigningError::Broadcast { chain, reason };

    // Reject an oversized body up front when the server declares one.
    if let Some(len) = response.content_length()
        && len as usize > MAX_RPC_RESPONSE_BYTES
    {
        return Err(reject(format!(
            "RPC response too large: {len} bytes (cap {MAX_RPC_RESPONSE_BYTES})"
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| reject(format!("failed to read RPC response body: {error}")))?;
    parse_jsonrpc_body(chain, &bytes)
}

/// Pure JSON-RPC body validation, factored out of [`read_jsonrpc_result`] so the
/// hostile-response handling (size cap, id check, error object, missing result)
/// is unit-testable without a live HTTP server.
pub(crate) fn parse_jsonrpc_body(
    chain: &'static str,
    bytes: &[u8],
) -> Result<serde_json::Value, ChainSigningError> {
    let reject = move |reason: String| ChainSigningError::Broadcast { chain, reason };

    // Defend against a chunked/undisclosed-length oversized body.
    if bytes.len() > MAX_RPC_RESPONSE_BYTES {
        return Err(reject(format!(
            "RPC response too large: {} bytes (cap {MAX_RPC_RESPONSE_BYTES})",
            bytes.len()
        )));
    }

    let body: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| reject(format!("invalid JSON-RPC response: {error}")))?;

    // The id must echo the request id. Anything else is crosstalk.
    match body.get("id") {
        Some(id) if id == &serde_json::json!(RPC_REQUEST_ID) => {}
        Some(other) => {
            return Err(reject(format!(
                "JSON-RPC response id mismatch: expected {RPC_REQUEST_ID}, got {other}"
            )));
        }
        None => return Err(reject("JSON-RPC response missing id".to_string())),
    }

    if let Some(error) = body.get("error") {
        return Err(reject(format!("node rejected transaction: {error}")));
    }

    body.get("result")
        .cloned()
        .ok_or_else(|| reject("JSON-RPC response missing result".to_string()))
}

/// Validate that `value` looks like a NEAR base58 transaction hash: a base58
/// string decoding to exactly 32 bytes (a NEAR `CryptoHash`). Returns the
/// canonical string on success. Rejects any other string so a hostile node
/// cannot smuggle an arbitrary value through as a "tx hash".
pub(crate) fn validate_near_tx_hash(
    chain: &'static str,
    value: &str,
) -> Result<String, ChainSigningError> {
    let reject = |reason: String| ChainSigningError::Broadcast { chain, reason };
    let decoded = bs58::decode(value)
        .into_vec()
        .map_err(|error| reject(format!("tx hash is not valid base58: {error}")))?;
    if decoded.len() != 32 {
        return Err(reject(format!(
            "tx hash decoded to {} bytes, expected 32",
            decoded.len()
        )));
    }
    Ok(value.to_string())
}

/// Lowercase-hex encode without an `0x` prefix.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decode lowercase/uppercase hex (no `0x` prefix) into bytes.
pub(crate) fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) {
        return Err("odd-length hex".to_string());
    }
    (0..input.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&input[index..index + 2], 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_http_and_https() {
        for url in [
            "https://rpc.example.com",
            "http://rpc.example.com:8545/",
            "https://mainnet.infura.io/v3/abc",
            "https://example.com:443/path?query=1",
        ] {
            let endpoint = RpcEndpoint::parse(url).expect("public URL must parse");
            assert!(endpoint.as_str().starts_with("http"));
        }
    }

    #[test]
    fn rejects_cloud_metadata_endpoint() {
        let err = RpcEndpoint::parse("http://169.254.169.254/latest/meta-data")
            .expect_err("metadata IP must be rejected");
        assert!(format!("{err}").contains("blocked"), "{err}");
    }

    #[test]
    fn rejects_loopback_v4() {
        assert!(RpcEndpoint::parse("http://127.0.0.1:8545").is_err());
        assert!(RpcEndpoint::parse("http://127.0.0.1").is_err());
    }

    #[test]
    fn rejects_loopback_v6() {
        assert!(RpcEndpoint::parse("http://[::1]:8545").is_err());
    }

    #[test]
    fn rejects_private_ranges() {
        for url in [
            "http://10.0.0.5:8545",
            "http://192.168.1.1",
            "http://172.16.0.1",
            "http://100.64.0.1", // CGNAT
            "http://0.0.0.0:8545",
        ] {
            assert!(RpcEndpoint::parse(url).is_err(), "{url} must be rejected");
        }
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_metadata() {
        // ::ffff:169.254.169.254 maps to the IPv4 metadata address.
        assert!(RpcEndpoint::parse("http://[::ffff:a9fe:a9fe]:80").is_err());
    }

    #[test]
    fn rejects_v6_unique_local_and_link_local() {
        assert!(RpcEndpoint::parse("http://[fc00::1]:8545").is_err());
        assert!(RpcEndpoint::parse("http://[fe80::1]:8545").is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(RpcEndpoint::parse("file:///etc/passwd").is_err());
        assert!(RpcEndpoint::parse("ftp://example.com").is_err());
        assert!(RpcEndpoint::parse("ws://example.com").is_err());
    }

    #[test]
    fn rejects_userinfo() {
        assert!(RpcEndpoint::parse("http://user:pass@example.com").is_err());
        assert!(RpcEndpoint::parse("http://user@169.254.169.254").is_err());
    }

    #[test]
    fn allowlist_permits_named_private_host() {
        // Explicit opt-in lets a deliberately-configured private node through,
        // but only that exact host string.
        let ok = RpcEndpoint::parse_with_allowlist("http://10.0.0.5:8545", &["10.0.0.5"]);
        assert!(ok.is_ok());
        let still_blocked =
            RpcEndpoint::parse_with_allowlist("http://10.0.0.6:8545", &["10.0.0.5"]);
        assert!(still_blocked.is_err());
    }

    // --- hostile JSON-RPC response handling -------------------------------

    #[test]
    fn parses_valid_result() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":"0xabc"}"#;
        let result = parse_jsonrpc_body("evm", body).expect("valid body");
        assert_eq!(result.as_str(), Some("0xabc"));
    }

    #[test]
    fn rejects_mismatched_id() {
        let body = br#"{"jsonrpc":"2.0","id":99,"result":"0xabc"}"#;
        let err = parse_jsonrpc_body("evm", body).expect_err("id mismatch");
        assert!(format!("{err}").contains("id mismatch"), "{err}");
    }

    #[test]
    fn rejects_missing_id() {
        let body = br#"{"jsonrpc":"2.0","result":"0xabc"}"#;
        assert!(parse_jsonrpc_body("evm", body).is_err());
    }

    #[test]
    fn surfaces_node_error_object() {
        let body = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"nonce too low"}}"#;
        let err = parse_jsonrpc_body("evm", body).expect_err("node error");
        assert!(
            format!("{err}").contains("node rejected transaction"),
            "{err}"
        );
    }

    #[test]
    fn rejects_missing_result() {
        let body = br#"{"jsonrpc":"2.0","id":1}"#;
        let err = parse_jsonrpc_body("evm", body).expect_err("missing result");
        assert!(format!("{err}").contains("missing result"), "{err}");
    }

    #[test]
    fn rejects_malformed_json() {
        let body = b"this is not json";
        assert!(parse_jsonrpc_body("evm", body).is_err());
    }

    #[test]
    fn rejects_oversized_body() {
        let mut body = br#"{"jsonrpc":"2.0","id":1,"result":""#.to_vec();
        body.extend(std::iter::repeat_n(b'a', MAX_RPC_RESPONSE_BYTES + 1));
        body.extend_from_slice(br#""}"#);
        let err = parse_jsonrpc_body("evm", &body).expect_err("oversized");
        assert!(format!("{err}").contains("too large"), "{err}");
    }

    #[test]
    fn near_tx_hash_accepts_valid_base58_32_bytes() {
        let hash = bs58::encode([7u8; 32]).into_string();
        let ok = validate_near_tx_hash("near", &hash).expect("valid near hash");
        assert_eq!(ok, hash);
    }

    #[test]
    fn near_tx_hash_rejects_arbitrary_string() {
        // Non-base58 characters.
        assert!(validate_near_tx_hash("near", "not a hash!!!").is_err());
        // Valid base58 but wrong length (decodes to != 32 bytes).
        let short = bs58::encode([1u8; 8]).into_string();
        assert!(validate_near_tx_hash("near", &short).is_err());
        let long = bs58::encode([1u8; 64]).into_string();
        assert!(validate_near_tx_hash("near", &long).is_err());
        // Empty.
        assert!(validate_near_tx_hash("near", "").is_err());
    }

    // --- hex round-trip helpers -------------------------------------------

    #[test]
    fn decode_hex_round_trips_with_hex_encode() {
        let bytes = [0x00u8, 0x0f, 0xa9, 0xfe, 0xff, 0x10];
        let encoded = hex_encode(&bytes);
        assert_eq!(encoded, "000fa9feff10");
        assert_eq!(decode_hex(&encoded).expect("decode"), bytes);
    }

    #[test]
    fn decode_hex_accepts_upper_and_lower_case() {
        assert_eq!(
            decode_hex("DeAdBeEf").expect("mixed case"),
            vec![0xde, 0xad, 0xbe, 0xef]
        );
    }

    #[test]
    fn decode_hex_accepts_empty() {
        assert_eq!(decode_hex("").expect("empty"), Vec::<u8>::new());
    }

    #[test]
    fn decode_hex_rejects_odd_length() {
        let err = decode_hex("abc").expect_err("odd length");
        assert!(err.contains("odd-length"), "{err}");
    }

    #[test]
    fn decode_hex_rejects_invalid_chars() {
        // Even length but a non-hex digit in the second byte.
        assert!(decode_hex("00zz").is_err());
        // Non-ASCII / punctuation.
        assert!(decode_hex("g0").is_err());
    }

    #[test]
    fn hostnames_pass_construction_gate() {
        // A hostname is not a literal IP; DNS rebinding is handled at request
        // time by the policy egress transport, not this construction gate.
        assert!(RpcEndpoint::parse("http://metadata.google.internal").is_ok());
    }
}
