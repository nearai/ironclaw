//! X (Twitter) bookmarks pipeline.
//!
//! IronClaw owns the full pipeline: it ingests scraped bookmarks via the
//! gateway, persists them to the database, runs an LLM triage step that
//! classifies each bookmark (`build`/`read`/`reference`/`dead`), and exposes
//! the resulting queue. Scraping itself happens upstream — typically a
//! claude-in-chrome session — and is out of scope for this crate.
//!
//! The triage LLM is configurable per-skill via `[skills.x_bookmarks]
//! triage_model = ...` (or the `X_BOOKMARKS_TRIAGE_MODEL` env var). When the
//! override is unset, the global IronClaw LLM provider's default model is
//! used. The skill ships with the override unset so the default behavior is
//! "use whatever IronClaw is configured with".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod triage;

/// A canonical bookmark record as persisted in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: uuid::Uuid,
    pub user_id: String,
    pub tweet_id: String,
    pub author_handle: Option<String>,
    pub author_name: Option<String>,
    pub text: String,
    pub url: Option<String>,
    /// JSON-encoded array of media URLs (preserved verbatim from the scraper).
    pub media_urls: Vec<String>,
    pub quoted_tweet: Option<String>,
    pub thread_id: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    pub scraped_at: DateTime<Utc>,
    pub status: BookmarkStatus,
    pub rationale: Option<String>,
    pub project_slug: Option<String>,
    pub tags: Vec<String>,
    pub triaged_at: Option<DateTime<Utc>>,
    pub triage_model: Option<String>,
}

/// One bookmark status. The default ("untriaged") is the post-ingest state
/// before the triage LLM runs. The other four are the canonical triage
/// outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookmarkStatus {
    Untriaged,
    Build,
    Read,
    Reference,
    Dead,
}

impl BookmarkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BookmarkStatus::Untriaged => "untriaged",
            BookmarkStatus::Build => "build",
            BookmarkStatus::Read => "read",
            BookmarkStatus::Reference => "reference",
            BookmarkStatus::Dead => "dead",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "untriaged" => Some(BookmarkStatus::Untriaged),
            "build" => Some(BookmarkStatus::Build),
            "read" => Some(BookmarkStatus::Read),
            "reference" => Some(BookmarkStatus::Reference),
            "dead" => Some(BookmarkStatus::Dead),
            _ => None,
        }
    }
}

/// Validated payload accepted by the ingest endpoint.
///
/// All fields are optional except `tweet_id`, `text`, and `url`. The scraper
/// is the producer; the gateway only enforces "is this minimally usable?"
/// not "is every field present?".
#[derive(Debug, Clone, Deserialize)]
pub struct BookmarkIngestItem {
    pub tweet_id: String,
    #[serde(default)]
    pub author_handle: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    pub url: String,
    #[serde(default)]
    pub media_urls: Vec<String>,
    #[serde(default)]
    pub quoted_tweet: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub posted_at: Option<DateTime<Utc>>,
}

