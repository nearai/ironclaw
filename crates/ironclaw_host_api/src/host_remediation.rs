//! The **host-authored remediation** channel — the trusted sibling of
//! [`crate::SafeSummary`].
//!
//! ## Why a second text channel exists
//!
//! `SafeSummary` is the channel for text whose PROVENANCE is untrusted: a WASM
//! tool's stderr, an MCP server's error body, a provider's rejection message.
//! Its rule is deliberately paranoid — 512 bytes, no control characters, no
//! `{}[]<>/\`` delimiters (so no URLs, no paths, no backticks), and no
//! credential *vocabulary* at all. Anything that trips it collapses to
//! [`SafeSummary::placeholder`](crate::SafeSummary::placeholder).
//!
//! That rule is correct for untrusted output and *wrong* for host-authored
//! operator remediation. "Run `ironclaw config set google.client_id
//! <id>.apps.googleusercontent.com`, then confirm the client at
//! <https://console.cloud.google.com/apis/credentials>" is a multi-line
//! instruction containing a URL, backticks, and the word `client_secret` — it
//! fails `SafeSummary` four separate ways and degrades to "capability summary
//! unavailable", which is exactly the dead end the remediation exists to
//! prevent.
//!
//! ## The distinction is PROVENANCE, not content shape
//!
//! This type does **not** sniff whether text "looks like remediation" — that is
//! a content heuristic and it does not work. It is a distinct channel that only
//! host code puts values into. The bound below is therefore a **value guard,
//! not a vocabulary guard**: credential *words* (`client_secret`, `password`)
//! are allowed because a host-authored instruction must be able to name the key
//! it is telling the operator to set, while credential *values* (`sk-…`,
//! `GOCSPX-…`, `ghp_…`, `xoxb-…`, long high-entropy runs) are rejected because
//! no legitimate host-authored instruction ever embeds one.
//!
//! ## INVARIANT: only host code may construct this
//!
//! [`HostRemediation::new`] is `pub` because the producers live in other crates
//! (`ironclaw_host_runtime`, `ironclaw_reborn_composition`), so there is no
//! compiler-enforceable way to restrict it. The invariant is therefore
//! documented and **tested**: capability output whose provenance is a WASM
//! module, an MCP server, or any first-party dispatch of arbitrary tool output
//! must never be routed through this type — it stays on the `SafeSummary`
//! channel and keeps collapsing to the placeholder. If you are reaching for
//! this type to stop some other text from being dropped, check the provenance
//! first: if the string is not a fixed host-authored constant (or built
//! entirely from host-authored constants), it does not belong here.

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Maximum length of a host-authored remediation, in bytes. Generous relative
/// to [`crate::SafeSummary`]'s 512 because a multi-step operator instruction is
/// genuinely long; matches the model-observation detail cap downstream so a
/// value that fits here is not truncated later.
pub const MAX_HOST_REMEDIATION_BYTES: usize = 4096;

/// A bounded, host-authored operator remediation instruction — see the module
/// docs for the provenance invariant this type encodes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct HostRemediation(String);

