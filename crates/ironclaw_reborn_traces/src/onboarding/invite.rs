//! Invite link parsing. The operator-handed invite link is the trust root
//! (spec §2.1): the invite-derived origin is authoritative for the issuer.

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InviteParseError {
    #[error("invite link must use https (http allowed for loopback only)")]
    InsecureScheme,
    #[error("invite link is missing an invite code")]
    MissingCode,
    #[error("invite link is malformed: {reason}")]
    Malformed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvite {
    /// scheme + host + optional non-default port; no path/query/fragment.
    pub origin: String,
    pub issuer_host: String,
    pub code: String,
}

impl ParsedInvite {
    pub fn parse(raw: &str) -> Result<Self, InviteParseError> {
        let raw = raw.trim();
        // Bare `code@host[:port]` form (implies HTTPS). Route it back through
        // the URL parser as a synthetic canonical link rather than parsing it
        // ourselves — this reuses the userinfo/host/IPv6 validation below and
        // avoids a parallel parse path. The synthetic string contains `://`,
        // so the recursive `parse` call always takes the URL branch and can
        // never re-enter this bare-form branch (no infinite recursion).
        if !raw.contains("://")
            && let Some((code, host)) = raw.split_once('@')
        {
            return Self::parse(&format!("https://{host}#{code}"));
        }
        let url = reqwest::Url::parse(raw).map_err(|e| InviteParseError::Malformed {
            reason: e.to_string(),
        })?;
        // The invite link is the trust anchor: reject embedded credentials
        // (`https://user:pass@host/...`) outright rather than silently
        // stripping them, so userinfo can never corrupt the origin or leak
        // Basic Auth into `onboard_endpoint()`.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(InviteParseError::Malformed {
                reason: "invite link must not contain userinfo".to_string(),
            });
        }
        let scheme = url.scheme();
        // `host_str()` keeps IPv6 brackets (returns `[::1]`), so the rebuilt
        // authority is already a valid URL authority; `host_only` below strips
        // the brackets for the loopback check and `issuer_host`.
        let host = url.host_str().ok_or_else(|| InviteParseError::Malformed {
            reason: "missing host".to_string(),
        })?;
        let host_port = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        // Code: fragment wins, then ?code= query param.
        let code = url
            .fragment()
            .map(str::to_string)
            .filter(|f| !f.trim().is_empty())
            .or_else(|| {
                url.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.into_owned())
            })
            .ok_or(InviteParseError::MissingCode)?;
        Self::from_parts(scheme, &host_port, &code)
    }

    fn from_parts(scheme: &str, host_port: &str, code: &str) -> Result<Self, InviteParseError> {
        let code = code.trim();
        if code.is_empty() {
            return Err(InviteParseError::MissingCode);
        }
        let host_only = host_only(host_port).to_ascii_lowercase();
        if !is_https_or_loopback(scheme, &host_only) {
            return Err(InviteParseError::InsecureScheme);
        }
        Ok(Self {
            origin: format!("{scheme}://{host_port}"),
            issuer_host: host_only,
            code: code.to_string(),
        })
    }

    pub fn onboard_endpoint(&self) -> String {
        format!("{}/v1/onboard", self.origin)
    }

    /// Canonical server allowlist subject hash for the invite code.
    pub fn invite_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"invite:");
        hasher.update(self.code.as_bytes());
        format!("sha256:{}", hex::encode(hasher.finalize()))
    }

    /// Filename-safe stable id for local pending key material.
    pub fn pending_key_hash(&self) -> String {
        self.invite_hash().replace(':', "_")
    }
}

/// Returns true if `scheme`+`host_only` satisfies the https-or-loopback rule:
/// HTTPS is always allowed; HTTP is allowed only for loopback hosts.
/// Additionally, regardless of scheme, multicast and link-local IPs are always
/// rejected (defense-in-depth against cloud-metadata and SSRF abuse).
/// Private ranges (10/8, 172.16/12, 192.168/16) and loopback remain accepted.
/// `host_only` must already be lowercased and bracket-stripped (as produced by
/// the `host_only` helper below).
pub(crate) fn is_https_or_loopback(scheme: &str, host_only: &str) -> bool {
    // Block multicast and link-local IPs regardless of scheme. This covers the
    // cloud-metadata address (169.254.169.254 is link-local) and link-local
    // IPv6 (fe80::/10). Private ranges and loopback are NOT blocked here.
    if let Ok(ip) = host_only.parse::<std::net::IpAddr>() {
        let blocked = ip.is_multicast()
            || match ip {
                std::net::IpAddr::V4(v4) => v4.is_link_local(), // 169.254.0.0/16
                std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80, // fe80::/10
            };
        if blocked {
            return false;
        }
    }
    if scheme == "https" {
        return true;
    }
    if scheme == "http" {
        return is_loopback_host(host_only);
    }
    false
}

