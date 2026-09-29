//! Turn-start tool selection settings: the `[tool_selection]` config
//! section and its `REBORN_TOOL_PREFETCH*` environment overrides.
//!
//! This crate only parses and validates the settings; composition turns them
//! into the loop host's typed configuration, and the binary binds the chosen
//! classifier. With selection on, a conversation advertises only the tools
//! chosen from its opening request, plus an always-on floor, and keeps that
//! list while the provider's prompt cache could be warm; with re-selection on
//! (the default whenever selection is on) it chooses again from the
//! conversation so far after an idle gap past the cache lifetime or a model
//! change (see `ironclaw_loop_host`'s `tool_prefetch` module).
//!
//! Precedence, field by field: an environment variable that is set and not
//! blank wins over `[tool_selection]` in `config.toml`, which wins over the
//! compiled defaults.
//!
//! Everything is refused at startup rather than silently defaulted: an
//! unknown mode or classifier, an unparsable or out-of-range number,
//! `semantic` with the local classifier but without a dense or hybrid
//! `REBORN_TOOL_RETRIEVAL` ranker, a floor (mandatory tools plus extras)
//! larger than the maximum advertised tools, and `jev` without its API key
//! or with an endpoint that is not a plain `https` URL.
//! With the mode `off` (the default) nothing else is read.

use crate::{ToolRetrievalMode, ToolSelectionSection};

/// Which ranker turn-start selection uses, or `off`.
pub const REBORN_TOOL_PREFETCH_ENV: &str = "REBORN_TOOL_PREFETCH";
/// Which classifier chooses the tools: `local` or `jev`.
pub const REBORN_TOOL_PREFETCH_CLASSIFIER_ENV: &str = "REBORN_TOOL_PREFETCH_CLASSIFIER";
/// Most tools the request's `tools` array may hold, floor and extras included.
pub(crate) const REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV: &str = "REBORN_TOOL_PREFETCH_MAX_TOOLS";
/// Absolute similarity threshold, applied to cosine (dense) scores only; off
/// (0) by default.
pub(crate) const REBORN_TOOL_PREFETCH_MIN_SIMILARITY_ENV: &str =
    "REBORN_TOOL_PREFETCH_MIN_SIMILARITY";
/// Relative threshold (a score over the top score of the same segment's
/// ranking), applied to every ranker; off (0) by default.
pub(crate) const REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV: &str = "REBORN_TOOL_PREFETCH_MIN_RELATIVE";
/// Most estimated schema tokens the advertised tools may add up to.
pub(crate) const REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV: &str = "REBORN_TOOL_PREFETCH_TOKEN_BUDGET";
/// Comma-separated extra tools always advertised when authorized.
pub(crate) const REBORN_TOOL_PREFETCH_ALWAYS_ENV: &str = "REBORN_TOOL_PREFETCH_ALWAYS";
/// `on` or `off`: whether a conversation re-selects once its prompt cache can
/// no longer be warm.
pub(crate) const REBORN_TOOL_PREFETCH_RESELECT_ENV: &str = "REBORN_TOOL_PREFETCH_RESELECT";
/// Prompt-cache lifetime assumed for providers the host cannot know, seconds.
pub(crate) const REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS_ENV: &str =
    "REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS";
/// Margin added to every cache lifetime, seconds.
pub(crate) const REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS_ENV: &str =
    "REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS";
/// Most user messages a selection reads (a re-selection's window), and most
/// segments the local classifier ranks.
pub(crate) const REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV: &str =
    "REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES";
/// Most bytes of one segment the local classifier ranks.
pub(crate) const REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV: &str =
    "REBORN_TOOL_PREFETCH_SEGMENT_BYTES";
/// The decisions endpoint the `jev` classifier posts to, a full `https` URL.
pub const REBORN_TOOL_PREFETCH_JEV_ENDPOINT_ENV: &str = "REBORN_TOOL_PREFETCH_JEV_ENDPOINT";
/// The model the `jev` classifier names.
pub(crate) const REBORN_TOOL_PREFETCH_JEV_MODEL_ENV: &str = "REBORN_TOOL_PREFETCH_JEV_MODEL";
/// The NAME of the environment variable holding the `jev` provider's key.
pub(crate) const REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV_ENV: &str =
    "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV";

const MAX_TOOLS_LABEL: &str = "REBORN_TOOL_PREFETCH_MAX_TOOLS / [tool_selection] max_tools";
const TOKEN_BUDGET_LABEL: &str =
    "REBORN_TOOL_PREFETCH_TOKEN_BUDGET / [tool_selection] token_budget";
const MIN_SIMILARITY_LABEL: &str =
    "REBORN_TOOL_PREFETCH_MIN_SIMILARITY / [tool_selection.local] min_similarity";
const MIN_RELATIVE_LABEL: &str =
    "REBORN_TOOL_PREFETCH_MIN_RELATIVE / [tool_selection.local] min_relative";
const CONTEXT_MESSAGES_LABEL: &str =
    "REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES / [tool_selection] context_messages";
const SEGMENT_BYTES_LABEL: &str =
    "REBORN_TOOL_PREFETCH_SEGMENT_BYTES / [tool_selection] segment_bytes";