impl HostRemediation {
    /// Construct a host-authored remediation.
    ///
    /// **Only host code may call this.** See the module docs: the argument must
    /// be a host-authored constant (or built entirely from host-authored
    /// constants), never capability output, a backend error string, or any
    /// other untrusted text.
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_host_remediation(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for HostRemediation {
    type Error = HostApiError;

    /// Wire revalidation matches construction: a persisted/relayed remediation
    /// is re-checked against the current rule on deserialize, never trusted
    /// transparently.
    fn try_from(value: String) -> Result<Self, HostApiError> {
        Self::new(value)
    }
}

impl AsRef<str> for HostRemediation {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HostRemediation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The host-remediation rule. A **value guard, not a vocabulary guard** — see
/// the module docs for why credential words are allowed here and credential
/// values are not.
fn validate_host_remediation(value: &str) -> Result<(), HostApiError> {
    if value.trim().is_empty() {
        return Err(HostApiError::invalid_host_remediation("must not be empty"));
    }
    if value.len() > MAX_HOST_REMEDIATION_BYTES {
        return Err(HostApiError::invalid_host_remediation(format!(
            "must be at most {MAX_HOST_REMEDIATION_BYTES} bytes"
        )));
    }
    // Newlines are the point: a numbered operator checklist is multi-line.
    // Every OTHER control character stays banned — a stray escape byte would
    // invalidate (and thereby drop) the whole model observation downstream.
    if value
        .chars()
        .any(|c| c == '\0' || (c.is_control() && c != '\n'))
    {
        return Err(HostApiError::invalid_host_remediation(
            "must not contain NUL or control characters other than newline",
        ));
    }
    let lower = value.to_ascii_lowercase();
    if crate::credential_redaction::contains_secret_like_token(&lower) {
        return Err(HostApiError::invalid_host_remediation(
            "must not contain credential-shaped tokens",
        ));
    }
    if contains_high_entropy_run(value) {
        return Err(HostApiError::invalid_host_remediation(
            "must not contain long high-entropy tokens",
        ));
    }
    Ok(())
}

/// Minimum length of an alphanumeric run treated as a credential value rather
/// than English. Every word in a host-authored instruction is far shorter; a
/// base64/hex secret is far longer.
const HIGH_ENTROPY_RUN_MIN_LEN: usize = 32;

/// Characters that CONTINUE a candidate credential run rather than breaking it.
///
/// Splitting on every non-alphanumeric character (the original rule) let two
/// real credential shapes fragment below the bound and escape: standard base64
/// bodies, whose `+`/`/`/`=` alphabet chopped a 300-char blob into sub-32
/// pieces, and dot-delimited JWTs. Those four characters therefore stay INSIDE
/// a run.
///
/// `_` and `-` deliberately keep BREAKING runs: they are the separators of the
/// long host-authored identifiers this guard must not false-positive on
/// (`IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI`, kebab-case project
/// slugs). Credential values that use them (`ghp_…`, `xoxb-…`, `sk_live_…`) are
/// caught by the prefix detector in `credential_redaction`, not by this rule.
fn is_run_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '.')
}

/// True when any run of [`HIGH_ENTROPY_RUN_MIN_LEN`]+ [run
/// characters](is_run_character) mixes letters and digits — the shape of a
/// base64/hex/JWT credential value, and a shape no host-authored English
/// instruction produces.
///
/// The letters-AND-digits requirement is what keeps the widened run charset
/// safe: URLs long enough to clear the bound (`https://console.cloud.google.com
/// /apis/credentials`) are pure letters, and pure-digit runs are version or id
/// numbers. Both are deliberately exempt — see
/// `pure_alphabetic_and_pure_numeric_runs_are_exempt`.
fn contains_high_entropy_run(value: &str) -> bool {
    value.split(|c: char| !is_run_character(c)).any(|run| {
        run.len() >= HIGH_ENTROPY_RUN_MIN_LEN
            && run.chars().any(|c| c.is_ascii_digit())
            && run.chars().any(|c| c.is_ascii_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejection(value: impl Into<String>) -> String {
        HostRemediation::new(value).unwrap_err().to_string()
    }

    /// The whole reason this type exists: the four shapes that make
    /// host-authored remediation fail `SafeSummary` must all be ACCEPTED here.
    #[test]
    fn accepts_the_shapes_safe_summary_rejects() {
        let multi_line_with_url_backticks_and_key_names = "Google OAuth setup:\n  \
             1. https://console.cloud.google.com/apis/credentials -> Create Credentials\n  \
             2. Run `ironclaw config set google.client_id <id>.apps.googleusercontent.com`\n  \
             3. Run `ironclaw config set google.client_secret` (prompts, hidden input)";
        let remediation = HostRemediation::new(multi_line_with_url_backticks_and_key_names)
            .expect("host-authored remediation must survive the trusted channel");
        assert_eq!(
            remediation.as_str(),
            multi_line_with_url_backticks_and_key_names
        );
        // And each individual shape, so a regression names which rule broke.
        for accepted in [
            "line one\nline two",
            "see https://console.cloud.google.com/apis/credentials",
            "run `ironclaw config set slack.enabled true`",
            "set google.client_secret and the password for the account",
            "brace { bracket [ angle <",
        ] {
            assert!(
                HostRemediation::new(accepted).is_ok(),
                "should accept {accepted:?}"
            );
        }
    }

    /// The guard is on VALUES, not vocabulary: every credential-token shape is
    /// rejected even though this is the trusted channel.
    #[test]
    fn rejects_credential_value_shapes_even_on_the_trusted_channel() {
        for bad in [
            "run config set google.client_secret sk-ant-abc123def456",
            "the client secret is GOCSPX-abc123def456ghi",
            "use ghp_0123456789abcdefghij as the token",
            "bot token xoxb-1234-5678-abcdefghijklmnop",
            "user token xoxp-1234-5678-abcdefghijklmnop",
            "aws key AKIA0123456789ABCDEF",
            "glpat-abcdefghij1234567890",
            "refresh with ya29.a0AfH6SMBx1234",
            // A long mixed alphanumeric run is a value regardless of prefix.
            "paste this: aG9tZXdvcmsxMjM0NTY3ODkwYWJjZGVmZ2hpams1",
            "hex 0123456789abcdef0123456789abcdef",
        ] {
            let why = rejection(bad);
            assert!(
                why.contains("credential-shaped tokens") || why.contains("high-entropy"),
                "should reject {bad:?} as a credential VALUE, got: {why}"
            );
        }
    }

    /// Long host-authored identifiers are not credential values — the entropy
    /// rule must not false-positive on the real production strings.
    #[test]
    fn does_not_false_positive_on_long_host_identifiers() {
        for ok in [
            "set IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI=<your redirect URL>",
            "ironclaw config set google.client_id <id>.apps.googleusercontent.com",
            "https://console.cloud.google.com/apis/credentials?project=my-project-1234",
            "add `enabled = true` under the [telegram] section of config.toml",
        ] {
            assert!(HostRemediation::new(ok).is_ok(), "should accept {ok:?}");
        }
    }

    /// The two shapes the original run-splitter let through: standard base64
    /// (whose `+`/`/`/`=` alphabet fragmented a long blob into sub-32 pieces)
    /// and a dot-delimited JWT.
    #[test]
    fn rejects_base64_and_jwt_values_that_used_to_fragment_below_the_bound() {
        for bad in [
            // Standard-alphabet base64: every run between `+`/`/` is short.
            "paste this: aG9t+ZXdv/cmsx+MjM0/NTY3+ODkw/YWJj+ZGVm/Z2hp+amsx/MjM0+NTY3",
            // Trailing `=` padding on a body whose pieces are also short.
            "value ab+cd/ef+gh/ij+kl/mn+op/qr+st/uv+wx/yz+01/23+45/67+89==",
            // A JWT: three dot-separated base64url segments.
            "Authorize with eyJhbG.ciOiJIUzI1.NiIsInR5cCI6IkpXVCJ9.abc123def456",
        ] {
            let why = rejection(bad);
            assert!(
                why.contains("high-entropy") || why.contains("credential-shaped tokens"),
                "should reject {bad:?} as a credential VALUE, got: {why}"
            );
        }
    }

    /// The threshold is a bound, not a vibe: 31 run characters pass, 32 fail.
    #[test]
    fn high_entropy_run_bound_is_exactly_thirty_two_characters() {
        let below = format!("token {}1", "a".repeat(HIGH_ENTROPY_RUN_MIN_LEN - 2));
        assert_eq!(below.len() - "token ".len(), HIGH_ENTROPY_RUN_MIN_LEN - 1);
        assert!(
            HostRemediation::new(&below).is_ok(),
            "a {}-character mixed run is below the bound: {below}",
            HIGH_ENTROPY_RUN_MIN_LEN - 1
        );

        let at_bound = format!("token {}1", "a".repeat(HIGH_ENTROPY_RUN_MIN_LEN - 1));
        assert_eq!(at_bound.len() - "token ".len(), HIGH_ENTROPY_RUN_MIN_LEN);
        assert!(
            rejection(&at_bound).contains("high-entropy"),
            "a {HIGH_ENTROPY_RUN_MIN_LEN}-character mixed run is at the bound: {at_bound}"
        );
    }

    /// Deliberate exemptions, pinned so a future tightening is a conscious
    /// choice rather than an accident: a run must mix letters AND digits.
    /// Pure-alphabetic runs are long URLs and prose; pure-numeric runs are ids,
    /// ports, and version numbers. Neither is a credential value shape.
    #[test]
    fn pure_alphabetic_and_pure_numeric_runs_are_exempt() {
        let pure_alphabetic = format!("see {}", "a".repeat(HIGH_ENTROPY_RUN_MIN_LEN * 2));
        assert!(HostRemediation::new(&pure_alphabetic).is_ok());
        let pure_numeric = format!("id {}", "1".repeat(HIGH_ENTROPY_RUN_MIN_LEN * 2));
        assert!(HostRemediation::new(&pure_numeric).is_ok());
        // The widened run charset does not change that: a URL long enough to
        // clear the bound is still pure letters across `.` and `/`.
        assert!(
            HostRemediation::new(
                "https://console.cloud.google.com/apis/credentials/oauthclient/edit"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_empty_overlong_and_control_characters() {
        assert!(rejection("").contains("must not be empty"));
        assert!(rejection("   \n  ").contains("must not be empty"));
        assert!(
            rejection("x".repeat(MAX_HOST_REMEDIATION_BYTES + 1)).contains("at most"),
            "overlong must be rejected for length"
        );
        assert!(HostRemediation::new("x".repeat(MAX_HOST_REMEDIATION_BYTES)).is_ok());
        // Newline allowed, every other control character banned.
        assert!(HostRemediation::new("a\nb").is_ok());
        for bad in ["a\tb", "a\rb", "a\u{0}b", "a\u{1b}[31mb"] {
            assert!(
                rejection(bad).contains("control characters"),
                "should reject {bad:?} for control characters"
            );
        }
    }

    #[test]
    fn serde_revalidates_on_the_wire() {
        let value = HostRemediation::new("run `ironclaw config set slack.enabled true`").unwrap();
        let json = serde_json::to_string(&value).unwrap();
        let back: HostRemediation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value);
        // A hostile wire value is rejected on deserialize, not trusted.
        let err = serde_json::from_str::<HostRemediation>("\"token sk-ant-abc123\"")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("credential-shaped tokens"),
            "wire rejection must carry the validation reason: {err}"
        );
    }
}