/// Per-bookmark validation: enforces non-empty `tweet_id`, a hostname-checked
/// `url` from a known X/Twitter domain, and a soft length cap on free-form
/// fields to prevent ingest of multi-megabyte payloads through one tweet.
///
/// Returns the trimmed/normalized form on success.
pub fn validate_ingest_item(raw: &BookmarkIngestItem) -> Result<NormalizedIngestItem, IngestError> {
    let tweet_id = raw.tweet_id.trim();
    if tweet_id.is_empty() {
        return Err(IngestError::InvalidField(
            "tweet_id".to_string(),
            "must be non-empty".to_string(),
        ));
    }
    if tweet_id.len() > 64
        || !tweet_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(IngestError::InvalidField(
            "tweet_id".to_string(),
            "must be alphanumeric, <= 64 chars".to_string(),
        ));
    }

    let url = raw.url.trim();
    if url.is_empty() {
        return Err(IngestError::InvalidField(
            "url".to_string(),
            "must be non-empty".to_string(),
        ));
    }
    if url.len() > 2048 {
        return Err(IngestError::InvalidField(
            "url".to_string(),
            "must be <= 2048 chars".to_string(),
        ));
    }
    let parsed = url::Url::parse(url).map_err(|_| {
        IngestError::InvalidField("url".to_string(), "is not a valid URL".to_string())
    })?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(IngestError::InvalidField(
            "url".to_string(),
            "scheme must be http(s)".to_string(),
        ));
    }
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if !is_allowed_host(&host) {
        return Err(IngestError::InvalidField(
            "url".to_string(),
            format!("host {host:?} is not an allowed X/Twitter domain"),
        ));
    }

    // Cap free-form text fields. The triage prompt embeds these, so a
    // multi-megabyte tweet body would blow out the LLM context window and
    // could double as a soft prompt-injection vector.
    let text = raw.text.clone().unwrap_or_default();
    if text.len() > MAX_TEXT_LEN {
        return Err(IngestError::InvalidField(
            "text".to_string(),
            format!("must be <= {MAX_TEXT_LEN} chars"),
        ));
    }
    let quoted = raw.quoted_tweet.clone().unwrap_or_default();
    if quoted.len() > MAX_TEXT_LEN {
        return Err(IngestError::InvalidField(
            "quoted_tweet".to_string(),
            format!("must be <= {MAX_TEXT_LEN} chars"),
        ));
    }

    if raw.media_urls.len() > MAX_MEDIA_URLS {
        return Err(IngestError::InvalidField(
            "media_urls".to_string(),
            format!("must contain <= {MAX_MEDIA_URLS} entries"),
        ));
    }
    for media_url in &raw.media_urls {
        if media_url.len() > 2048 {
            return Err(IngestError::InvalidField(
                "media_urls".to_string(),
                "each entry must be <= 2048 chars".to_string(),
            ));
        }
    }

    Ok(NormalizedIngestItem {
        tweet_id: tweet_id.to_string(),
        author_handle: trim_opt(&raw.author_handle, MAX_HANDLE_LEN),
        author_name: trim_opt(&raw.author_name, MAX_NAME_LEN),
        text,
        url: url.to_string(),
        media_urls: raw.media_urls.clone(),
        quoted_tweet: if quoted.is_empty() {
            None
        } else {
            Some(quoted)
        },
        thread_id: trim_opt(&raw.thread_id, MAX_HANDLE_LEN),
        posted_at: raw.posted_at,
    })
}

const MAX_TEXT_LEN: usize = 16 * 1024; // 16 KB — covers every real tweet by
// ~60x and leaves room for the triage
// prompt envelope.
const MAX_HANDLE_LEN: usize = 256;
const MAX_NAME_LEN: usize = 256;
const MAX_MEDIA_URLS: usize = 8;