/// Default [`ToolPrefetchSettings::max_tools`], from the baseline benchmark.
pub(crate) const DEFAULT_TOOL_PREFETCH_MAX_TOOLS: usize = 100;
/// Default [`ToolPrefetchSettings::token_budget`]: about 200 tools at the
/// baseline's ~155 estimated tokens per tool, so `max_tools` is the cap that
/// binds and the budget guards only against a few huge schemas.
pub(crate) const DEFAULT_TOOL_PREFETCH_TOKEN_BUDGET: u32 = 32_000;
/// Default [`ToolPrefetchSettings::min_similarity`] and
/// [`ToolPrefetchSettings::min_relative`]: off. Selection fills `max_tools`
/// by rank; a threshold does not calibrate across score scales.
pub(crate) const DEFAULT_TOOL_PREFETCH_THRESHOLD: f32 = 0.0;
/// Default [`ToolReselectionSettings::cache_lifetime_secs`]: OpenAI documents
/// 5 to 10 minutes idle, up to an hour off-peak; assuming the long end leans
/// towards "warm", which only misses a re-selection when wrong.
pub(crate) const DEFAULT_TOOL_PREFETCH_CACHE_LIFETIME_SECS: u64 = 3_600;
/// Default [`ToolReselectionSettings::cache_margin_secs`].
pub(crate) const DEFAULT_TOOL_PREFETCH_CACHE_MARGIN_SECS: u64 = 60;
/// Default [`ToolPrefetchSettings::context_messages`].
pub(crate) const DEFAULT_TOOL_PREFETCH_CONTEXT_MESSAGES: usize = 16;
/// Largest [`ToolPrefetchSettings::context_messages`]; mirrors the loop
/// host's `MAX_CONTEXT_MESSAGES` (composition pins the two equal).
pub const MAX_TOOL_PREFETCH_CONTEXT_MESSAGES: usize = 64;
/// Default [`ToolPrefetchSettings::segment_bytes`]: about 512 tokens of
/// prose, what a small embedding model such as `bge-small` reads.
pub(crate) const DEFAULT_TOOL_PREFETCH_SEGMENT_BYTES: usize = 2_048;
/// Smallest and largest [`ToolPrefetchSettings::segment_bytes`]; mirror the
/// loop host's `MIN_CONTEXT_SEGMENT_BYTES` and
/// `MAX_CONTEXT_SEGMENT_BYTES` (composition pins them equal).
pub const MIN_TOOL_PREFETCH_SEGMENT_BYTES: usize = 128;
pub const MAX_TOOL_PREFETCH_SEGMENT_BYTES: usize = 4 * 1_024;
/// Default [`JevSettings::endpoint`]: TypeSafe's decisions endpoint. Any
/// provider serving the same decisions API can be named instead.
pub const DEFAULT_JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// Default [`JevSettings::model`]: TypeSafe's `jev-latest` alias, its
/// flagship. It moves between Jev releases, so the classifier logs the model
/// the server reports; `[tool_selection.jev] model` pins a version (for
/// example `jev-1.13.0`).
pub const DEFAULT_JEV_MODEL: &str = "jev-latest";
/// Default [`JevSettings::api_key_env`]: TypeSafe's API key.
pub const DEFAULT_JEV_API_KEY_ENV: &str = "TYPESAFE_API_KEY";
/// Default [`JevSettings::timeout_ms`].
pub const DEFAULT_JEV_TIMEOUT_MS: u64 = 500;

/// Tools every selection advertises when authorized, counted against
/// `max_tools`. Mirrors the loop host's floor; composition pins the two
/// lists equal.
pub const TOOL_PREFETCH_MANDATORY_FLOOR: [&str; 4] =
    ["tool_search", "tool_describe", "tool_call", "result_read"];

/// The operator's `REBORN_TOOL_PREFETCH` choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolPrefetchMode {
    /// Today's behaviour: no turn-start selection.
    #[default]
    Off,
    /// Rank with the host-bundled BM25F ranker. Nothing leaves the host.
    Lexical,
    /// Rank with the `REBORN_TOOL_RETRIEVAL` ranker, which must be `dense`
    /// or `hybrid`. The opening request goes to the embedding endpoint.
    Semantic,
}

impl ToolPrefetchMode {
    const ACCEPTED: &'static str = "`off`, `lexical` or `semantic`";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
        }
    }

    fn parse(raw: Option<&str>) -> Result<Self, ToolPrefetchSettingsError> {
        let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::default());
        };
        [Self::Off, Self::Lexical, Self::Semantic]
            .into_iter()
            .find(|mode| value.eq_ignore_ascii_case(mode.as_str()))
            .ok_or_else(|| ToolPrefetchSettingsError::UnknownMode {
                value: value.to_string(),
            })
    }
}

/// Jev settings. The API key itself is never held here: the binary reads it
/// host-side from `api_key_env` when it builds the classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JevSettings {
    /// The decisions endpoint: an `https` URL with a host, no userinfo, no
    /// query and no fragment (checked here). The egress pin allows exactly
    /// its host and port.
    pub endpoint: String,
    pub model: String,
    /// NAME of the environment variable holding the key; checked to be set
    /// and not blank.
    pub api_key_env: String,
    /// Time allowed for one whole classification.
    pub timeout_ms: u64,
}

/// Which classifier chooses the tools.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ToolSelectionClassifierSettings {
    /// The loop host's ranker and thresholds, on the host.
    #[default]
    Local,
    /// Jev, a hosted classifier served by the configured provider (TypeSafe
    /// by default); sends the opening request and the tool catalog to that
    /// third party.
    Jev(JevSettings),
}

impl ToolSelectionClassifierSettings {
    const ACCEPTED: &'static str = "`local` or `jev`";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Jev(_) => "jev",
        }
    }
}

/// Validated turn-start selection settings (mode `lexical` or `semantic`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPrefetchSettings {
    pub mode: ToolPrefetchMode,
    pub classifier: ToolSelectionClassifierSettings,
    pub max_tools: usize,
    pub token_budget: u32,
    /// Absolute cosine threshold; 0 (off) by default.
    pub min_similarity: f32,
    /// Relative threshold; 0 (off) by default.
    pub min_relative: f32,
    /// Extra floor tools, trimmed, empty entries dropped, first spelling kept.
    pub always: Vec<String>,
    /// Most user messages a selection reads, and most segments the local
    /// classifier ranks.
    pub context_messages: usize,
    /// Most bytes of one segment the local classifier ranks.
    pub segment_bytes: usize,
    pub reselection: ToolReselectionSettings,
}

