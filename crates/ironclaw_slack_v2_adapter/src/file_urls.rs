//! Slack-controlled file URL validation shared by inbound and outbound transfers.

use thiserror::Error;
use url::Url;

use crate::payload::SLACK_FILES_HOST;

/// A Slack file URL was malformed or escaped the declared file host.
#[derive(Debug, Error)]
pub enum SlackFileUrlError {
    #[error("Slack returned an invalid file URL")]
    InvalidUrl(#[source] url::ParseError),
    #[error("Slack file URL escaped the allowed file host")]
    EscapedAllowedHost,
}

/// Validate a provider-issued Slack file URL and return its path and query.
///
/// Callers always dispatch to [`SLACK_FILES_HOST`] separately, so the parsed
/// authority never crosses into the mediated egress request.
pub fn confined_slack_file_path(raw: &str) -> Result<String, SlackFileUrlError> {
    let parsed = Url::parse(raw).map_err(SlackFileUrlError::InvalidUrl)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(SLACK_FILES_HOST)
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
    {
        return Err(SlackFileUrlError::EscapedAllowedHost);
    }
    let mut path = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_is_exact_host_confined_and_preserves_query() {
        assert_eq!(
            confined_slack_file_path(
                "https://files.slack.com/files-pri/T-F/report.pdf?pub_secret=x"
            )
            .expect("exact Slack file host"),
            "/files-pri/T-F/report.pdf?pub_secret=x"
        );
        for url in [
            "http://files.slack.com/files-pri/report.pdf",
            "https://files.slack.com.evil.example/report.pdf",
            "https://user@files.slack.com/report.pdf",
            "https://files.slack.com:8443/report.pdf",
            "https://files.slack.com/report.pdf#fragment",
        ] {
            assert!(
                confined_slack_file_path(url).is_err(),
                "{url} must be rejected"
            );
        }
    }
}
