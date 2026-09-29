//! `REBORN_TOOL_RETRIEVAL`: which ranker serves `tool_search`.
//!
//! This crate only parses the setting; composition turns the parsed mode into
//! a bound ranker (`ironclaw_composition::resolve_tool_retrieval_provider`).
//! The setting is explicit rather than implied by an `[embeddings]` section,
//! because the dense and hybrid rankers send every search query to the
//! embedding endpoint.

/// Environment variable selecting the `tool_search` ranker.
pub const REBORN_TOOL_RETRIEVAL_ENV: &str = "REBORN_TOOL_RETRIEVAL";

/// The `tool_search` ranker an operator selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolRetrievalMode {
    /// The host-bundled BM25F ranker. Nothing leaves the host.
    #[default]
    Native,
    /// The embedding ranker. Tool documents and search queries go to the
    /// configured embedding endpoint.
    Dense,
    /// BM25F fused with the embedding ranker, falling back to BM25F alone for
    /// any search the embedding ranker fails or is too slow to answer. Sends
    /// the same data to the embedding endpoint as `Dense`.
    Hybrid,
}

/// `REBORN_TOOL_RETRIEVAL` held a value that names no mode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{REBORN_TOOL_RETRIEVAL_ENV}={value:?} is not a known tool retrieval mode; expected {}",
    ToolRetrievalMode::ACCEPTED
)]
pub struct UnknownToolRetrievalMode {
    pub value: String,
}

impl ToolRetrievalMode {
    /// Every accepted spelling, for error messages.
    const ACCEPTED: &'static str = "`native`, `dense` or `hybrid`";

    /// Parse the raw setting. Unset or blank is the default; matching ignores
    /// ASCII case; anything else is an error, never a silent default.
    pub fn parse(raw: Option<&str>) -> Result<Self, UnknownToolRetrievalMode> {
        let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::default());
        };
        if value.eq_ignore_ascii_case("native") {
            Ok(Self::Native)
        } else if value.eq_ignore_ascii_case("dense") {
            Ok(Self::Dense)
        } else if value.eq_ignore_ascii_case("hybrid") {
            Ok(Self::Hybrid)
        } else {
            Err(UnknownToolRetrievalMode {
                value: value.to_string(),
            })
        }
    }

    /// The canonical spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Dense => "dense",
            Self::Hybrid => "hybrid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_blank_are_native() {
        assert_eq!(ToolRetrievalMode::default(), ToolRetrievalMode::Native);
        for raw in [None, Some(""), Some("  ")] {
            assert_eq!(ToolRetrievalMode::parse(raw), Ok(ToolRetrievalMode::Native));
        }
    }

    #[test]
    fn known_modes_parse_case_insensitively() {
        for (raw, mode) in [
            ("native", ToolRetrievalMode::Native),
            ("NATIVE", ToolRetrievalMode::Native),
            ("dense", ToolRetrievalMode::Dense),
            (" Dense ", ToolRetrievalMode::Dense),
            ("hybrid", ToolRetrievalMode::Hybrid),
            ("HyBrid", ToolRetrievalMode::Hybrid),
        ] {
            assert_eq!(ToolRetrievalMode::parse(Some(raw)), Ok(mode), "{raw:?}");
            assert_eq!(ToolRetrievalMode::parse(Some(mode.as_str())), Ok(mode));
        }
    }

    #[test]
    fn unknown_values_are_errors_naming_the_setting() {
        let error = ToolRetrievalMode::parse(Some("semantic")).expect_err("unknown mode");
        assert_eq!(
            error,
            UnknownToolRetrievalMode {
                value: "semantic".to_string()
            }
        );
        let message = error.to_string();
        assert!(message.contains(REBORN_TOOL_RETRIEVAL_ENV), "{message}");
        assert!(
            message.contains("`native`, `dense` or `hybrid`"),
            "{message}"
        );
    }
}