/// When a conversation chooses its tools again after its first selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReselectionSettings {
    /// Re-select once the prompt cache can no longer be warm (default on).
    pub enabled: bool,
    /// Cache lifetime assumed for providers the host cannot know.
    pub cache_lifetime_secs: u64,
    /// Added to every cache lifetime before the cache counts as cold.
    pub cache_margin_secs: u64,
}

impl Default for ToolReselectionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_lifetime_secs: DEFAULT_TOOL_PREFETCH_CACHE_LIFETIME_SECS,
            cache_margin_secs: DEFAULT_TOOL_PREFETCH_CACHE_MARGIN_SECS,
        }
    }
}

/// Why the turn-start selection settings were refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ToolPrefetchSettingsError {
    #[error(
        "{REBORN_TOOL_PREFETCH_ENV} / [tool_selection] mode = {value:?} is not a known tool \
         prefetch mode; expected {}",
        ToolPrefetchMode::ACCEPTED
    )]
    UnknownMode { value: String },
    #[error(
        "{REBORN_TOOL_PREFETCH_CLASSIFIER_ENV} / [tool_selection] classifier = {value:?} is not \
         a known tool classifier; expected {}",
        ToolSelectionClassifierSettings::ACCEPTED
    )]
    UnknownClassifier { value: String },
    #[error("{name} must contain valid UTF-8")]
    NotUnicode { name: String },
    #[error("{name}={value:?} is not a {expected}")]
    InvalidNumber {
        name: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("{name}={value} must be between 0 and 1")]
    OutOfRange { name: &'static str, value: f32 },
    #[error("{name} must be greater than 0")]
    Zero { name: &'static str },
    #[error(
        "{REBORN_TOOL_PREFETCH_RESELECT_ENV} / [tool_selection] reselect = {value:?} is not \
         `on` or `off`"
    )]
    InvalidSwitch { value: String },
    #[error("{name}={value} must be between {min} and {max}")]
    NotInRange {
        name: &'static str,
        value: usize,
        min: usize,
        max: usize,
    },
    #[error(
        "{REBORN_TOOL_PREFETCH_ENV}=semantic ranks with the {}={} ranker, which is not a \
         semantic ranker; set {} to `dense` or `hybrid`, or use \
         {REBORN_TOOL_PREFETCH_ENV}=lexical",
        crate::REBORN_TOOL_RETRIEVAL_ENV,
        retrieval.as_str(),
        crate::REBORN_TOOL_RETRIEVAL_ENV
    )]
    SemanticNeedsDenseOrHybrid { retrieval: ToolRetrievalMode },
    #[error(
        "the always-on tool floor ({floor} tools: the {} mandatory tools plus \
         {REBORN_TOOL_PREFETCH_ALWAYS_ENV} / [tool_selection] always) exceeds \
         {REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV} / [tool_selection] max_tools = {max_tools}; raise \
         the maximum or list fewer extras",
        TOOL_PREFETCH_MANDATORY_FLOOR.len()
    )]
    FloorExceedsMaxTools { floor: usize, max_tools: usize },
    #[error(
        "the `jev` tool classifier needs its provider's API key in the environment variable \
         {api_key_env}, which is unset or empty; set it, or choose the `local` classifier. \
         Turn-start selection does not fall back to `local` on its own"
    )]
    JevKeyMissing { api_key_env: String },
    #[error(
        "{REBORN_TOOL_PREFETCH_JEV_ENDPOINT_ENV} / [tool_selection.jev] endpoint is refused: \
         {reason}. It must be a full https URL with a host and no userinfo, query or fragment, \
         for example {DEFAULT_JEV_ENDPOINT}"
    )]
    InvalidJevEndpoint { reason: &'static str },
    #[error(
        "{REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV_ENV} must be the NAME of an environment variable \
         (letters, digits and `_`, not starting with a digit), never the key itself"
    )]
    InvalidJevApiKeyEnv,
}

/// One environment lookup: `Ok(None)` when unset, an error when set but not
/// valid UTF-8.
pub type ToolPrefetchEnvLookup<'a> =
    dyn Fn(&str) -> Result<Option<String>, ToolPrefetchSettingsError> + 'a;

