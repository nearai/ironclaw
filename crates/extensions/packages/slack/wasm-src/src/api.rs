//! Slack Web API implementation for the personal (user-token) tool.
//!
//! All API calls go through the host's HTTP capability, which injects the
//! `slack_user_token` secret as a bearer token and scans responses for
//! leaks. The WASM tool never sees the actual token.
//!
//! Every op shapes its output into the standardized messaging framework's
//! canonical envelopes (`crates/contracts/ironclaw_host_api/schemas/messaging/*.json`)
//! and maps every Slack failure onto the closed `messaging.*` error
//! vocabulary — the host validates both post-dispatch, so a shape or code
//! drift here becomes a model-visible tool failure.

use crate::near::agent::host;
use crate::types::*;

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Emit the host runtime's structured guest-error contract
/// (`StructuredWasmGuestError { code, kind }`, parsed in
/// `crates/kernel/ironclaw_host_runtime/src/services/wasm_execution.rs`) so a Slack
/// failure keeps its actionable error code instead of collapsing to a generic
/// "the tool operation failed". `kind` must be one of the host parser's enum
/// values: `auth_required` | `input` | `output_too_large` | `executor` |
/// `network_denied` | `client` | `operation_failed`.
fn structured_error(code: &str, kind: &'static str) -> String {
    // Slack error codes are snake_case ASCII identifiers; reduce to that shape
    // so a hostile response body cannot smuggle free text into the error
    // channel (the host sanitizes again before anything reaches the model).
    let code: String = code
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
        .take(64)
        .collect();
    let code = if code.is_empty() {
        "unknown_error".to_string()
    } else {
        code
    };
    serde_json::json!({ "code": code, "kind": kind }).to_string()
}

/// True for Slack error codes that mean the credential itself is invalid or
/// revoked — never a business-logic denial. These keep riding the existing
/// `AuthRequired` re-auth gate, per the standardized messaging framework spec
/// §8: "credential problems are not taxonomy". `missing_scope` is
/// deliberately excluded: a scope shortfall on an already-declared,
/// already-consented tool is a vendor-side permission question the model can
/// act on (`messaging.permission_denied`), not a signal to blindly restart
/// OAuth (a restart would request the exact same manifest-declared scopes
/// again).
fn slack_error_requires_reauth(code: &str) -> bool {
    matches!(
        code,
        "not_authed" | "invalid_auth" | "account_inactive" | "token_revoked"
    )
}

/// Map a raw Slack `error` code onto the closed standard messaging error
/// taxonomy (`ironclaw_host_api::messaging::StandardMessagingErrorCode`,
/// spec §8). Unmapped codes — including this module's own synthetic
/// non-vendor codes (`http_status_*`, `invalid_json_response`,
/// `missing_ts_in_response`) — fall to `messaging.vendor_error`, the closed
/// vocabulary's catch-all, never a raw or invented code.
fn slack_error_to_standard_code(code: &str) -> &'static str {
    match code {
        "channel_not_found" => "messaging.unknown_conversation",
        // `thread_not_found` is Slack's answer when a ts anchors no thread —
        // for the ops that address one message, that is the message ref not
        // resolving, not a conversation problem.
        "message_not_found" | "thread_not_found" => "messaging.unknown_message",
        "user_not_found" => "messaging.unknown_user",
        "not_in_channel" => "messaging.not_a_member",
        // `cant_delete_message` is Slack's "not yours to delete" — an
        // authorization answer about someone else's message, distinct from
        // `cant_update_message`, which Slack also returns for its own edit
        // window and therefore maps to `edit_not_allowed` below.
        "missing_scope" | "not_allowed_token_type" | "cant_delete_message" => {
            "messaging.permission_denied"
        }
        // The recipient exists and is reachable in principle, but Slack will
        // not open free-form messaging to them.
        "cannot_dm_bot" | "user_disabled" | "user_not_visible" => "messaging.cannot_message_user",
        // `invalid_name` is Slack rejecting an emoji token it cannot render
        // (unknown short name, or a unicode character where a name belongs).
        "invalid_name" | "invalid_name_specials" | "no_item_specified" => {
            "messaging.unsupported_content"
        }
        "msg_too_long" => "messaging.message_too_long",
        // W16 (pre-merge amendment wave, verified defect): `no_text` means
        // Slack rejected the message body's CONTENT (empty after
        // formatting/whitespace stripping) — a length limit was never
        // involved, so this belongs under `unsupported_content`, not
        // `message_too_long`.
        "no_text" => "messaging.unsupported_content",
        "ratelimited" | "rate_limited" => "messaging.rate_limited",
        "cant_update_message" | "edit_window_closed" => "messaging.edit_not_allowed",
        _ => "messaging.vendor_error",
    }
}

/// The WASM structured-error `kind` for a standard messaging error code,
/// chosen from the finite set the host actually parses
/// (`StructuredWasmGuestErrorKind` in
/// `crates/kernel/ironclaw_host_runtime/src/services/wasm_execution.rs:333-343`) —
/// NOT the spec §8 class-column labels verbatim, which do not name real
/// variants of that enum. `input` covers the invalid-input-class codes
/// (immediately model-visible; the model can retry with different
/// arguments). `client` is reserved for `messaging.rate_limited`, matching
/// this module's pre-existing convention (rate limiting is the one
/// genuinely retryable condition — the host retries the same call before
/// bothering the model). Every other code — vendor-side denials
/// (`not_a_member`, `permission_denied`, `edit_not_allowed`) and the
/// `vendor_error` catch-all alike — maps to `operation_failed`: still
/// immediately model-visible (never silently retried against a call that
/// cannot succeed by repetition), which also keeps this module's
/// non-taxonomy synthetic failures (a malformed/missing conversation
/// identity, a send with no `ts`) on the exact kind they used before this
/// task.
fn standard_messaging_error_kind(standard_code: &str) -> &'static str {
    match standard_code {
        "messaging.unknown_conversation"
        | "messaging.unknown_message"
        | "messaging.unknown_user"
        | "messaging.message_too_long"
        | "messaging.unsupported_content" => "input",
        "messaging.rate_limited" => "client",
        _ => "operation_failed",
    }
}

/// Map a raw Slack error code straight to the host's structured guest-error
/// JSON via the standard taxonomy — the shared tail every [`RawSlackFailure`]
/// arm not requiring special handling (auth, `as_user` retry) funnels
/// through.
fn structured_taxonomy_error(slack_code: &str) -> String {
    let standard_code = slack_error_to_standard_code(slack_code);
    structured_error(standard_code, standard_messaging_error_kind(standard_code))
}

/// Percent-encode a string for use as a URL query parameter value.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

/// The pagination cursor of a Slack list-shaped response. Slack signals
/// "no more pages" with an EMPTY `response_metadata.next_cursor`, so an
/// empty cursor becomes `None` here — every paging operation shares this
/// one reading of that contract.
fn next_cursor_from_response(parsed: &serde_json::Value) -> Option<String> {
    parsed["response_metadata"]["next_cursor"]
        .as_str()
        .filter(|cursor| !cursor.is_empty())
        .map(|cursor| cursor.to_string())
}

/// One Slack API failure, before taxonomy mapping. Kept distinct from the
/// final structured JSON error so `send_message` can inspect the RAW vendor
/// code for the classic-vs-granular-app `as_user` retry without
/// string-matching an already-mapped/JSON-encoded error.
#[derive(Debug)]
enum RawSlackFailure {
    /// A host-level transport failure (network denial, timeout, ...). The
    /// string is already one of the host's own literal error identifiers
    /// (`host_http_network_denied` etc., see `wasm_guest_error_kind`'s
    /// literal-string table) — passed straight through unmapped, exactly as
    /// before this task.
    Transport(String),
    /// Non-2xx HTTP status that was not itself a recognized rate limit.
    HttpStatus(u16),
    /// HTTP 429 / Slack-signaled rate limiting.
    RateLimited,
    /// The response body was not valid JSON.
    InvalidJson,
    /// Slack returned `ok: false` with this `error` code.
    Vendor(String),
}

/// Map a [`RawSlackFailure`] to the host's structured guest-error JSON:
/// genuine credential problems keep riding the existing `AuthRequired`
/// re-auth gate; everything else maps onto the closed `messaging.*`
/// vocabulary.
fn structured_error_for(failure: RawSlackFailure) -> String {
    match failure {
        RawSlackFailure::Transport(raw) => raw,
        RawSlackFailure::RateLimited => structured_taxonomy_error("ratelimited"),
        RawSlackFailure::HttpStatus(status) => {
            structured_taxonomy_error(&format!("http_status_{status}"))
        }
        RawSlackFailure::InvalidJson => structured_taxonomy_error("invalid_json_response"),
        RawSlackFailure::Vendor(code) if slack_error_requires_reauth(&code) => {
            structured_error(&code, "auth_required")
        }
        RawSlackFailure::Vendor(code) => structured_taxonomy_error(&code),
    }
}