fn trim_opt(s: &Option<String>, max_len: usize) -> Option<String> {
    s.as_ref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() > max_len {
            // Truncate at the last UTF-8 char boundary <= max_len.
            // Naïve byte slicing here would panic on multibyte split, which
            // would turn a hostile but well-formed JSON ingest into a 500.
            Some(truncate_at_char_boundary(trimmed, max_len).to_string())
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Return a `&str` slice of `s` truncated to at most `max_len` bytes, never
/// splitting a UTF-8 code point.
fn truncate_at_char_boundary(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// X/Twitter domains we accept on ingest. We reject everything else so the
/// scraper cannot accidentally feed arbitrary URLs into the triage LLM.
fn is_allowed_host(host: &str) -> bool {
    matches!(
        host,
        "x.com"
            | "www.x.com"
            | "twitter.com"
            | "www.twitter.com"
            | "mobile.twitter.com"
            | "mobile.x.com"
    )
}

/// Validated ingest item ready for persistence.
#[derive(Debug, Clone)]
pub struct NormalizedIngestItem {
    pub tweet_id: String,
    pub author_handle: Option<String>,
    pub author_name: Option<String>,
    pub text: String,
    pub url: String,
    pub media_urls: Vec<String>,
    pub quoted_tweet: Option<String>,
    pub thread_id: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid {0}: {1}")]
    InvalidField(String, String),
    #[error("batch must contain between 1 and {0} items")]
    BatchSize(usize),
}

/// Maximum number of items per ingest batch.
///
/// Sized to fit comfortably under the 14 MiB request body limit even with
/// pathological 16 KB text fields, and to keep dedupe/insert latency bounded.
pub const MAX_INGEST_BATCH: usize = 500;

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_item() -> BookmarkIngestItem {
        BookmarkIngestItem {
            tweet_id: "1820000000000000000".to_string(),
            author_handle: Some("alice".to_string()),
            author_name: Some("Alice".to_string()),
            text: Some("hello world".to_string()),
            url: "https://x.com/alice/status/1820000000000000000".to_string(),
            media_urls: vec![],
            quoted_tweet: None,
            thread_id: None,
            posted_at: None,
        }
    }

    #[test]
    fn validate_accepts_canonical_x_url() {
        validate_ingest_item(&ok_item()).unwrap();
    }

    #[test]
    fn validate_accepts_twitter_com() {
        let mut item = ok_item();
        item.url = "https://twitter.com/alice/status/1820000000000000000".to_string();
        validate_ingest_item(&item).unwrap();
    }

    #[test]
    fn validate_rejects_unknown_host() {
        let mut item = ok_item();
        item.url = "https://evil.example/alice/status/1".to_string();
        assert!(matches!(
            validate_ingest_item(&item),
            Err(IngestError::InvalidField(field, _)) if field == "url"
        ));
    }

    #[test]
    fn validate_rejects_non_http_scheme() {
        let mut item = ok_item();
        item.url = "javascript:alert(1)".to_string();
        assert!(matches!(
            validate_ingest_item(&item),
            Err(IngestError::InvalidField(field, _)) if field == "url"
        ));
    }

    #[test]
    fn validate_rejects_empty_tweet_id() {
        let mut item = ok_item();
        item.tweet_id = String::new();
        assert!(matches!(
            validate_ingest_item(&item),
            Err(IngestError::InvalidField(field, _)) if field == "tweet_id"
        ));
    }

    #[test]
    fn validate_rejects_non_alphanumeric_tweet_id() {
        let mut item = ok_item();
        item.tweet_id = "1234'; DROP TABLE x_bookmarks;--".to_string();
        assert!(matches!(
            validate_ingest_item(&item),
            Err(IngestError::InvalidField(field, _)) if field == "tweet_id"
        ));
    }

    #[test]
    fn validate_rejects_oversized_text() {
        let mut item = ok_item();
        item.text = Some("a".repeat(MAX_TEXT_LEN + 1));
        assert!(matches!(
            validate_ingest_item(&item),
            Err(IngestError::InvalidField(field, _)) if field == "text"
        ));
    }

    #[test]
    fn status_round_trips() {
        for s in [
            BookmarkStatus::Untriaged,
            BookmarkStatus::Build,
            BookmarkStatus::Read,
            BookmarkStatus::Reference,
            BookmarkStatus::Dead,
        ] {
            assert_eq!(BookmarkStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(BookmarkStatus::parse("nonsense"), None);
    }

    /// Regression: hostile metadata that crosses `max_len` mid-UTF-8 must
    /// not panic in `trim_opt`. Reported by Codex adversarial review.
    #[test]
    fn validate_handles_multibyte_metadata_at_truncation_boundary() {
        // 3-byte char "字" repeated. With MAX_HANDLE_LEN=256 this is well
        // within the cap, but if a future cap lands mid-codepoint, the slice
        // must round down to a char boundary.
        let mut item = ok_item();
        item.author_handle = Some("字".repeat(MAX_HANDLE_LEN));
        let normalized = validate_ingest_item(&item).expect("validation succeeds");
        let h = normalized.author_handle.unwrap();
        assert!(h.is_char_boundary(h.len()));
        assert!(h.len() <= MAX_HANDLE_LEN);
    }

    #[test]
    fn truncate_at_char_boundary_never_splits_codepoint() {
        // 4-byte char "𝓗". Asking for 3 bytes forces the helper to back off.
        let s = "𝓗abc";
        let truncated = truncate_at_char_boundary(s, 3);
        assert_eq!(truncated, "");
        let truncated = truncate_at_char_boundary(s, 4);
        assert_eq!(truncated, "𝓗");
    }
}