impl ToolPrefetchSettings {
    /// Resolve the settings from `file` (`[tool_selection]`, when present)
    /// and the environment through `lookup`, which wins field by field.
    /// `Ok(None)` means selection is off; `retrieval` is the parsed
    /// `REBORN_TOOL_RETRIEVAL` mode, which a `semantic` local selection
    /// ranks with.
    pub fn resolve(
        file: Option<&ToolSelectionSection>,
        lookup: &ToolPrefetchEnvLookup<'_>,
        retrieval: ToolRetrievalMode,
    ) -> Result<Option<Self>, ToolPrefetchSettingsError> {
        let from_env = |name: &'static str| -> Result<Option<String>, ToolPrefetchSettingsError> {
            Ok(lookup(name)?.filter(|value| !value.trim().is_empty()))
        };
        let mode = ToolPrefetchMode::parse(
            from_env(REBORN_TOOL_PREFETCH_ENV)?
                .as_deref()
                .or(file.and_then(|file| file.mode.as_deref())),
        )?;
        if mode == ToolPrefetchMode::Off {
            return Ok(None);
        }
        let local = file.and_then(|file| file.local.as_ref());
        let classifier = match from_env(REBORN_TOOL_PREFETCH_CLASSIFIER_ENV)?
            .as_deref()
            .or(file.and_then(|file| file.classifier.as_deref()))
            .map(str::trim)
        {
            None => ToolSelectionClassifierSettings::Local,
            Some(value) if value.eq_ignore_ascii_case("local") => {
                ToolSelectionClassifierSettings::Local
            }
            Some(value) if value.eq_ignore_ascii_case("jev") => {
                let jev = file.and_then(|file| file.jev.as_ref());
                let api_key_env = match from_env(REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV_ENV)? {
                    // The file's value is checked where the file is parsed.
                    Some(name) => check_env_var_name(name.trim())?,
                    None => jev
                        .and_then(|jev| jev.api_key_env.clone())
                        .unwrap_or_else(|| DEFAULT_JEV_API_KEY_ENV.to_string()),
                };
                let timeout_ms = jev
                    .and_then(|jev| jev.timeout_ms)
                    .unwrap_or(DEFAULT_JEV_TIMEOUT_MS);
                if timeout_ms == 0 {
                    return Err(ToolPrefetchSettingsError::Zero {
                        name: "[tool_selection.jev] timeout_ms",
                    });
                }
                let endpoint = check_jev_endpoint(
                    from_env(REBORN_TOOL_PREFETCH_JEV_ENDPOINT_ENV)?
                        .as_deref()
                        .or(jev.and_then(|jev| jev.endpoint.as_deref()))
                        .unwrap_or(DEFAULT_JEV_ENDPOINT),
                )?;
                if lookup(&api_key_env)?.is_none_or(|key| key.trim().is_empty()) {
                    return Err(ToolPrefetchSettingsError::JevKeyMissing { api_key_env });
                }
                ToolSelectionClassifierSettings::Jev(JevSettings {
                    endpoint,
                    model: from_env(REBORN_TOOL_PREFETCH_JEV_MODEL_ENV)?
                        .map(|model| model.trim().to_string())
                        .or_else(|| jev.and_then(|jev| jev.model.clone()))
                        .unwrap_or_else(|| DEFAULT_JEV_MODEL.to_string()),
                    api_key_env,
                    timeout_ms,
                })
            }
            Some(value) => {
                return Err(ToolPrefetchSettingsError::UnknownClassifier {
                    value: value.to_string(),
                });
            }
        };
        // The ranker matters only to the local classifier.
        if mode == ToolPrefetchMode::Semantic
            && classifier == ToolSelectionClassifierSettings::Local
            && retrieval == ToolRetrievalMode::Native
        {
            return Err(ToolPrefetchSettingsError::SemanticNeedsDenseOrHybrid { retrieval });
        }
        let max_tools = parse_number(
            from_env(REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV)?,
            REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV,
        )?
        .or(file.and_then(|file| file.max_tools))
        .unwrap_or(DEFAULT_TOOL_PREFETCH_MAX_TOOLS);
        if max_tools == 0 {
            return Err(ToolPrefetchSettingsError::Zero {
                name: MAX_TOOLS_LABEL,
            });
        }
        let token_budget = parse_number(
            from_env(REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV)?,
            REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV,
        )?
        .or(file.and_then(|file| file.token_budget))
        .unwrap_or(DEFAULT_TOOL_PREFETCH_TOKEN_BUDGET);
        if token_budget == 0 {
            return Err(ToolPrefetchSettingsError::Zero {
                name: TOKEN_BUDGET_LABEL,
            });
        }
        let min_similarity = check_fraction(
            MIN_SIMILARITY_LABEL,
            parse_fraction(
                from_env(REBORN_TOOL_PREFETCH_MIN_SIMILARITY_ENV)?,
                REBORN_TOOL_PREFETCH_MIN_SIMILARITY_ENV,
            )?
            .or(local
                .and_then(|local| local.min_similarity)
                .map(|value| value as f32)),
        )?
        .unwrap_or(DEFAULT_TOOL_PREFETCH_THRESHOLD);
        let min_relative = check_fraction(
            MIN_RELATIVE_LABEL,
            parse_fraction(
                from_env(REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV)?,
                REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV,
            )?
            .or(local
                .and_then(|local| local.min_relative)
                .map(|value| value as f32)),
        )?
        .unwrap_or(DEFAULT_TOOL_PREFETCH_THRESHOLD);
        let listed: Vec<String> = match from_env(REBORN_TOOL_PREFETCH_ALWAYS_ENV)? {
            Some(value) => value.split(',').map(str::to_string).collect(),
            None => file
                .and_then(|file| file.always.clone())
                .unwrap_or_default(),
        };
        let mut always: Vec<String> = Vec::new();
        for name in listed.iter().map(|name| name.trim()) {
            if !name.is_empty() && !always.iter().any(|kept| kept == name) {
                always.push(name.to_string());
            }
        }
        let floor = TOOL_PREFETCH_MANDATORY_FLOOR.len()
            + always
                .iter()
                .filter(|name| !TOOL_PREFETCH_MANDATORY_FLOOR.contains(&name.as_str()))
                .count();
        if floor > max_tools {
            return Err(ToolPrefetchSettingsError::FloorExceedsMaxTools { floor, max_tools });
        }
        let context_messages = parse_bounded(
            from_env(REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV)?,
            REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV,
            file.and_then(|file| file.context_messages),
            (
                CONTEXT_MESSAGES_LABEL,
                1,
                MAX_TOOL_PREFETCH_CONTEXT_MESSAGES,
            ),
        )?
        .unwrap_or(DEFAULT_TOOL_PREFETCH_CONTEXT_MESSAGES);
        let segment_bytes = parse_bounded(
            from_env(REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV)?,
            REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV,
            file.and_then(|file| file.segment_bytes),
            (
                SEGMENT_BYTES_LABEL,
                MIN_TOOL_PREFETCH_SEGMENT_BYTES,
                MAX_TOOL_PREFETCH_SEGMENT_BYTES,
            ),
        )?
        .unwrap_or(DEFAULT_TOOL_PREFETCH_SEGMENT_BYTES);
        let reselection = ToolReselectionSettings::resolve(file, &from_env)?;
        Ok(Some(Self {
            mode,
            classifier,
            max_tools,
            token_budget,
            min_similarity,
            min_relative,
            always,
            context_messages,
            segment_bytes,
            reselection,
        }))
    }
}