/// Make a Slack API call and return the parsed JSON value, surfacing HTTP
/// and Slack (`ok: false`) failures as the RAW [`RawSlackFailure`] shape —
/// before taxonomy mapping. Shared by [`slack_api_call`] (which maps every
/// failure to the host's structured guest-error JSON) and `send_message`
/// (which additionally needs the RAW vendor code to decide whether to retry
/// without `as_user`).
fn slack_api_call_raw(
    method: &str,
    endpoint: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, RawSlackFailure> {
    let url = format!("{}/{}", SLACK_API_BASE, endpoint);

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json; charset=utf-8"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    // Log only the static resource name; the query string carries private
    // lookup data (search terms, DM/channel IDs, pagination timestamps).
    let resource = endpoint.split('?').next().unwrap_or(endpoint);
    host::log(
        host::LogLevel::Debug,
        &format!("Slack API: {} {}", method, resource),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)
        .map_err(RawSlackFailure::Transport)?;

    if response.status < 200 || response.status >= 300 {
        // The raw body may carry private data; keep it in debug logs only.
        host::log(
            host::LogLevel::Debug,
            &format!(
                "Slack API {} returned status {}: {}",
                resource,
                response.status,
                String::from_utf8_lossy(&response.body)
            ),
        );
        // Slack signals rate limiting at the HTTP layer (429 + Retry-After);
        // the host's structured error shape carries only {code, kind}, so the
        // Retry-After value cannot ride along.
        if response.status == 429 {
            return Err(RawSlackFailure::RateLimited);
        }
        return Err(RawSlackFailure::HttpStatus(response.status));
    }

    let parsed: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| RawSlackFailure::InvalidJson)?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"]
            .as_str()
            .unwrap_or("unknown_error")
            .to_string();
        host::log(
            host::LogLevel::Debug,
            &format!("Slack API {} error: {}", resource, error),
        );
        return Err(RawSlackFailure::Vendor(error));
    }

    Ok(parsed)
}