/// Returns true if the host is literally loopback: `localhost` or a loopback
/// IP. Accepts bracketed IPv6 (`[::1]`) and mixed case. This is the only host
/// shape for which a non-HTTPS Trace Commons endpoint can enter the policy
/// (the loopback-HTTP dev invite form above), so the claim/profile/ingest
/// validators in `contribution.rs` honor the same exception.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    let bare = host_only(host).to_ascii_lowercase();
    bare == "localhost"
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Given a fully-parsed `reqwest::Url`, return the scheme://host[:port] origin
/// string in the same format as `ParsedInvite::origin` (brackets preserved for
/// IPv6, non-default port included). Returns `None` if the URL has no host.
pub(crate) fn origin_of(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?;
    let scheme = url.scheme();
    let host_port = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    Some(format!("{scheme}://{host_port}"))
}

/// Extract the bare host (no port, no brackets) from a host[:port] authority.
/// Handles three shapes:
///   - bracketed IPv6 `[::1]` or `[::1]:3917` -> `::1`
///   - bare IPv6 literal `2001:db8::1` (>=2 colons, no brackets) -> unchanged
///   - host or IPv4, optionally `:port` -> host part before the single colon
///
/// Note: an *unbracketed* IPv6 host *with* a port is inherently ambiguous
/// (`::1:3917` could be host `::1` port `3917` or the address `::1:3917`).
/// That shape is unreachable here: every caller routes through the URL parser
/// (the bare `code@host` form is reparsed as a synthetic `https://` URL), and
/// the URL parser rejects unbracketed IPv6 authorities and always emits IPv6
/// hosts bracketed. The bare-IPv6 branch below therefore only ever sees a
/// portless literal.
pub(crate) fn host_only(host_port: &str) -> &str {
    if let Some(rest) = host_port.strip_prefix('[') {
        // Bracketed IPv6: take everything up to the closing bracket.
        return rest.split(']').next().unwrap_or(rest);
    }
    // A bare host with 2+ colons is an unbracketed IPv6 literal, not host:port.
    if host_port.matches(':').count() >= 2 {
        return host_port;
    }
    host_port.split(':').next().unwrap_or(host_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_fragment_form() {
        let p = ParsedInvite::parse("https://issuer.example.com/onboard#INV9K3RT5FBQ72JX").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com");
        assert_eq!(p.code, "INV9K3RT5FBQ72JX");
        assert_eq!(p.issuer_host, "issuer.example.com");
    }

    #[test]
    fn parses_query_form_and_discards_path() {
        let p = ParsedInvite::parse("https://issuer.example.com:8443/anything/else?code=INVAAAA")
            .unwrap();
        assert_eq!(p.origin, "https://issuer.example.com:8443");
        assert_eq!(p.code, "INVAAAA");
    }

    #[test]
    fn parses_code_at_host_form_implying_https() {
        let p = ParsedInvite::parse("INV9K3RT5FBQ72JX@issuer.example.com").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com");
        let p = ParsedInvite::parse("INVAAAA@issuer.example.com:8443").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com:8443");
    }

    #[test]
    fn rejects_non_loopback_http() {
        assert!(matches!(
            ParsedInvite::parse("http://issuer.example.com/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn allows_loopback_http_for_dev() {
        let p = ParsedInvite::parse("http://localhost:3917/onboard#INVAAAA").unwrap();
        assert_eq!(p.origin, "http://localhost:3917");
        assert!(ParsedInvite::parse("http://127.0.0.1:3917/onboard#INVAAAA").is_ok());
    }

    #[test]
    fn allows_loopback_ipv6_http() {
        let p = ParsedInvite::parse("http://[::1]:3917/onboard#INVAAAA").unwrap();
        assert_eq!(p.origin, "http://[::1]:3917");
        assert_eq!(p.issuer_host, "::1");
        assert_eq!(p.code, "INVAAAA");
    }

    #[test]
    fn allows_non_loopback_ipv6_https() {
        let p = ParsedInvite::parse("https://[2001:db8::1]/onboard#INVAAAA").unwrap();
        assert_eq!(p.origin, "https://[2001:db8::1]");
        assert_eq!(p.issuer_host, "2001:db8::1");
    }

    #[test]
    fn rejects_non_loopback_ipv6_http() {
        assert!(matches!(
            ParsedInvite::parse("http://[2001:db8::1]/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn parses_code_at_ipv6_host_form() {
        let p = ParsedInvite::parse("INVAAAA@[::1]:3917").unwrap();
        assert_eq!(p.origin, "https://[::1]:3917");
        assert_eq!(p.issuer_host, "::1");
    }

    #[test]
    fn rejects_empty_or_whitespace_code() {
        assert!(matches!(
            ParsedInvite::parse("https://issuer.example.com/onboard#  "),
            Err(InviteParseError::MissingCode)
        ));
        assert!(ParsedInvite::parse("https://issuer.example.com/onboard").is_err());
    }

    #[test]
    fn onboard_endpoint_is_origin_plus_v1_onboard() {
        let p = ParsedInvite::parse("https://issuer.example.com/onboard#INVAAAA").unwrap();
        assert_eq!(
            p.onboard_endpoint(),
            "https://issuer.example.com/v1/onboard"
        );
    }

    #[test]
    fn invite_hash_is_sha256_prefixed_hex() {
        let p = ParsedInvite::parse("https://h.example/onboard#INVAAAA").unwrap();
        let hash = p.invite_hash();
        assert_eq!(hash.len(), 71);
        let digest = hash
            .strip_prefix("sha256:")
            .expect("invite hash must use canonical sha256 prefix");
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn invite_hash_matches_server_sha256_scheme() {
        // Ground-truth vector: `printf '%s' INVAAAA | shasum -a 256`.
        // Pins our hash to the server's allowlist subject_hash scheme so a
        // drift (e.g. trailing newline, hex casing, different digest) can't
        // pass silently.
        let p = ParsedInvite::parse("https://h.example/onboard#INVAAAA").unwrap();
        assert_eq!(
            p.invite_hash(),
            "sha256:06f41d1d6db426b1a7da035727af91450134b3711b4903e5c701bb912ec5737a"
        );
    }

    #[test]
    fn pending_key_hash_is_filename_safe() {
        let p = ParsedInvite::parse("https://h.example/onboard#INVAAAA").unwrap();
        assert_eq!(
            p.pending_key_hash(),
            "sha256_06f41d1d6db426b1a7da035727af91450134b3711b4903e5c701bb912ec5737a"
        );
    }

    #[test]
    fn rejects_bare_form_with_embedded_userinfo() {
        // First '@' split makes the synthetic URL `https://user:pass@real-host#INV`;
        // the userinfo rejection then catches it instead of letting "user"
        // become the issuer_host or credentials enter the origin.
        assert!(matches!(
            ParsedInvite::parse("INV@user:pass@real-host"),
            Err(InviteParseError::Malformed { .. })
        ));
    }

    #[test]
    fn rejects_url_form_with_userinfo() {
        assert!(matches!(
            ParsedInvite::parse("https://user:pass@host/onboard#CODE"),
            Err(InviteParseError::Malformed { .. })
        ));
        // Username-only userinfo is rejected too.
        assert!(matches!(
            ParsedInvite::parse("https://user@host/onboard#CODE"),
            Err(InviteParseError::Malformed { .. })
        ));
    }

    #[test]
    fn bare_form_host_with_fragment_does_not_leak() {
        // `INV@host#extra` -> synthetic `https://host#extra#INV`. The url crate
        // treats everything after the first '#' as the fragment, so the code
        // becomes `extra#INV` and the origin stays clean (`https://host`) — the
        // '#' never leaks into the origin/issuer_host trust anchor.
        let p = ParsedInvite::parse("INV@host#extra").unwrap();
        assert_eq!(p.origin, "https://host");
        assert_eq!(p.issuer_host, "host");
        assert_eq!(p.code, "extra#INV");
    }

    #[test]
    fn allows_127_8_loopback_range_http() {
        // IpAddr::is_loopback covers the whole 127.0.0.0/8 block, not just .1.
        let p = ParsedInvite::parse("http://127.0.0.2:3917/onboard#INVAAAA").unwrap();
        assert_eq!(p.origin, "http://127.0.0.2:3917");
        assert_eq!(p.issuer_host, "127.0.0.2");
    }

    // ── Finding 3: block cloud-metadata / link-local / multicast IPs ──────────

    #[test]
    fn rejects_cloud_metadata_ip_169_254_169_254() {
        // 169.254.169.254 is the cloud-metadata address (link-local, IPv4).
        assert!(matches!(
            ParsedInvite::parse("https://169.254.169.254/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn rejects_link_local_ipv4_169_254_x_x() {
        // Any 169.254.x.x (link-local range) must be rejected.
        assert!(matches!(
            ParsedInvite::parse("https://169.254.1.1/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn rejects_link_local_ipv6_fe80() {
        // fe80::1 is in the link-local fe80::/10 range.
        assert!(matches!(
            ParsedInvite::parse("https://[fe80::1]/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn rejects_multicast_ipv4() {
        // 224.0.0.1 is a multicast address.
        assert!(matches!(
            ParsedInvite::parse("https://224.0.0.1/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn allows_private_ipv4_10_x_x_x() {
        // 10.x.x.x is a private range; must remain accepted.
        let p = ParsedInvite::parse("https://10.0.0.5/onboard#INVAAAA").unwrap();
        assert_eq!(p.issuer_host, "10.0.0.5");
    }

    #[test]
    fn allows_private_ipv4_192_168_x_x() {
        // 192.168.x.x is a private range; must remain accepted.
        let p = ParsedInvite::parse("https://192.168.1.10/onboard#INVAAAA").unwrap();
        assert_eq!(p.issuer_host, "192.168.1.10");
    }

    #[test]
    fn rejects_bare_form_unbracketed_ipv6() {
        // `INV@2001:db8::1` -> synthetic `https://2001:db8::1#INV`. The url
        // crate rejects an unbracketed IPv6 authority, so this is Malformed.
        assert!(matches!(
            ParsedInvite::parse("INV@2001:db8::1"),
            Err(InviteParseError::Malformed { .. })
        ));
    }
}