impl ToolReselectionSettings {
    fn resolve(
        file: Option<&ToolSelectionSection>,
        from_env: &dyn Fn(&'static str) -> Result<Option<String>, ToolPrefetchSettingsError>,
    ) -> Result<Self, ToolPrefetchSettingsError> {
        let defaults = Self::default();
        let enabled = match from_env(REBORN_TOOL_PREFETCH_RESELECT_ENV)? {
            Some(value) => parse_switch(&value)?,
            None => file
                .and_then(|file| file.reselect)
                .unwrap_or(defaults.enabled),
        };
        let cache_lifetime_secs = parse_number(
            from_env(REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS_ENV)?,
            REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS_ENV,
        )?
        .or(file.and_then(|file| file.cache_lifetime_secs))
        .unwrap_or(defaults.cache_lifetime_secs);
        let cache_margin_secs = parse_number(
            from_env(REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS_ENV)?,
            REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS_ENV,
        )?
        .or(file.and_then(|file| file.cache_margin_secs))
        .unwrap_or(defaults.cache_margin_secs);
        Ok(Self {
            enabled,
            cache_lifetime_secs,
            cache_margin_secs,
        })
    }
}

/// `raw`, trimmed, when it is an `https` URL with a host and no userinfo,
/// query or fragment. The error names the rule broken, never the URL, which
/// could carry a password in its userinfo. The Jev package checks the same
/// rules again when it derives its egress pin.
fn check_jev_endpoint(raw: &str) -> Result<String, ToolPrefetchSettingsError> {
    let invalid = |reason: &'static str| ToolPrefetchSettingsError::InvalidJevEndpoint { reason };
    let trimmed = raw.trim();
    let url = url::Url::parse(trimmed).map_err(|error| match error {
        url::ParseError::EmptyHost => invalid("it has no host"),
        _ => invalid("it is not a valid URL"),
    })?;
    if url.scheme() != "https" {
        return Err(invalid("it does not use https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("it carries userinfo"));
    }
    if url.query().is_some() {
        return Err(invalid("it carries a query"));
    }
    if url.fragment().is_some() {
        return Err(invalid("it carries a fragment"));
    }
    match url.host_str() {
        None | Some("") => return Err(invalid("it has no host")),
        Some(host) if host.contains('*') => return Err(invalid("its host is a wildcard")),
        Some(_) => {}
    }
    Ok(trimmed.to_string())
}

/// `name` when it is shaped like an environment variable name. The error
/// never repeats it: a pasted key must not reach a log.
fn check_env_var_name(name: &str) -> Result<String, ToolPrefetchSettingsError> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_');
    if valid {
        Ok(name.to_string())
    } else {
        Err(ToolPrefetchSettingsError::InvalidJevApiKeyEnv)
    }
}

fn parse_switch(value: &str) -> Result<bool, ToolPrefetchSettingsError> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("on") {
        Ok(true)
    } else if value.eq_ignore_ascii_case("off") {
        Ok(false)
    } else {
        Err(ToolPrefetchSettingsError::InvalidSwitch {
            value: value.to_string(),
        })
    }
}

fn parse_number<T: std::str::FromStr>(
    raw: Option<String>,
    name: &'static str,
) -> Result<Option<T>, ToolPrefetchSettingsError> {
    raw.map(|value| {
        let value = value.trim();
        value
            .parse()
            .map_err(|_| ToolPrefetchSettingsError::InvalidNumber {
                name,
                value: value.to_string(),
                expected: "whole number",
            })
    })
    .transpose()
}

/// A whole number from the environment (`raw`, named `env_name`) or else
/// the file, checked against `(label, min, max)` inclusive.
fn parse_bounded(
    raw: Option<String>,
    env_name: &'static str,
    file: Option<usize>,
    (label, min, max): (&'static str, usize, usize),
) -> Result<Option<usize>, ToolPrefetchSettingsError> {
    let value = parse_number(raw, env_name)?.or(file);
    match value {
        Some(value) if !(min..=max).contains(&value) => {
            Err(ToolPrefetchSettingsError::NotInRange {
                name: label,
                value,
                min,
                max,
            })
        }
        value => Ok(value),
    }
}

fn parse_fraction(
    raw: Option<String>,
    name: &'static str,
) -> Result<Option<f32>, ToolPrefetchSettingsError> {
    raw.map(|value| {
        let value = value.trim();
        value
            .parse()
            .map_err(|_| ToolPrefetchSettingsError::InvalidNumber {
                name,
                value: value.to_string(),
                expected: "number between 0 and 1",
            })
    })
    .transpose()
}

fn check_fraction(
    name: &'static str,
    value: Option<f32>,
) -> Result<Option<f32>, ToolPrefetchSettingsError> {
    match value {
        Some(value) if !(value.is_finite() && (0.0..=1.0).contains(&value)) => {
            Err(ToolPrefetchSettingsError::OutOfRange { name, value })
        }
        value => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn read(
        vars: &[(&str, &str)],
        retrieval: ToolRetrievalMode,
    ) -> Result<Option<ToolPrefetchSettings>, ToolPrefetchSettingsError> {
        resolve(None, vars, retrieval)
    }

    fn resolve(
        file: Option<&str>,
        vars: &[(&str, &str)],
        retrieval: ToolRetrievalMode,
    ) -> Result<Option<ToolPrefetchSettings>, ToolPrefetchSettingsError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        let file = file.map(|text| {
            crate::RebornConfigFile::parse_text(text, std::path::Path::new("/test/config.toml"))
                .expect("valid config")
                .tool_selection
                .expect("[tool_selection]")
        });
        ToolPrefetchSettings::resolve(
            file.as_ref(),
            &|name| Ok(vars.get(name).cloned()),
            retrieval,
        )
    }

    #[test]
    fn off_is_the_default_and_ignores_the_other_settings() {
        for mode in [None, Some(""), Some("off"), Some(" OFF ")] {
            let mut vars = vec![(REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV, "not a number")];
            if let Some(mode) = mode {
                vars.push((REBORN_TOOL_PREFETCH_ENV, mode));
            }
            assert_eq!(read(&vars, ToolRetrievalMode::Native), Ok(None), "{mode:?}");
        }
    }

    #[test]
    fn lexical_uses_the_documented_defaults() {
        let settings = read(
            &[(REBORN_TOOL_PREFETCH_ENV, "Lexical")],
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(
            settings,
            ToolPrefetchSettings {
                mode: ToolPrefetchMode::Lexical,
                classifier: ToolSelectionClassifierSettings::Local,
                max_tools: 100,
                token_budget: 32_000,
                min_similarity: 0.0,
                min_relative: 0.0,
                always: Vec::new(),
                context_messages: 16,
                segment_bytes: 2_048,
                reselection: ToolReselectionSettings {
                    enabled: true,
                    cache_lifetime_secs: 3_600,
                    cache_margin_secs: 60,
                },
            }
        );
    }

    #[test]
    fn every_setting_parses_and_extras_are_trimmed_and_deduplicated() {
        let settings = read(
            &[
                (REBORN_TOOL_PREFETCH_ENV, "semantic"),
                (REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV, " 40 "),
                (REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV, "9000"),
                (REBORN_TOOL_PREFETCH_MIN_SIMILARITY_ENV, "0.4"),
                (REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV, "0.6"),
                (
                    REBORN_TOOL_PREFETCH_ALWAYS_ENV,
                    "outbound_deliver, outbound_delivery_targets_list,,outbound_deliver",
                ),
            ],
            ToolRetrievalMode::Hybrid,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(settings.mode, ToolPrefetchMode::Semantic);
        assert_eq!(settings.max_tools, 40);
        assert_eq!(settings.token_budget, 9_000);
        assert_eq!(settings.min_similarity, 0.4);
        assert_eq!(settings.min_relative, 0.6);
        assert_eq!(
            settings.always,
            vec!["outbound_deliver", "outbound_delivery_targets_list"]
        );
    }

    #[test]
    fn unknown_modes_and_bad_numbers_refuse_startup() {
        let unknown = read(
            &[(REBORN_TOOL_PREFETCH_ENV, "dense")],
            ToolRetrievalMode::Dense,
        )
        .expect_err("unknown mode");
        assert!(unknown.to_string().contains(REBORN_TOOL_PREFETCH_ENV));
        assert!(
            unknown
                .to_string()
                .contains("`off`, `lexical` or `semantic`")
        );

        for (name, value) in [
            (REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV, "-3"),
            (REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV, "lots"),
            (REBORN_TOOL_PREFETCH_MIN_SIMILARITY_ENV, "high"),
        ] {
            let error = read(
                &[(REBORN_TOOL_PREFETCH_ENV, "lexical"), (name, value)],
                ToolRetrievalMode::Native,
            )
            .expect_err("bad number");
            assert!(matches!(
                error,
                ToolPrefetchSettingsError::InvalidNumber { .. }
            ));
            assert!(error.to_string().contains(name), "{error}");
        }
        for value in ["1.5", "-0.1", "NaN"] {
            let error = read(
                &[
                    (REBORN_TOOL_PREFETCH_ENV, "lexical"),
                    (REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV, value),
                ],
                ToolRetrievalMode::Native,
            )
            .expect_err("out of range");
            assert!(matches!(
                error,
                ToolPrefetchSettingsError::OutOfRange { .. }
            ));
        }
    }

    #[test]
    fn reselection_settings_are_checked() {
        let lexical = (REBORN_TOOL_PREFETCH_ENV, "lexical");
        for (name, value) in [
            (REBORN_TOOL_PREFETCH_RESELECT_ENV, "maybe"),
            (REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS_ENV, "an hour"),
            (REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV, "0"),
            (REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV, "65"),
            (REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV, "many"),
            (REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV, "127"),
            (REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV, "4097"),
        ] {
            let error =
                read(&[lexical, (name, value)], ToolRetrievalMode::Native).expect_err("refused");
            assert!(error.to_string().contains(name), "{error}");
        }
        let off = read(
            &[lexical, (REBORN_TOOL_PREFETCH_RESELECT_ENV, " off ")],
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        assert!(!off.reselection.enabled);
    }

    #[test]
    fn semantic_refuses_the_native_ranker() {
        let error = read(
            &[(REBORN_TOOL_PREFETCH_ENV, "semantic")],
            ToolRetrievalMode::Native,
        )
        .expect_err("semantic over native");
        assert_eq!(
            error,
            ToolPrefetchSettingsError::SemanticNeedsDenseOrHybrid {
                retrieval: ToolRetrievalMode::Native
            }
        );
        assert!(error.to_string().contains("REBORN_TOOL_RETRIEVAL"));
        for retrieval in [ToolRetrievalMode::Dense, ToolRetrievalMode::Hybrid] {
            assert!(
                read(&[(REBORN_TOOL_PREFETCH_ENV, "semantic")], retrieval)
                    .expect("dense and hybrid are semantic rankers")
                    .is_some()
            );
        }
    }

    #[test]
    fn a_floor_larger_than_max_tools_refuses_startup() {
        let error = read(
            &[
                (REBORN_TOOL_PREFETCH_ENV, "lexical"),
                (REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV, "5"),
                (
                    REBORN_TOOL_PREFETCH_ALWAYS_ENV,
                    "outbound_deliver,trigger_create,tool_search",
                ),
            ],
            ToolRetrievalMode::Native,
        )
        .expect_err("floor of six over five");
        assert_eq!(
            error,
            ToolPrefetchSettingsError::FloorExceedsMaxTools {
                floor: 6,
                max_tools: 5
            }
        );
        assert!(error.to_string().contains(REBORN_TOOL_PREFETCH_ALWAYS_ENV));
    }

    const FILE: &str = r#"
[tool_selection]
mode = "lexical"
always = ["outbound_deliver"]
max_tools = 60
token_budget = 12000
reselect = false
cache_lifetime_secs = 900
cache_margin_secs = 30
context_messages = 8
segment_bytes = 1024

[tool_selection.local]
min_similarity = 0.5
min_relative = 0.6

[tool_selection.jev]
endpoint = "https://jev.example.test/api/v1/decisions"
model = "jev-1.12.0"
api_key_env = "MY_JEV_KEY"
timeout_ms = 900
"#;

    #[test]
    fn the_file_configures_selection_and_the_environment_wins_field_by_field() {
        let from_file = resolve(Some(FILE), &[], ToolRetrievalMode::Native)
            .expect("valid")
            .expect("on");
        assert_eq!(
            from_file,
            ToolPrefetchSettings {
                mode: ToolPrefetchMode::Lexical,
                classifier: ToolSelectionClassifierSettings::Local,
                max_tools: 60,
                token_budget: 12_000,
                min_similarity: 0.5,
                min_relative: 0.6,
                always: vec!["outbound_deliver".to_string()],
                context_messages: 8,
                segment_bytes: 1_024,
                reselection: ToolReselectionSettings {
                    enabled: false,
                    cache_lifetime_secs: 900,
                    cache_margin_secs: 30,
                },
            }
        );

        let overridden = resolve(
            Some(FILE),
            &[
                (REBORN_TOOL_PREFETCH_ENV, "semantic"),
                (REBORN_TOOL_PREFETCH_MAX_TOOLS_ENV, "40"),
                (REBORN_TOOL_PREFETCH_MIN_RELATIVE_ENV, "0.2"),
                (REBORN_TOOL_PREFETCH_ALWAYS_ENV, "trigger_create"),
                (REBORN_TOOL_PREFETCH_RESELECT_ENV, "ON"),
                (REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS_ENV, "600"),
                (REBORN_TOOL_PREFETCH_CONTEXT_MESSAGES_ENV, "32"),
                (REBORN_TOOL_PREFETCH_SEGMENT_BYTES_ENV, "4096"),
                // Blank means unset: the file's value stays.
                (REBORN_TOOL_PREFETCH_TOKEN_BUDGET_ENV, " "),
                (REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS_ENV, ""),
            ],
            ToolRetrievalMode::Hybrid,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(overridden.mode, ToolPrefetchMode::Semantic);
        assert_eq!(overridden.max_tools, 40);
        assert_eq!(overridden.token_budget, 12_000);
        assert_eq!(overridden.min_similarity, 0.5);
        assert_eq!(overridden.min_relative, 0.2);
        assert_eq!(overridden.context_messages, 32);
        assert_eq!(overridden.segment_bytes, 4_096);
        assert_eq!(overridden.always, vec!["trigger_create"]);
        assert_eq!(
            overridden.reselection,
            ToolReselectionSettings {
                enabled: true,
                cache_lifetime_secs: 600,
                cache_margin_secs: 30,
            }
        );

        // The environment can also switch selection off.
        assert_eq!(
            resolve(
                Some(FILE),
                &[(REBORN_TOOL_PREFETCH_ENV, "off")],
                ToolRetrievalMode::Native
            ),
            Ok(None)
        );
    }

    #[test]
    fn jev_is_chosen_by_file_or_environment_and_needs_its_key() {
        let with_key = [("MY_JEV_KEY", "jev-key")];
        let jev_file = FILE.replace(
            "mode = \"lexical\"",
            "mode = \"lexical\"\nclassifier = \"jev\"",
        );
        let settings = resolve(Some(&jev_file), &with_key, ToolRetrievalMode::Native)
            .expect("valid")
            .expect("on");
        assert_eq!(
            settings.classifier,
            ToolSelectionClassifierSettings::Jev(JevSettings {
                endpoint: "https://jev.example.test/api/v1/decisions".to_string(),
                model: "jev-1.12.0".to_string(),
                api_key_env: "MY_JEV_KEY".to_string(),
                timeout_ms: 900,
            })
        );

        // The environment override, with the compiled Jev defaults.
        let settings = resolve(
            None,
            &[
                (REBORN_TOOL_PREFETCH_ENV, "semantic"),
                (REBORN_TOOL_PREFETCH_CLASSIFIER_ENV, "JEV"),
                ("TYPESAFE_API_KEY", "jev-key"),
            ],
            // Jev does not rank, so semantic does not need a dense ranker.
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(
            settings.classifier,
            ToolSelectionClassifierSettings::Jev(JevSettings {
                endpoint: DEFAULT_JEV_ENDPOINT.to_string(),
                model: DEFAULT_JEV_MODEL.to_string(),
                api_key_env: DEFAULT_JEV_API_KEY_ENV.to_string(),
                timeout_ms: DEFAULT_JEV_TIMEOUT_MS,
            })
        );
        assert_eq!(DEFAULT_JEV_MODEL, "jev-latest");
        assert_eq!(DEFAULT_JEV_API_KEY_ENV, "TYPESAFE_API_KEY");
        assert_eq!(DEFAULT_JEV_ENDPOINT, "https://api.typesafe.ai/v1/systemone");

        // The endpoint's environment override wins over the file.
        let settings = resolve(
            Some(&jev_file),
            &[
                ("MY_JEV_KEY", "jev-key"),
                (
                    REBORN_TOOL_PREFETCH_JEV_ENDPOINT_ENV,
                    " https://decisions.example.test:8443/api/v1/systemone ",
                ),
            ],
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        let ToolSelectionClassifierSettings::Jev(jev) = settings.classifier else {
            panic!("jev");
        };
        assert_eq!(
            jev.endpoint,
            "https://decisions.example.test:8443/api/v1/systemone"
        );

        // So do the model's and the key variable's.
        let settings = resolve(
            Some(&jev_file),
            &[
                (REBORN_TOOL_PREFETCH_JEV_MODEL_ENV, " jev-1.13.0 "),
                (REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV_ENV, "OTHER_JEV_KEY"),
                ("OTHER_JEV_KEY", "jev-key"),
            ],
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(
            settings.classifier,
            ToolSelectionClassifierSettings::Jev(JevSettings {
                endpoint: "https://jev.example.test/api/v1/decisions".to_string(),
                model: "jev-1.13.0".to_string(),
                api_key_env: "OTHER_JEV_KEY".to_string(),
                timeout_ms: 900,
            })
        );
        // A key pasted where its variable's name belongs is refused, and
        // never repeated.
        let error = resolve(
            Some(&jev_file),
            &[(
                REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV_ENV,
                "sk-live-0123456789abcdef",
            )],
            ToolRetrievalMode::Native,
        )
        .expect_err("a key is not a name");
        assert_eq!(error, ToolPrefetchSettingsError::InvalidJevApiKeyEnv);
        assert!(!error.to_string().contains("sk-live"), "{error}");

        // The environment can switch a jev file back to local.
        let local = resolve(
            Some(&jev_file),
            &[(REBORN_TOOL_PREFETCH_CLASSIFIER_ENV, "local")],
            ToolRetrievalMode::Native,
        )
        .expect("valid")
        .expect("on");
        assert_eq!(local.classifier, ToolSelectionClassifierSettings::Local);

        for missing in [&[][..], &[("MY_JEV_KEY", "  ")][..]] {
            let error = resolve(Some(&jev_file), missing, ToolRetrievalMode::Native)
                .expect_err("no key, no start");
            assert_eq!(
                error,
                ToolPrefetchSettingsError::JevKeyMissing {
                    api_key_env: "MY_JEV_KEY".to_string()
                }
            );
            assert!(error.to_string().contains("MY_JEV_KEY"));
            assert!(error.to_string().contains("does not fall back"));
        }
    }

    #[test]
    fn a_jev_endpoint_that_is_not_a_plain_https_url_refuses_startup() {
        let jev = [
            (REBORN_TOOL_PREFETCH_ENV, "lexical"),
            (REBORN_TOOL_PREFETCH_CLASSIFIER_ENV, "jev"),
            ("TYPESAFE_API_KEY", "jev-key"),
        ];
        for (endpoint, reason) in [
            (
                "http://jev.example.test/v1/systemone",
                "it does not use https",
            ),
            ("jev.example.test/v1/systemone", "it is not a valid URL"),
            ("https://", "it has no host"),
            ("https://:8443/v1/systemone", "it has no host"),
            (
                "https://user:hunter2@jev.example.test/v1/systemone",
                "it carries userinfo",
            ),
            (
                "https://jev.example.test/v1/systemone?key=1",
                "it carries a query",
            ),
            (
                "https://jev.example.test/v1/systemone#top",
                "it carries a fragment",
            ),
            (
                "https://*.example.test/v1/systemone",
                "its host is a wildcard",
            ),
        ] {
            let mut vars = jev.to_vec();
            vars.push((REBORN_TOOL_PREFETCH_JEV_ENDPOINT_ENV, endpoint));
            let error = read(&vars, ToolRetrievalMode::Native).expect_err(endpoint);
            assert_eq!(
                error,
                ToolPrefetchSettingsError::InvalidJevEndpoint { reason },
                "{endpoint}"
            );
            let message = error.to_string();
            assert!(
                message.contains("[tool_selection.jev] endpoint"),
                "{message}"
            );
            assert!(message.contains(reason), "{message}");
            assert!(!message.contains("hunter2"), "{message}");
        }
        // The file is checked the same way.
        let file = "[tool_selection]\nmode = \"lexical\"\nclassifier = \"jev\"\n\
                    [tool_selection.jev]\nendpoint = \"http://jev.example.test/v1\"\n";
        assert_eq!(
            resolve(Some(file), &jev[2..], ToolRetrievalMode::Native),
            Err(ToolPrefetchSettingsError::InvalidJevEndpoint {
                reason: "it does not use https"
            })
        );
    }

    #[test]
    fn a_jev_table_under_the_local_classifier_is_inert() {
        // FILE carries a [tool_selection.jev] table naming a key that is not
        // set; with the local classifier it is never read.
        let settings = resolve(Some(FILE), &[], ToolRetrievalMode::Native)
            .expect("the jev table is inert")
            .expect("on");
        assert_eq!(settings.classifier, ToolSelectionClassifierSettings::Local);
    }

    #[test]
    fn unknown_classifiers_and_out_of_range_values_refuse_startup() {
        let error = read(
            &[
                (REBORN_TOOL_PREFETCH_ENV, "lexical"),
                (REBORN_TOOL_PREFETCH_CLASSIFIER_ENV, "gpt"),
            ],
            ToolRetrievalMode::Native,
        )
        .expect_err("unknown classifier");
        assert!(matches!(
            error,
            ToolPrefetchSettingsError::UnknownClassifier { .. }
        ));
        assert!(error.to_string().contains("`local` or `jev`"), "{error}");

        for (file, what) in [
            (
                "[tool_selection]\nmode = \"lexical\"\nmax_tools = 0\n",
                "max_tools = 0",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\ntoken_budget = 0\n",
                "token_budget = 0",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\nmax_tools = 3\n",
                "max_tools below the floor",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\n[tool_selection.local]\nmin_relative = 1.5\n",
                "min_relative above 1",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\ncontext_messages = 0\n",
                "context_messages = 0",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\nsegment_bytes = 100000\n",
                "segment_bytes above the maximum",
            ),
            (
                "[tool_selection]\nmode = \"lexical\"\nclassifier = \"jev\"\n[tool_selection.jev]\ntimeout_ms = 0\n",
                "timeout_ms = 0",
            ),
        ] {
            let error = resolve(
                Some(file),
                &[("TYPESAFE_API_KEY", "jev-key")],
                ToolRetrievalMode::Native,
            )
            .expect_err(what);
            assert!(!error.to_string().is_empty(), "{what}");
        }
    }
}