/// [`slack_api_call_raw`], mapped to the host's structured guest-error JSON
/// — the call every op except `send_message`'s first attempt uses.
fn slack_api_call(
    method: &str,
    endpoint: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, String> {
    slack_api_call_raw(method, endpoint, body).map_err(structured_error_for)
}

/// Convert Slack's `ts` field (`"<unix_seconds>.<fraction>"`) to an RFC3339
/// UTC timestamp string. Returns `None` when `ts` is not a valid Slack
/// timestamp — the canonical `timestamp` field is optional; a bad ts is left
/// absent, never fabricated.
///
/// W13 (pre-merge amendment wave, verified defect): Slack's fractional part
/// is real sub-second precision (conventionally microseconds, e.g.
/// `"1751970001.000100"`), not a suffix to discard — RFC3339 supports
/// fractional seconds, so it is preserved verbatim rather than truncating to
/// whole seconds. A fractional part that is present but not all-ASCII-digits
/// is dropped (never fabricated); the whole-second timestamp still renders.
fn slack_ts_to_rfc3339(ts: &str) -> Option<String> {
    let mut parts = ts.splitn(2, '.');
    let seconds_part = parts.next()?;
    let fraction_part = parts.next();
    let total_seconds: i64 = seconds_part.parse().ok()?;
    if total_seconds < 0 {
        return None;
    }
    let days = total_seconds.div_euclid(86_400);
    let secs_of_day = total_seconds.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    let whole_seconds = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    match fraction_part.filter(|fraction| {
        !fraction.is_empty() && fraction.chars().all(|character| character.is_ascii_digit())
    }) {
        Some(fraction) => Some(format!("{whole_seconds}.{fraction}Z")),
        None => Some(format!("{whole_seconds}Z")),
    }
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 (proleptic
/// Gregorian) -> (year, month, day). Public-domain algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html>), used here to
/// avoid pulling a date/time crate into a WASM guest module for one field.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// The synchronous WASM tool resolves names with sequential `users.info`
/// round-trips, so one read is allowed at most this many lookups; first-seen
/// ids win the budget and the rest keep raw ids (the same degraded shape as a
/// failed lookup). The single `auth.test` identity call does NOT count
/// against this budget.
const MAX_USER_NAME_LOOKUPS: usize = 25;

/// Resolve the CONNECTED account via `auth.test`: `(user_id, team_id)`.
fn auth_test() -> Result<(String, Option<String>), String> {
    let parsed = slack_api_call("GET", "auth.test", None)?;
    let user_id = parsed["user_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| structured_taxonomy_error("auth_test_missing_user_id"))?
        .to_string();
    let team_id = parsed["team_id"].as_str().map(|s| s.to_string());
    Ok((user_id, team_id))
}

/// Best-effort connected-account lookup for identity marking on reads: a
/// failing `auth.test` (missing scope, outage) must never break the read
/// itself — identity fields are simply absent.
fn current_user_id_best_effort() -> Option<String> {
    match auth_test() {
        Ok((user_id, _team_id)) => Some(user_id),
        Err(error) => {
            host::log(
                host::LogLevel::Debug,
                &format!("auth.test identity lookup skipped: {error}"),
            );
            None
        }
    }
}

/// Mark which messages the CONNECTED account authored. `is_self` is a
/// required, always-concrete bool (never fabricated `true`): when the
/// connected identity is unknown, every message keeps whatever default it
/// was constructed with (`false`).
fn mark_is_self(messages: &mut [Message], current_user_id: Option<&str>) {
    let Some(current_user_id) = current_user_id else {
        return;
    };
    for message in messages.iter_mut() {
        message.is_self = message.author.user_ref == current_user_id;
    }
}

/// Fetch one user's raw `users.info` response. Kept separate from
/// [`get_user_info`] (the canonical public result) because the display-name
/// fallback chain below needs Slack's raw username, which has no canonical
/// field to live in.
fn raw_user_info(user_id: &str) -> Result<serde_json::Value, String> {
    let url = format!("users.info?user={}", url_encode(user_id));
    slack_api_call("GET", &url, None)
}

/// Slack's display-name fallback chain over one USER object: `display_name`
/// -> `real_name` -> the raw username, first non-empty value wins. Shared by
/// `users.info` (which nests the user under a `user` key) and `users.list`
/// (whose `members` entries are bare user objects), so the two can never
/// disagree about what a person is called.
fn display_name_fallback_from_user(user: &serde_json::Value) -> Option<String> {
    let profile = &user["profile"];
    profile["display_name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            profile["real_name"]
                .as_str()
                .filter(|name| !name.is_empty())
        })
        .or_else(|| user["name"].as_str().filter(|name| !name.is_empty()))
        .map(|name| name.to_string())
}

/// [`display_name_fallback_from_user`] against a `users.info` response.
fn display_name_fallback_from_user_info(parsed: &serde_json::Value) -> Option<String> {
    display_name_fallback_from_user(&parsed["user"])
}

/// Resolve Slack user IDs to human-readable names via `users.info`, one
/// lookup per distinct ID in first-seen order, capped at
/// [`MAX_USER_NAME_LOOKUPS`]. Best-effort by contract: a failing lookup
/// (missing scope, deactivated user, outage) is skipped so read operations
/// never fail because a name could not be resolved.
fn resolve_user_display_names(
    user_ids: impl IntoIterator<Item = String>,
) -> std::collections::HashMap<String, String> {
    let mut distinct = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut skipped_over_budget = 0usize;
    for user_id in user_ids {
        if !seen.insert(user_id.clone()) {
            continue;
        }
        if distinct.len() < MAX_USER_NAME_LOOKUPS {
            distinct.push(user_id);
        } else {
            skipped_over_budget += 1;
        }
    }
    if skipped_over_budget > 0 {
        host::log(
            host::LogLevel::Debug,
            &format!(
                "users.info budget reached: {skipped_over_budget} distinct users left unresolved"
            ),
        );
    }
    let mut names = std::collections::HashMap::new();
    for user_id in distinct {
        match raw_user_info(&user_id) {
            Ok(parsed) => {
                if let Some(display) = display_name_fallback_from_user_info(&parsed) {
                    names.insert(user_id, display);
                }
            }
            Err(error) => {
                host::log(
                    host::LogLevel::Debug,
                    &format!("users.info lookup skipped: {error}"),
                );
            }
        }
    }
    names
}

/// Slack user ids are uppercase alphanumeric and start with `U` (or `W` for
/// Enterprise Grid users) — distinct from bot ids (`B…`), which are never
/// eligible for a `users.info` lookup.
fn is_slack_user_id(id: &str) -> bool {
    (id.starts_with('U') || id.starts_with('W'))
        && id.len() > 1
        && id
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

/// Extract the user ids referenced by `<@U…>` / `<@U…|label>` mention tokens
/// inside message text, in order of appearance.
fn mention_user_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('>') else {
            break;
        };
        let id = after[..end].split('|').next().unwrap_or("");
        if is_slack_user_id(id) {
            ids.push(id.to_string());
        }
        rest = &after[end + 1..];
    }
    ids
}

/// Rewrite Slack control tokens in message text for human consumption
/// (inbound entity hygiene): resolved `<@U…>` / `<@U…|label>` mentions become
/// `@Display Name`; when `users.info` is unavailable but the token carries an
/// inline `|label`, that label is rendered (`@label`) — Slack's own text, not a
/// fabrication — so a labeled mention never leaks its raw `U…` id. A bare
/// unresolved `<@U…>` is left as-is (inventing a name would be fabrication).
/// `<#C…|name>` channel refs become `#name`, other tokens (links, `<!here>`)
/// pass through untouched, and Slack's HTML entities (&lt; &gt; &amp;) are
/// decoded AFTER token rewriting so literal `&lt;@U…&gt;` text never turns
/// into a live token.
fn humanize_message_text(text: &str, names: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            // Unterminated token: emit the remainder verbatim.
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let token = &after[..end];
        let original = &rest[start..start + end + 2];
        if let Some(mention) = token.strip_prefix('@') {
            // Slack encodes user mentions as `<@U…>` or `<@U…|label>`, where the
            // label is Slack's own inline rendering of that user's name.
            let (id, inline_label) = match mention.split_once('|') {
                Some((id, label)) => (id, Some(label)),
                None => (mention, None),
            };
            match names.get(id) {
                // Prefer the freshly resolved users.info display name.
                Some(name) if is_slack_user_id(id) => {
                    out.push('@');
                    out.push_str(name);
                }
                // users.info was unavailable (missing scope, over budget,
                // outage) but Slack embedded a label in the token: render
                // Slack's own `@label` rather than leak the raw `<@U…|label>`
                // token (which carries the raw `U…` id). The label is provided
                // by Slack, not fabricated — the same inline-label fallback the
                // `<#C…|name>` channel-ref arm below already uses. A bare
                // unresolved `<@U…>` still stays as-is: inventing a name would
                // be fabrication.
                _ => match inline_label.filter(|label| !label.is_empty()) {
                    Some(label) if is_slack_user_id(id) => {
                        out.push('@');
                        out.push_str(label);
                    }
                    _ => out.push_str(original),
                },
            }
        } else if let Some(channel_ref) = token.strip_prefix('#') {
            match channel_ref
                .split_once('|')
                .map(|(_id, label)| label)
                .filter(|label| !label.is_empty())
            {
                Some(label) => {
                    out.push('#');
                    out.push_str(label);
                }
                None => out.push_str(original),
            }
        } else {
            out.push_str(original);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    // &amp; must decode LAST so a literal "&amp;lt;" ends as "&lt;".
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Map Slack's `is_im`/`is_mpim`/`is_channel` conversation flags onto the
/// standard messaging framework's closed `kind` vocabulary.
fn conversation_kind(is_im: bool, is_mpim: bool, is_channel: bool) -> ConversationKind {
    if is_im {
        ConversationKind::Dm
    } else if is_mpim {
        ConversationKind::GroupDm
    } else if is_channel {
        ConversationKind::Channel
    } else {
        ConversationKind::Other
    }
}

/// Map Slack's conversation object into the canonical `conversation_info`
/// shape. Both list and exact lookup use this mapper so their DM identity
/// semantics cannot drift apart.
fn conversation_info_from_value(value: &serde_json::Value) -> ConversationInfo {
    let is_im = value["is_im"].as_bool().unwrap_or(false);
    let is_mpim = value["is_mpim"].as_bool().unwrap_or(false);
    let is_channel = value["is_channel"].as_bool().unwrap_or(false);
    let counterpart_user = value["user"].as_str().filter(|id| !id.is_empty());
    ConversationInfo {
        conversation: value["id"].as_str().unwrap_or("").to_string(),
        kind: conversation_kind(is_im, is_mpim, is_channel),
        display_name: value["name"].as_str().map(|name| name.to_string()),
        // Absent when Slack omits it (DMs have no membership axis) — never
        // fabricated.
        is_member: value["is_member"].as_bool(),
        counterpart: counterpart_user.map(|user_ref| Author {
            user_ref: user_ref.to_string(),
            display_name: None,
        }),
    }
}

/// Resolve a DM counterpart's display name without making the exact
/// conversation lookup fail when `users.info` is unavailable.
fn enrich_conversation_info_counterpart(info: &mut ConversationInfo) {
    let Some(counterpart) = info.counterpart.as_mut() else {
        return;
    };
    let names = resolve_user_display_names([counterpart.user_ref.clone()]);
    counterpart.display_name = names.get(&counterpart.user_ref).cloned();
}

/// Map Slack's `types` filter onto the requested canonical `kinds`. Omitted
/// (or empty) `kinds` returns every kind Slack has an equivalent for.
/// `other` has no Slack-native conversation type, so requesting only `other`
/// yields an empty (deduped) type list — handled by the caller as "nothing
/// to fetch", never a silent fallback to "everything".
fn slack_types_for_kinds(kinds: Option<&[ConversationKind]>) -> String {
    let requested: Vec<ConversationKind> = match kinds {
        Some(kinds) if !kinds.is_empty() => kinds.to_vec(),
        _ => vec![
            ConversationKind::Dm,
            ConversationKind::GroupDm,
            ConversationKind::Channel,
        ],
    };
    let mut types = Vec::new();
    for kind in requested {
        match kind {
            ConversationKind::Dm => types.push("im"),
            ConversationKind::GroupDm => types.push("mpim"),
            ConversationKind::Channel => {
                types.push("public_channel");
                types.push("private_channel");
            }
            ConversationKind::Other => {}
        }
    }
    types.dedup();
    types.join(",")
}

/// List conversations visible to the user token (channels, DMs, group DMs);
/// `is_member` marks which channels the connected account belongs to.
pub fn list_conversations(
    kinds: Option<&[ConversationKind]>,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<ListConversationsResult, String> {
    let types = slack_types_for_kinds(kinds);
    if types.is_empty() {
        return Ok(ListConversationsResult {
            conversations: Vec::new(),
            next_cursor: None,
        });
    }
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.unwrap_or(200).clamp(1, 999);
    let mut url = format!(
        "conversations.list?types={}&limit={}&exclude_archived=true",
        url_encode(&types),
        limit
    );
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    let mut conversations: Vec<ConversationInfo> = parsed["channels"]
        .as_array()
        .map(|arr| arr.iter().map(conversation_info_from_value).collect())
        .unwrap_or_default();

    // DMs carry no `name`; resolve the counterpart's display name so output
    // is human-readable without a follow-up lookup per conversation.
    let counterpart_ids: Vec<String> = conversations
        .iter()
        .filter_map(|info| info.counterpart.as_ref())
        .map(|counterpart| counterpart.user_ref.clone())
        .collect();
    let names = resolve_user_display_names(counterpart_ids);
    for info in &mut conversations {
        if let Some(counterpart) = info.counterpart.as_mut() {
            counterpart.display_name = names.get(&counterpart.user_ref).cloned();
        }
    }

    let next_cursor = next_cursor_from_response(&parsed);

    Ok(ListConversationsResult {
        conversations,
        next_cursor,
    })
}

/// Retrieve one exact Slack conversation by ID. Unlike
/// `list_conversations`, this response cannot be hidden beyond a model-output
/// preview boundary or confused with another same-name DM.
pub fn get_conversation_info(conversation: &str) -> Result<ConversationInfo, String> {
    let url = format!("conversations.info?channel={}", url_encode(conversation));
    let parsed = slack_api_call("GET", &url, None)?;
    let mut info = conversation_info_from_value(&parsed["channel"]);
    if info.conversation != conversation {
        return Err(structured_taxonomy_error("conversation_identity_mismatch"));
    }
    if matches!(info.kind, ConversationKind::Dm)
        && !matches!(info.counterpart.as_ref(), Some(counterpart) if !counterpart.user_ref.is_empty())
    {
        return Err(structured_taxonomy_error("dm_missing_counterpart"));
    }
    enrich_conversation_info_counterpart(&mut info);
    Ok(info)
}

/// Map one raw Slack message (`conversations.history` / `conversations.replies`)
/// into the canonical [`Message`] shape. Falls back to `bot_id` when Slack
/// omits `user` (bot-authored messages carry no user id) rather than
/// fabricating one; `is_self`/`author.display_name` are filled in by the
/// caller's enrichment pass.
fn history_message_from_value(conversation: &str, value: &serde_json::Value) -> Option<Message> {
    if conversation.is_empty() {
        return None;
    }
    let user_ref = value["user"]
        .as_str()
        .or_else(|| value["bot_id"].as_str())
        .filter(|user_ref| !user_ref.is_empty())?
        .to_string();
    let ts = value["ts"]
        .as_str()
        .filter(|ts| !ts.is_empty())?
        .to_string();
    let thread = value["thread_ts"].as_str().map(|s| s.to_string());
    let reply_count = value["reply_count"].as_u64();
    Some(Message {
        message_ref: MessageRef {
            conversation: conversation.to_string(),
            message_id: ts.clone(),
        },
        author: Author {
            user_ref,
            display_name: None,
        },
        text: value["text"].as_str().unwrap_or("").to_string(),
        timestamp: slack_ts_to_rfc3339(&ts),
        is_self: false,
        thread: thread.map(|thread| ThreadRef {
            thread,
            reply_count,
        }),
        edited: value["edited"].is_object().then_some(true),
    })
}

/// Map a Slack `messages` array (conversations.history / conversations.replies)
/// into canonical [`Message`] entries.
fn history_messages_from_response(conversation: &str, parsed: &serde_json::Value) -> Vec<Message> {
    parsed["messages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| history_message_from_value(conversation, m))
                .collect()
        })
        .unwrap_or_default()
}

/// Shared post-processing for `conversations.history` / `conversations.replies`
/// responses: map messages, resolve authors AND in-text `<@U…>` mentions to
/// display names (one users.info per distinct id, shared budget) so
/// user-facing output never has to echo raw `U…` ids, rewrite Slack control
/// tokens/entities in text, and mark the CONNECTED account's own messages.
/// All enrichments are best-effort: identity/name fields are absent (and
/// tokens stay raw) on failure, and the read still succeeds.
fn enrich_messages(messages: &mut [Message]) {
    // One combined id pool per read: message author first, then the mention
    // tokens inside its text, in reading order — all against the same
    // MAX_USER_NAME_LOOKUPS budget and cache. Bot ids are never eligible for
    // a users.info lookup.
    let mut referenced_ids = Vec::new();
    for message in messages.iter() {
        if is_slack_user_id(&message.author.user_ref) {
            referenced_ids.push(message.author.user_ref.clone());
        }
        referenced_ids.extend(mention_user_ids(&message.text));
    }
    let names = resolve_user_display_names(referenced_ids);
    for message in messages.iter_mut() {
        if let Some(name) = names.get(&message.author.user_ref) {
            message.author.display_name = Some(name.clone());
        }
        message.text = humanize_message_text(&message.text, &names);
    }

    let current_user_id = current_user_id_best_effort();
    mark_is_self(messages, current_user_id.as_deref());
}

fn enriched_history_result(
    conversation: &str,
    parsed: &serde_json::Value,
) -> ConversationHistoryResult {
    let mut messages = history_messages_from_response(conversation, parsed);
    enrich_messages(&mut messages);

    let next_cursor = next_cursor_from_response(parsed);

    ConversationHistoryResult {
        messages,
        next_cursor,
    }
}

/// Read message history from any conversation (channel, DM, or group DM).
pub fn get_conversation_history(
    conversation: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<ConversationHistoryResult, String> {
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.unwrap_or(50).clamp(1, 999);
    let mut url = format!(
        "conversations.history?channel={}&limit={}",
        url_encode(conversation),
        limit
    );
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    Ok(enriched_history_result(conversation, &parsed))
}

/// Read the replies of one thread (`conversations.replies`), with the same
/// enrichment contract as history: resolved display names and
/// connected-account marking.
pub fn get_thread_replies(
    conversation: &str,
    thread: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<ConversationHistoryResult, String> {
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.unwrap_or(50).clamp(1, 999);
    let mut url = format!(
        "conversations.replies?channel={}&ts={}&limit={}",
        url_encode(conversation),
        url_encode(thread),
        limit
    );
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    Ok(enriched_history_result(conversation, &parsed))
}

/// Map one `search.messages` match into the canonical [`Message`] shape.
fn search_match_from_value(value: &serde_json::Value) -> Option<Message> {
    let user_ref = value["user"]
        .as_str()
        .filter(|user_ref| !user_ref.is_empty())?
        .to_string();
    let ts = value["ts"]
        .as_str()
        .filter(|ts| !ts.is_empty())?
        .to_string();
    let conversation = value["channel"]["id"]
        .as_str()
        .filter(|conversation| !conversation.is_empty())?
        .to_string();
    let thread = value["thread_ts"].as_str().map(|s| s.to_string());
    Some(Message {
        message_ref: MessageRef {
            conversation,
            message_id: ts.clone(),
        },
        author: Author {
            user_ref,
            display_name: None,
        },
        text: value["text"].as_str().unwrap_or("").to_string(),
        timestamp: slack_ts_to_rfc3339(&ts),
        is_self: false,
        thread: thread.map(|thread| ThreadRef {
            thread,
            reply_count: None,
        }),
        edited: value["edited"].is_object().then_some(true),
    })
}

/// Search all messages visible to the user token. Slack paginates search
/// results by page number rather than an opaque cursor token; `cursor` here
/// is the standard messaging framework's opaque wrapper around that page
/// number (spec: "search encodes page numbers into next_cursor").
pub fn search_messages(
    query: &str,
    sort: Option<SearchMessagesSort>,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<SearchMessagesResult, String> {
    let count = limit.unwrap_or(20).clamp(1, 100);
    // W14 (pre-merge amendment wave, binding ruling): a SUPPLIED cursor that
    // fails to decode as a Slack page number must surface a model-visible
    // error — never silently restart the search at page 1, which would wrap
    // pagination invisibly and look like a fresh, complete result set. No
    // cursor at all is a legitimate first page, not an error.
    // `unsupported_content` is the closest honest fit in the closed
    // taxonomy: the vendor cannot process this continuation token.
    let page = match cursor {
        None => None,
        Some(raw_cursor) => match raw_cursor.parse::<u32>().ok().filter(|page| *page >= 1) {
            Some(page) => Some(page),
            None => {
                host::log(
                    host::LogLevel::Debug,
                    &format!("search_messages: unrecognized pagination cursor: {raw_cursor}"),
                );
                return Err(structured_error("messaging.unsupported_content", "input"));
            }
        },
    };
    let mut url = format!(
        "search.messages?query={}&count={}",
        url_encode(query),
        count
    );
    match sort {
        Some(SearchMessagesSort::Relevance) => url.push_str("&sort=score"),
        Some(SearchMessagesSort::Timestamp) => {
            url.push_str("&sort=timestamp&sort_dir=desc");
        }
        None => {}
    }
    if let Some(page) = page {
        url.push_str(&format!("&page={page}"));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    let messages_obj = &parsed["messages"];
    let total = messages_obj["total"].as_u64();
    let mut matches: Vec<Message> = messages_obj["matches"]
        .as_array()
        .map(|arr| arr.iter().filter_map(search_match_from_value).collect())
        .unwrap_or_default();
    enrich_messages(&mut matches);

    let current_page = messages_obj["paging"]["page"]
        .as_u64()
        .unwrap_or(u64::from(page.unwrap_or(1)));
    let total_pages = messages_obj["paging"]["pages"].as_u64().unwrap_or(0);
    let next_cursor = (total_pages > current_page).then(|| (current_page + 1).to_string());

    Ok(SearchMessagesResult {
        matches,
        next_cursor,
        total,
    })
}

/// Get information about a user.
pub fn get_user_info(user_ref: &str) -> Result<GetUserInfoResult, String> {
    let parsed = raw_user_info(user_ref)?;

    let user = &parsed["user"];
    let profile = &user["profile"];
    let non_empty = |value: &serde_json::Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string())
    };
    // Slack reports 0 for "no expiration"; only a real timestamp is
    // presence-relevant. No canonical field exists for this, but it is the
    // only machine-readable way to know when a status auto-clears
    // (`status_text` does not encode it), so it rides the schema's `vendor`
    // passthrough instead of being dropped.
    let status_expiration = profile["status_expiration"]
        .as_i64()
        .filter(|expiration| *expiration != 0);

    Ok(GetUserInfoResult {
        user_ref: user["id"].as_str().unwrap_or("").to_string(),
        display_name: non_empty(&profile["display_name"]),
        real_name: non_empty(&profile["real_name"]),
        // No email field: the slack_personal OAuth grant has no
        // users:read.email scope, so Slack never returns one.
        status_text: non_empty(&profile["status_text"]),
        status_emoji: non_empty(&profile["status_emoji"]),
        timezone: non_empty(&user["tz"]),
        title: non_empty(&profile["title"]),
        is_bot: user["is_bot"].as_bool().unwrap_or(false),
        vendor: status_expiration.map(|status_expiration| GetUserInfoVendor { status_expiration }),
    })
}

/// Resolve who the CONNECTED account is: `auth.test` for the user id (the
/// operation itself, so its failure fails the call) plus a best-effort
/// `users.info` for the human-readable name.
pub fn whoami() -> Result<WhoamiResult, String> {
    let (user_id, _team_id) = auth_test()?;
    let display_name = match raw_user_info(&user_id) {
        Ok(parsed) => display_name_fallback_from_user_info(&parsed),
        Err(error) => {
            host::log(
                host::LogLevel::Debug,
                &format!("whoami users.info lookup skipped: {error}"),
            );
            None
        }
    };
    Ok(WhoamiResult {
        user_ref: user_id,
        display_name,
    })
}

fn send_message_payload(
    conversation: &str,
    text: &str,
    thread: Option<&str>,
    as_user: bool,
) -> Result<String, String> {
    let mut payload = serde_json::json!({
        "channel": conversation,
        "text": text,
    });
    if as_user {
        payload["as_user"] = serde_json::Value::Bool(true);
    }
    if let Some(ts) = thread {
        payload["thread_ts"] = serde_json::Value::String(ts.to_string());
    }
    serde_json::to_string(&payload).map_err(|e| e.to_string())
}

/// Send a message as the user.
///
/// Pins `as_user=true`: with a CLASSIC Slack app's user token,
/// `chat.postMessage` defaults to as_user=false and attributes the post to
/// the APP (bot_id/bot_profile) instead of the connected user — the exact
/// wrong-identity failure this tool exists to avoid ("posts as you").
/// Granular (new) apps reject the legacy flag with `as_user_not_supported`;
/// their user-token posts are always authored by the user, so retry exactly
/// once without it.
///
/// A response missing `ts` (successful HTTP/`ok:true`, but no evidence of
/// the sent message) is a vendor anomaly, not a send — this returns
/// `messaging.vendor_error` and never a `message_ref` with an empty
/// `message_id`.
///
/// `thread` (a thread/topic container) and `reply_to` (a quoted message) are
/// distinct in the standard (pre-merge amendment W3/W4), but Slack has only
/// one thread-anchor mechanism (`thread_ts`) — both map onto it. When
/// `thread` is absent, `reply_to`'s `message_id` becomes the effective
/// `thread_ts` (they coincide on Slack). When both are supplied and name
/// different messages, `thread` wins for the vendor call and `reply_to` is
/// ignored for threading purposes — it is still echoed back on the output
/// below, since it was genuinely supplied on input and echoing it is what
/// makes a silent drop checkable; Slack's own thread_ts precedence is a
/// vendor-specific interaction the model does not need surfaced separately.
pub fn send_message(
    conversation: &str,
    text: &str,
    thread: Option<&str>,
    reply_to: Option<&MessageRefInput>,
) -> Result<SendMessageResult, String> {
    let effective_thread_ts =
        thread.or_else(|| reply_to.map(|reply_to| reply_to.message_id.as_str()));
    let payload = send_message_payload(conversation, text, effective_thread_ts, true)?;
    let parsed = match slack_api_call_raw("POST", "chat.postMessage", Some(&payload)) {
        Ok(parsed) => parsed,
        Err(RawSlackFailure::Vendor(code)) if code == "as_user_not_supported" => {
            let payload = send_message_payload(conversation, text, effective_thread_ts, false)?;
            slack_api_call("POST", "chat.postMessage", Some(&payload))?
        }
        Err(failure) => return Err(structured_error_for(failure)),
    };

    let message_id = parsed["ts"].as_str().filter(|ts| !ts.is_empty());
    let Some(message_id) = message_id else {
        return Err(structured_taxonomy_error("missing_ts_in_response"));
    };
    let conversation_out = parsed["channel"]
        .as_str()
        .unwrap_or(conversation)
        .to_string();

    Ok(SendMessageResult {
        message_ref: MessageRef {
            conversation: conversation_out,
            message_id: message_id.to_string(),
        },
        thread: thread.map(|thread| thread.to_string()),
        reply_to: reply_to.map(|reply_to| MessageRef {
            conversation: reply_to.conversation.clone(),
            message_id: reply_to.message_id.clone(),
        }),
    })
}

/// Echo an input `message_ref` back as evidence. Used where Slack confirms an
/// operation with `ok: true` but does not restate the address it acted on.
fn echo_message_ref(message_ref: &MessageRefInput) -> MessageRef {
    MessageRef {
        conversation: message_ref.conversation.clone(),
        message_id: message_ref.message_id.clone(),
    }
}

/// The `{"channel": …, "ts": …}` body every message-addressing write shares.
fn message_ref_payload(message_ref: &MessageRefInput) -> serde_json::Value {
    serde_json::json!({
        "channel": message_ref.conversation,
        "ts": message_ref.message_id,
    })
}

/// Edit one of your own messages (`chat.update`).
///
/// Slack replaces the entire body — there is no partial edit — and rejects an
/// edit of anyone else's message, or one past the workspace's edit window,
/// with `cant_update_message` (`messaging.edit_not_allowed`).
///
/// Like `send_message`, a response missing `ts` is a vendor anomaly rather
/// than an edit: it returns `messaging.vendor_error` and never a
/// `message_ref` with an empty `message_id`.
pub fn edit_message(
    message_ref: &MessageRefInput,
    text: &str,
) -> Result<EditMessageResult, String> {
    let mut payload = message_ref_payload(message_ref);
    payload["text"] = serde_json::Value::String(text.to_string());
    let payload = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    let parsed = slack_api_call("POST", "chat.update", Some(&payload))?;

    let Some(message_id) = parsed["ts"].as_str().filter(|ts| !ts.is_empty()) else {
        return Err(structured_taxonomy_error("missing_ts_in_response"));
    };
    let conversation = parsed["channel"]
        .as_str()
        .filter(|conversation| !conversation.is_empty())
        .unwrap_or(message_ref.conversation.as_str());

    Ok(EditMessageResult {
        message_ref: MessageRef {
            conversation: conversation.to_string(),
            message_id: message_id.to_string(),
        },
    })
}

/// Delete one of your own messages (`chat.delete`).
///
/// `ok: true` IS the evidence here — Slack confirms the removal rather than
/// restating the message — so the returned ref falls back to the input
/// address when the response omits it. `deleted` is always `true`: a delete
/// that did not happen has already returned an error above.
pub fn delete_message(message_ref: &MessageRefInput) -> Result<DeleteMessageResult, String> {
    let payload =
        serde_json::to_string(&message_ref_payload(message_ref)).map_err(|e| e.to_string())?;

    let parsed = slack_api_call("POST", "chat.delete", Some(&payload))?;

    let message_id = parsed["ts"]
        .as_str()
        .filter(|ts| !ts.is_empty())
        .unwrap_or(message_ref.message_id.as_str());
    let conversation = parsed["channel"]
        .as_str()
        .filter(|conversation| !conversation.is_empty())
        .unwrap_or(message_ref.conversation.as_str());

    Ok(DeleteMessageResult {
        deleted: true,
        message_ref: MessageRef {
            conversation: conversation.to_string(),
            message_id: message_id.to_string(),
        },
    })
}

/// Normalize a caller-supplied emoji token to the bare Slack short name.
///
/// Slack's reaction endpoints take `thumbsup`, never `:thumbsup:`, and reject
/// unicode characters outright — so the wrapping colons the model is likely
/// to write are stripped rather than passed through to a guaranteed
/// `invalid_name`. Anything else is left alone for Slack to judge.
fn normalize_emoji(emoji: &str) -> String {
    emoji.trim().trim_matches(':').trim().to_string()
}

/// `reactions.add`/`reactions.remove` share this body shape.
fn reaction_payload(message_ref: &MessageRefInput, name: &str) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({
        "channel": message_ref.conversation,
        "timestamp": message_ref.message_id,
        "name": name,
    }))
    .map_err(|e| e.to_string())
}

/// Slack's "you already reacted with this" code — `reactions.add`'s
/// end-state-already-holds answer.
const ALREADY_REACTED: &str = "already_reacted";
/// Slack's "that reaction is not there" code — `reactions.remove`'s.
const NO_REACTION: &str = "no_reaction";

/// Collapse a reaction write's outcome, treating ONE vendor code as success:
/// the code that means the requested end state already holds.
///
/// Slack returns `already_reacted`/`no_reaction` only for a message it
/// resolved and a reaction it can see, so the caller's intent is satisfied
/// either way. Reporting that as an error would push the model into retrying
/// a call that cannot change anything. Every other failure still maps through
/// the closed taxonomy — including the OTHER family's idempotent code, which
/// is a genuine vendor error on the wrong endpoint.
fn reaction_write_outcome(
    result: Result<serde_json::Value, RawSlackFailure>,
    end_state_code: &str,
) -> Result<(), String> {
    match result {
        Ok(_) => Ok(()),
        Err(RawSlackFailure::Vendor(code)) if code == end_state_code => Ok(()),
        Err(failure) => Err(structured_error_for(failure)),
    }
}

/// Add an emoji reaction to a message, as the connected account.
pub fn add_reaction(
    message_ref: &MessageRefInput,
    emoji: &str,
) -> Result<AddReactionResult, String> {
    let name = normalize_emoji(emoji);
    if name.is_empty() {
        return Err(structured_error("messaging.unsupported_content", "input"));
    }
    let payload = reaction_payload(message_ref, &name)?;

    reaction_write_outcome(
        slack_api_call_raw("POST", "reactions.add", Some(&payload)),
        ALREADY_REACTED,
    )?;

    Ok(AddReactionResult {
        message_ref: echo_message_ref(message_ref),
        emoji: name,
    })
}

/// Remove one named reaction.
fn remove_one_reaction(message_ref: &MessageRefInput, name: &str) -> Result<(), String> {
    let payload = reaction_payload(message_ref, name)?;
    reaction_write_outcome(
        slack_api_call_raw("POST", "reactions.remove", Some(&payload)),
        NO_REACTION,
    )
}

/// The emoji names the CONNECTED account has reacted with on one message
/// (`reactions.get`, `reactions:read`).
///
/// The identity is load-bearing rather than decorative here — without it
/// there is no way to tell the connected account's reactions from anyone
/// else's — so unlike the best-effort identity marking on reads, an
/// `auth.test` failure fails the call instead of degrading it. Removing
/// someone else's reaction is the failure this guards against.
fn own_reaction_names(message_ref: &MessageRefInput) -> Result<Vec<String>, String> {
    let (current_user_id, _team_id) = auth_test()?;
    // `full=true` is load-bearing: without it Slack truncates each
    // reaction's `users` array on popular messages, and a truncated list
    // that omits the connected account would make the ownership filter
    // below silently skip a reaction that IS ours — a false "all removed"
    // success.
    let url = format!(
        "reactions.get?channel={}&timestamp={}&full=true",
        url_encode(&message_ref.conversation),
        url_encode(&message_ref.message_id)
    );
    let parsed = slack_api_call("GET", &url, None)?;
    Ok(own_reaction_names_from_response(&parsed, &current_user_id))
}

/// The decision half of [`own_reaction_names`], over an already-parsed
/// `reactions.get` response: which emoji names did THIS user react with.
///
/// Split out from the request so it can be tested — an inverted or absent
/// filter here removes other people's reactions, which is the one outcome
/// this operation must never produce, and no schema or taxonomy check
/// downstream would notice.
fn own_reaction_names_from_response(
    parsed: &serde_json::Value,
    current_user_id: &str,
) -> Vec<String> {
    let Some(reactions) = parsed["message"]["reactions"].as_array() else {
        return Vec::new();
    };
    reactions
        .iter()
        .filter(|reaction| {
            // A reaction with no `users` array tells us nothing about
            // authorship, so it is NOT ours — absence of evidence is not
            // evidence of ownership.
            reaction["users"].as_array().is_some_and(|users| {
                users
                    .iter()
                    .any(|user| user.as_str() == Some(current_user_id))
            })
        })
        .filter_map(|reaction| {
            reaction["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string())
        })
        .collect()
}

/// Remove a reaction from a message, as the connected account.
///
/// With `emoji`, one named reaction is removed and echoed back. Without it,
/// the canonical contract asks for "the connected account's own reaction(s)"
/// — Slack's endpoint cannot express that, so this reads the message's
/// reactions first and removes each one the connected account added. That
/// variant names no single emoji on the way out, which is exactly the case
/// the canonical output schema leaves `emoji` optional for.
pub fn remove_reaction(
    message_ref: &MessageRefInput,
    emoji: Option<&str>,
) -> Result<RemoveReactionResult, String> {
    match emoji {
        Some(emoji) => {
            let name = normalize_emoji(emoji);
            if name.is_empty() {
                return Err(structured_error("messaging.unsupported_content", "input"));
            }
            remove_one_reaction(message_ref, &name)?;
            Ok(RemoveReactionResult {
                message_ref: echo_message_ref(message_ref),
                emoji: Some(name),
            })
        }
        None => {
            for name in own_reaction_names(message_ref)? {
                remove_one_reaction(message_ref, &name)?;
            }
            Ok(RemoveReactionResult {
                message_ref: echo_message_ref(message_ref),
                emoji: None,
            })
        }
    }
}

/// Open, or fetch the already-open, DM with one person
/// (`conversations.open`, `im:write`).
///
/// Slack returns the existing DM when one is already open, so this is safe to
/// call repeatedly and never creates a duplicate conversation.
pub fn open_dm(user_ref: &str) -> Result<OpenDmResult, String> {
    // Slack's `users` field takes a comma-separated LIST: two ids would
    // silently open a multi-person group DM while every contract layer
    // promises the 1:1 DM with one person. Requiring a single well-formed
    // user id here (commas and lowercase both fail the check) keeps that
    // promise instead of delegating it to the model's good behavior.
    if !is_slack_user_id(user_ref) {
        return Err(structured_error("messaging.unknown_user", "input"));
    }
    let payload = serde_json::to_string(&serde_json::json!({ "users": user_ref }))
        .map_err(|e| e.to_string())?;

    let parsed = slack_api_call("POST", "conversations.open", Some(&payload))?;

    let conversation = parsed["channel"]["id"]
        .as_str()
        .filter(|conversation| !conversation.is_empty())
        .ok_or_else(|| structured_taxonomy_error("open_dm_missing_conversation"))?;

    Ok(OpenDmResult {
        conversation: conversation.to_string(),
    })
}

/// Find the message whose `ts` matches EXACTLY inside a history/replies
/// response. Exactness is the whole point: `conversations.history`'s `latest`
/// is a range bound, so a ts that no longer exists would otherwise hand back
/// whichever message sits just before it as though it were the one asked for.
fn exact_message(
    conversation: &str,
    target_ts: &str,
    parsed: &serde_json::Value,
) -> Option<Message> {
    parsed["messages"]
        .as_array()?
        .iter()
        .find(|value| value["ts"].as_str() == Some(target_ts))
        .and_then(|value| history_message_from_value(conversation, value))
}

/// Fetch one message by reference.
///
/// Slack has no single-message endpoint. `conversations.history` addresses the
/// conversation timeline, but a threaded REPLY is not on that timeline, so a
/// miss falls back to `conversations.replies`, which returns the whole thread
/// a ts belongs to. Only an exact ts match is ever returned; anything else is
/// `messaging.unknown_message` rather than a neighbouring message.
pub fn get_message(message_ref: &MessageRefInput) -> Result<GetMessageResult, String> {
    let conversation = message_ref.conversation.as_str();
    let target_ts = message_ref.message_id.as_str();

    let history_url = format!(
        "conversations.history?channel={}&latest={}&inclusive=true&limit=1",
        url_encode(conversation),
        url_encode(target_ts)
    );
    let parsed = slack_api_call("GET", &history_url, None)?;
    let mut found = exact_message(conversation, target_ts, &parsed);

    if found.is_none() {
        // Slack rejects a ts that anchors no thread with `thread_not_found`;
        // that is this ref not resolving, which the taxonomy already calls
        // `unknown_message` — fall through to the shared miss below rather
        // than surfacing it as a distinct failure.
        //
        // `oldest=latest=ts&inclusive=true` pinches the range to exactly the
        // target timestamp, so the page holds the target (plus at most the
        // quirk where Slack prepends the thread parent to a first page) no
        // matter how deep in the thread it sits and regardless of the
        // endpoint's sort order — where a bare `limit=N` page starts at one
        // end of the thread and misses anything past N. `limit=5` is quirk
        // headroom, not a scan.
        let replies_url = format!(
            "conversations.replies?channel={}&ts={}&oldest={}&latest={}&inclusive=true&limit=5",
            url_encode(conversation),
            url_encode(target_ts),
            url_encode(target_ts),
            url_encode(target_ts)
        );
        match slack_api_call_raw("GET", &replies_url, None) {
            Ok(parsed) => found = exact_message(conversation, target_ts, &parsed),
            Err(RawSlackFailure::Vendor(code)) if code == "thread_not_found" => {}
            Err(failure) => return Err(structured_error_for(failure)),
        }
    }

    let Some(message) = found else {
        return Err(structured_error("messaging.unknown_message", "input"));
    };

    let mut messages = vec![message];
    enrich_messages(&mut messages);
    let Some(message) = messages.pop() else {
        return Err(structured_taxonomy_error(
            "message_enrichment_dropped_message",
        ));
    };

    Ok(GetMessageResult { message })
}

/// The largest `users.list` page one `resolve_user` call scans, and the
/// default when the caller names no `limit` (Slack's recommended page size).
///
/// The requested page size and the match cap are deliberately the SAME
/// number: Slack cursors resume at page granularity only, so a call that
/// stopped collecting matches mid-page would return a `next_cursor` that
/// skips the rest of that page — matches the caller can never reach. Scanning
/// exactly `limit` entries per call keeps every cursor loss-free.
const RESOLVE_USER_SCAN_PAGE: u32 = 200;

/// One `users.list` member as a canonical person, when it is an eligible
/// human AND `needle` (already lowercased) matches its display name, real
/// name, or handle.
///
/// The matching half of [`resolve_user`], split from the request so the
/// eligibility rules and the case-insensitive comparison are testable — this
/// is the only thing standing between the model and a directory scan that
/// returns deactivated accounts or bots as people to message.
fn matching_person_from_member(member: &serde_json::Value, needle: &str) -> Option<Author> {
    if member["deleted"].as_bool().unwrap_or(false) || member["is_bot"].as_bool().unwrap_or(false) {
        return None;
    }
    let user_ref = member["id"].as_str().filter(|id| !id.is_empty())?;
    let display_name = display_name_fallback_from_user(member);
    let matched = [
        display_name.as_deref(),
        member["profile"]["real_name"].as_str(),
        member["name"].as_str(),
    ]
    .into_iter()
    .flatten()
    .any(|candidate| candidate.to_lowercase().contains(needle));

    matched.then(|| Author {
        user_ref: user_ref.to_string(),
        display_name,
    })
}

/// The member ids in a `conversations.members` response, dropping empties so
/// a blank entry never becomes a `user_ref` the canonical schema rejects for
/// `minLength: 1`.
fn member_ids_from_response(parsed: &serde_json::Value) -> Vec<String> {
    parsed["members"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|member| member.as_str())
                .filter(|member| !member.is_empty())
                .map(|member| member.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Search the workspace directory for people.
///
/// Slack has no server-side people search for user tokens
/// (`users.lookupByEmail` needs `users:read.email`, which this extension does
/// not request), so one call scans a single `users.list` page and filters it
/// locally against display name, real name, and handle. `next_cursor` walks
/// the directory onward, so a caller that has not found their person yet
/// keeps paging. Deactivated accounts and bots are skipped.
///
/// The scanned page IS `limit` entries long (see [`RESOLVE_USER_SCAN_PAGE`]):
/// every entry fetched is scanned before the cursor is reported, so
/// `next_cursor` never points past a match this call withheld.
pub fn resolve_user(
    query: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<ResolveUserResult, String> {
    let limit = limit
        .unwrap_or(RESOLVE_USER_SCAN_PAGE)
        .clamp(1, RESOLVE_USER_SCAN_PAGE);
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Err(structured_error("messaging.unsupported_content", "input"));
    }

    let mut url = format!("users.list?limit={limit}");
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }
    let parsed = slack_api_call("GET", &url, None)?;

    let mut matches = Vec::new();
    for member in parsed["members"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        // With page == limit this cap can only trigger if Slack overshoots
        // the requested page size; it preserves the "at most `limit`
        // matches" contract without ever stopping mid-scan on a well-formed
        // page.
        if matches.len() >= limit as usize {
            break;
        }
        if let Some(person) = matching_person_from_member(member, &needle) {
            matches.push(person);
        }
    }

    let next_cursor = next_cursor_from_response(&parsed);

    Ok(ResolveUserResult {
        matches,
        next_cursor,
    })
}

/// List the members of a channel, group DM, or DM
/// (`conversations.members`).
///
/// Display names are resolved best-effort under the shared
/// [`MAX_USER_NAME_LOOKUPS`] budget: members past it keep their raw id rather
/// than failing the read, the same degraded shape every other read uses.
pub fn list_members(
    conversation: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> Result<ListMembersResult, String> {
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.unwrap_or(100).clamp(1, 999);
    let mut url = format!(
        "conversations.members?channel={}&limit={}",
        url_encode(conversation),
        limit
    );
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    let member_ids = member_ids_from_response(&parsed);

    let names = resolve_user_display_names(
        member_ids
            .iter()
            .filter(|member| is_slack_user_id(member))
            .cloned(),
    );
    let members = member_ids
        .into_iter()
        .map(|user_ref| Author {
            display_name: names.get(&user_ref).cloned(),
            user_ref,
        })
        .collect();

    let next_cursor = next_cursor_from_response(&parsed);

    Ok(ListMembersResult {
        members,
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only pure mapping helpers are exercised here: everything else in this
    /// module reaches the host HTTP import, which does not exist on the test
    /// target.
    #[test]
    fn emoji_tokens_lose_their_wrapping_colons() {
        // What the model is most likely to write, and what Slack requires.
        assert_eq!(normalize_emoji(":thumbsup:"), "thumbsup");
        assert_eq!(normalize_emoji("thumbsup"), "thumbsup");
        assert_eq!(normalize_emoji("  :tada:  "), "tada");
        // Slack's own skin-tone and compound names survive untouched.
        assert_eq!(normalize_emoji(":+1::skin-tone-2:"), "+1::skin-tone-2");
        // Degenerate input normalizes to empty, which both reaction ops turn
        // into `messaging.unsupported_content` rather than calling Slack.
        assert_eq!(normalize_emoji(":::"), "");
        assert_eq!(normalize_emoji("   "), "");
    }

    /// A message-addressing write sends Slack's own field names (`channel`,
    /// `ts`), never the canonical ones — the canonical vocabulary stops at
    /// this module's boundary.
    #[test]
    fn message_ref_payload_uses_slack_field_names() {
        let payload = message_ref_payload(&MessageRefInput {
            conversation: "C1".to_string(),
            message_id: "1751970001.000100".to_string(),
        });
        assert_eq!(
            payload,
            serde_json::json!({ "channel": "C1", "ts": "1751970001.000100" })
        );
    }

    /// Reactions address the message with `timestamp`, NOT `ts` — Slack's one
    /// inconsistency between the chat.* and reactions.* families, and a silent
    /// `invalid_arguments` if it drifts.
    #[test]
    fn reaction_payload_addresses_the_message_with_timestamp() {
        let payload = reaction_payload(
            &MessageRefInput {
                conversation: "C1".to_string(),
                message_id: "1751970001.000100".to_string(),
            },
            "thumbsup",
        )
        .expect("payload serializes");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).expect("payload parses"),
            serde_json::json!({
                "channel": "C1",
                "timestamp": "1751970001.000100",
                "name": "thumbsup"
            })
        );
    }

    /// The vendor->canonical error map is the contract the model sees. These
    /// pin the codes the new operations introduce, including the two that
    /// deliberately do NOT collapse together: Slack returns
    /// `cant_update_message` for its own edit window (edit_not_allowed) and
    /// `cant_delete_message` for someone else's message (permission_denied).
    #[test]
    fn new_vendor_errors_map_onto_the_closed_taxonomy() {
        for (slack_code, expected) in [
            ("message_not_found", "messaging.unknown_message"),
            ("thread_not_found", "messaging.unknown_message"),
            ("cant_delete_message", "messaging.permission_denied"),
            ("cant_update_message", "messaging.edit_not_allowed"),
            ("cannot_dm_bot", "messaging.cannot_message_user"),
            ("user_disabled", "messaging.cannot_message_user"),
            ("invalid_name", "messaging.unsupported_content"),
            ("user_not_found", "messaging.unknown_user"),
            // Unmapped vendor codes still fall to the catch-all, never a raw
            // or invented code.
            ("some_new_slack_error", "messaging.vendor_error"),
        ] {
            assert_eq!(
                slack_error_to_standard_code(slack_code),
                expected,
                "{slack_code}"
            );
        }
    }

    /// An unresolvable message ref must reach the model as retryable INPUT,
    /// not as a terminal operation failure.
    #[test]
    fn unknown_message_is_an_input_class_failure() {
        assert_eq!(
            standard_messaging_error_kind("messaging.unknown_message"),
            "input"
        );
        assert_eq!(
            standard_messaging_error_kind("messaging.cannot_message_user"),
            "operation_failed"
        );
    }

    /// Credential problems are not taxonomy: they keep riding the
    /// `AuthRequired` re-auth gate. A scope shortfall is deliberately NOT one
    /// of them — re-running OAuth would request the same manifest scopes.
    /// This is the behavior an account connected before the scope widening
    /// hits on the three new write tools.
    #[test]
    fn a_missing_scope_is_a_denial_not_a_reauth() {
        assert!(!slack_error_requires_reauth("missing_scope"));
        assert_eq!(
            slack_error_to_standard_code("missing_scope"),
            "messaging.permission_denied"
        );
        assert!(slack_error_requires_reauth("token_revoked"));
    }

    /// The omit-emoji `remove_reaction` variant removes only what the
    /// CONNECTED account reacted with. Removing someone else's reaction is
    /// the one outcome the operation must never produce, and nothing
    /// downstream would catch it — the output schema is satisfied either way.
    #[test]
    fn own_reactions_are_filtered_to_the_connected_account() {
        let response = serde_json::json!({
            "message": {
                "reactions": [
                    { "name": "thumbsup", "users": ["U_ME", "U_OTHER"] },
                    { "name": "tada",     "users": ["U_OTHER"] },
                    { "name": "eyes",     "users": ["U_ME"] }
                ]
            }
        });
        assert_eq!(
            own_reaction_names_from_response(&response, "U_ME"),
            vec!["thumbsup".to_string(), "eyes".to_string()],
            "only reactions the connected account joined"
        );
        // A user who reacted with nothing removes nothing.
        assert!(own_reaction_names_from_response(&response, "U_NOBODY").is_empty());
    }

    #[test]
    fn reactions_without_evidence_of_authorship_are_never_ours() {
        // No `users` array, an empty one, and a malformed entry: none of
        // these say the connected account reacted, so none may be removed.
        let response = serde_json::json!({
            "message": {
                "reactions": [
                    { "name": "thumbsup" },
                    { "name": "tada", "users": [] },
                    { "name": "", "users": ["U_ME"] },
                    { "name": "eyes", "users": ["U_ME"] }
                ]
            }
        });
        assert_eq!(
            own_reaction_names_from_response(&response, "U_ME"),
            vec!["eyes".to_string()]
        );

        // A message with no reactions at all, and a response missing the
        // message entirely, both mean "nothing to remove" — not an error and
        // not everything.
        assert!(
            own_reaction_names_from_response(&serde_json::json!({ "message": {} }), "U_ME")
                .is_empty()
        );
        assert!(own_reaction_names_from_response(&serde_json::json!({}), "U_ME").is_empty());
    }

    /// `already_reacted`/`no_reaction` mean the requested end state holds, so
    /// they are successes — but each belongs to ONE endpoint. The
    /// cross-pairing assertions matter: they prove the codes are not a
    /// blanket "reaction errors are fine" rule.
    #[test]
    fn reaction_writes_accept_only_their_own_end_state_code() {
        let vendor = |code: &str| Err(RawSlackFailure::Vendor(code.to_string()));

        assert!(reaction_write_outcome(Ok(serde_json::json!({})), ALREADY_REACTED).is_ok());
        assert!(reaction_write_outcome(vendor(ALREADY_REACTED), ALREADY_REACTED).is_ok());
        assert!(reaction_write_outcome(vendor(NO_REACTION), NO_REACTION).is_ok());

        // The other family's code is a real error, not a free pass.
        assert!(reaction_write_outcome(vendor(NO_REACTION), ALREADY_REACTED).is_err());
        assert!(reaction_write_outcome(vendor(ALREADY_REACTED), NO_REACTION).is_err());

        // Everything else still maps through the closed taxonomy.
        let error = reaction_write_outcome(vendor("message_not_found"), ALREADY_REACTED)
            .expect_err("an unresolvable message is a failure");
        assert!(
            error.contains("messaging.unknown_message"),
            "expected the canonical code, got {error}"
        );
    }

    /// `conversations.history`'s `latest` is a RANGE bound, so a timestamp
    /// that no longer exists would otherwise hand back whichever message sits
    /// just before it as though it were the one asked for.
    #[test]
    fn exact_message_requires_an_exact_timestamp_match() {
        let response = serde_json::json!({
            "messages": [
                { "ts": "1751970000.000000", "user": "U1", "text": "the one before" }
            ]
        });
        assert!(
            exact_message("C1", "1751970001.000100", &response).is_none(),
            "a near miss must not be returned as the requested message"
        );

        let response = serde_json::json!({
            "messages": [
                { "ts": "1751970000.000000", "user": "U1", "text": "the one before" },
                { "ts": "1751970001.000100", "user": "U2", "text": "the one asked for" }
            ]
        });
        let found = exact_message("C1", "1751970001.000100", &response)
            .expect("the exact timestamp resolves");
        assert_eq!(found.text, "the one asked for");
        assert_eq!(found.message_ref.message_id, "1751970001.000100");
        assert_eq!(found.message_ref.conversation, "C1");

        assert!(exact_message("C1", "1751970001.000100", &serde_json::json!({})).is_none());
    }

    #[test]
    fn directory_matching_is_case_insensitive_across_name_handle_and_real_name() {
        let member = |extra: serde_json::Value| {
            let mut base = serde_json::json!({
                "id": "U1",
                "name": "ahandle",
                "profile": { "display_name": "Alice Example", "real_name": "Alice Q. Example" }
            });
            for (key, val) in extra.as_object().expect("object") {
                base[key] = val.clone();
            }
            base
        };
        let none = serde_json::json!({});

        for needle in [
            "alice",
            "ALICE".to_lowercase().as_str(),
            "q. example",
            "ahandle",
        ] {
            assert!(
                matching_person_from_member(&member(none.clone()), needle).is_some(),
                "{needle} should match"
            );
        }
        assert!(matching_person_from_member(&member(none.clone()), "zzz").is_none());

        // The match carries the resolved display name through.
        let matched = matching_person_from_member(&member(none.clone()), "alice").expect("match");
        assert_eq!(matched.user_ref, "U1");
        assert_eq!(matched.display_name.as_deref(), Some("Alice Example"));
    }

    #[test]
    fn directory_matching_skips_bots_deactivated_accounts_and_id_less_entries() {
        let base = serde_json::json!({
            "id": "U1",
            "name": "alice",
            "profile": { "display_name": "Alice" }
        });

        let mut bot = base.clone();
        bot["is_bot"] = serde_json::json!(true);
        assert!(matching_person_from_member(&bot, "alice").is_none());

        let mut deleted = base.clone();
        deleted["deleted"] = serde_json::json!(true);
        assert!(matching_person_from_member(&deleted, "alice").is_none());

        let mut id_less = base.clone();
        id_less["id"] = serde_json::json!("");
        assert!(matching_person_from_member(&id_less, "alice").is_none());

        assert!(matching_person_from_member(&base, "alice").is_some());
    }

    #[test]
    fn conversation_member_ids_drop_blanks_and_survive_an_absent_list() {
        assert_eq!(
            member_ids_from_response(&serde_json::json!({ "members": ["U1", "", "U2"] })),
            vec!["U1".to_string(), "U2".to_string()]
        );
        assert!(member_ids_from_response(&serde_json::json!({})).is_empty());
    }
}
