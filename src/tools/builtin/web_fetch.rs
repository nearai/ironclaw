//! Fetch a web page and summarize it with a secondary LLM.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::context::JobContext;
use crate::db::UserStore;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::secrets::SecretsStore;
use crate::tools::builtin::{HttpTool, extract_host_from_params};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::tools::wasm::SharedCredentialRegistry;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_CACHE_ENTRIES: usize = 128;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 800;
const MAX_OUTPUT_TOKENS: u32 = 2000;
const MAX_SOURCE_CHARS: usize = 80_000;
const TRUSTED_DOMAINS: &[&str] = &[
    "github.com",
    "docs.github.com",
    "raw.githubusercontent.com",
    "developer.mozilla.org",
    "docs.rs",
    "crates.io",
    "wikipedia.org",
];

#[derive(Clone)]
struct CachedPage {
    status: u16,
    headers: serde_json::Value,
    content: String,
    created_at: Instant,
}

#[derive(Clone)]
struct CachedSummary {
    output: serde_json::Value,
    raw: String,
    created_at: Instant,
}

#[derive(Default)]
struct WebFetchCache {
    pages: HashMap<String, CachedPage>,
    summaries: HashMap<String, CachedSummary>,
}

/// Tool that fetches a URL, converts web pages to text via `http`, and returns
/// a focused secondary-model summary instead of the raw page body.
pub struct WebFetchTool {
    http: HttpTool,
    llm: Arc<dyn LlmProvider>,
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    cache: Mutex<WebFetchCache>,
}

impl WebFetchTool {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            http: HttpTool::new(),
            llm,
            credential_registry: None,
            cache: Mutex::new(WebFetchCache::default()),
        }
    }

    pub fn with_credentials(
        mut self,
        registry: Arc<SharedCredentialRegistry>,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.http = self
            .http
            .with_credentials(Arc::clone(&registry), secrets_store);
        self.credential_registry = Some(registry);
        self
    }

    pub fn with_role_lookup(mut self, role_lookup: Arc<dyn UserStore>) -> Self {
        self.http = self.http.with_role_lookup(role_lookup);
        self
    }

    async fn fetch_page(
        &self,
        url: &str,
        timeout_secs: Option<u64>,
        ctx: &JobContext,
    ) -> Result<(CachedPage, bool), ToolError> {
        let page_key = page_cache_key(&ctx.user_id, url, timeout_secs);
        if let Some(page) = self.get_cached_page(&page_key) {
            return Ok((page, true));
        }

        let mut params = serde_json::json!({
            "method": "GET",
            "url": url,
        });
        if let Some(timeout_secs) = timeout_secs {
            params["timeout_secs"] = serde_json::json!(timeout_secs);
        }

        let output = self.http.execute(params, ctx).await?;
        let result = output.result;
        let status = result
            .get("status")
            .and_then(|v| v.as_u64())
            .and_then(|s| u16::try_from(s).ok())
            .unwrap_or(0);
        let headers = result
            .get("headers")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let content = output.raw.unwrap_or_else(|| {
            result
                .get("body")
                .map(body_to_text)
                .unwrap_or_else(String::new)
        });

        let page = CachedPage {
            status,
            headers,
            content,
            created_at: Instant::now(),
        };
        self.store_page(page_key, page.clone());
        Ok((page, false))
    }

    fn get_cached_page(&self, key: &str) -> Option<CachedPage> {
        let now = Instant::now();
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .pages
            .get(key)
            .filter(|entry| now.duration_since(entry.created_at) < CACHE_TTL)
            .cloned()
    }

    fn store_page(&self, key: String, page: CachedPage) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut cache);
        if cache.pages.len() >= MAX_CACHE_ENTRIES
            && let Some(oldest) = oldest_key(&cache.pages, |entry| entry.created_at)
        {
            cache.pages.remove(&oldest);
        }
        cache.pages.insert(key, page);
    }

    fn get_cached_summary(&self, key: &str) -> Option<CachedSummary> {
        let now = Instant::now();
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .summaries
            .get(key)
            .filter(|entry| now.duration_since(entry.created_at) < CACHE_TTL)
            .cloned()
    }

    fn store_summary(&self, key: String, summary: CachedSummary) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut cache);
        if cache.summaries.len() >= MAX_CACHE_ENTRIES
            && let Some(oldest) = oldest_key(&cache.summaries, |entry| entry.created_at)
        {
            cache.summaries.remove(&oldest);
        }
        cache.summaries.insert(key, summary);
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page and return a concise secondary-model summary relevant to a prompt. \
         Use this for articles, documentation, and web pages when raw page content would waste context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTPS URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "What to extract, answer, or summarize from the fetched page"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 30, max: 300)"
                },
                "max_output_tokens": {
                    "type": "integer",
                    "description": "Maximum tokens for the summary (default: 800, max: 2000)"
                }
            },
            "required": ["url", "prompt"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let url = require_str(&params, "url")?;
        let prompt = require_str(&params, "prompt")?;
        let timeout_secs = parse_optional_u64(&params, "timeout_secs")?;
        let max_output_tokens = parse_output_tokens(&params)?;
        let trusted = is_trusted_domain(url);
        let summary_key = summary_cache_key(
            &ctx.user_id,
            url,
            prompt,
            timeout_secs,
            max_output_tokens,
            trusted,
        );

        if let Some(summary) = self.get_cached_summary(&summary_key) {
            let mut result = summary.output;
            result["cached"] = serde_json::json!(true);
            return Ok(ToolOutput::success(result, start.elapsed()).with_raw(summary.raw));
        }

        let (page, page_cached) = self.fetch_page(url, timeout_secs, ctx).await?;
        let source = truncate_source(&page.content);
        let system = include_str!("web_fetch_prompt.md");
        let copyright_mode = if trusted {
            "trusted domain: answer may include necessary short excerpts, but prefer paraphrase"
        } else {
            "untrusted domain: paraphrase; do not include more than 25 quoted words total"
        };
        let user = format!(
            "URL: {url}\nHTTP status: {status}\nCopyright mode: {copyright_mode}\nUser prompt: {prompt}\n\nFetched page text:\n{source}",
            status = page.status
        );
        let request =
            CompletionRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)])
                .with_max_tokens(max_output_tokens)
                .with_temperature(0.2);
        let response = self.llm.complete(request).await.map_err(|e| {
            ToolError::ExternalService(format!("web_fetch summarization failed: {}", e))
        })?;

        let summary = enforce_quote_limit(response.content.trim(), trusted);
        let content_type = content_type_from_headers(&page.headers);
        let result = serde_json::json!({
            "url": url,
            "status": page.status,
            "summary": summary,
            "cached": false,
            "page_cached": page_cached,
            "trusted_domain": trusted,
            "model": self.llm.active_model_name(),
            "content_type": content_type,
        });
        self.store_summary(
            summary_key,
            CachedSummary {
                output: result.clone(),
                raw: summary.clone(),
                created_at: Instant::now(),
            },
        );

        Ok(ToolOutput::success(result, start.elapsed()).with_raw(summary))
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(8))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let has_credentials = ironclaw_safety::params_contain_manual_credentials(params)
            || (self.credential_registry.as_ref().is_some_and(|registry| {
                extract_host_from_params(params)
                    .is_some_and(|host| registry.has_credentials_for_host(&host))
            }));

        if has_credentials {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 300))
    }
}

fn parse_optional_u64(params: &serde_json::Value, name: &str) -> Result<Option<u64>, ToolError> {
    match params.get(name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
            ToolError::InvalidParameters(format!("{name} must be a non-negative integer"))
        }),
        Some(_) => Err(ToolError::InvalidParameters(format!(
            "{name} must be an integer"
        ))),
    }
}

fn parse_output_tokens(params: &serde_json::Value) -> Result<u32, ToolError> {
    let tokens = parse_optional_u64(params, "max_output_tokens")?
        .unwrap_or(u64::from(DEFAULT_MAX_OUTPUT_TOKENS));
    if tokens == 0 || tokens > u64::from(MAX_OUTPUT_TOKENS) {
        return Err(ToolError::InvalidParameters(format!(
            "max_output_tokens must be between 1 and {MAX_OUTPUT_TOKENS}"
        )));
    }
    u32::try_from(tokens)
        .map_err(|_| ToolError::InvalidParameters("max_output_tokens is too large".to_string()))
}

fn body_to_text(body: &serde_json::Value) -> String {
    body.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string())
}

fn content_type_from_headers(headers: &serde_json::Value) -> Option<String> {
    headers.as_object().and_then(|map| {
        map.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .and_then(|(_, value)| value.as_str().map(str::to_string))
    })
}

fn truncate_source(content: &str) -> String {
    if content.len() <= MAX_SOURCE_CHARS {
        return content.to_string();
    }

    let mut end = MAX_SOURCE_CHARS.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[Content truncated before summarization.]",
        &content[..end]
    )
}

fn is_trusted_domain(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            TRUSTED_DOMAINS
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        })
}

fn enforce_quote_limit(summary: &str, trusted: bool) -> String {
    if trusted {
        return summary.to_string();
    }

    let mut quoted_words = 0usize;
    let mut in_quote = false;
    let mut out = String::with_capacity(summary.len());
    for ch in summary.chars() {
        if ch == '"' {
            in_quote = !in_quote;
            out.push(ch);
            continue;
        }
        if in_quote && !ch.is_whitespace() {
            let starts_word = out
                .chars()
                .last()
                .is_none_or(|prev| prev.is_whitespace() || prev == '"');
            if starts_word {
                quoted_words += 1;
            }
        }
        if in_quote && quoted_words > 25 {
            continue;
        }
        out.push(ch);
    }
    if quoted_words > 25 {
        out.push_str(" [quote truncated]");
    }
    out
}

fn page_cache_key(user_id: &str, url: &str, timeout_secs: Option<u64>) -> String {
    hash_parts(&["page", user_id, url, &timeout_secs.unwrap_or(0).to_string()])
}

fn summary_cache_key(
    user_id: &str,
    url: &str,
    prompt: &str,
    timeout_secs: Option<u64>,
    max_output_tokens: u32,
    trusted: bool,
) -> String {
    hash_parts(&[
        "summary",
        user_id,
        url,
        prompt,
        &timeout_secs.unwrap_or(0).to_string(),
        &max_output_tokens.to_string(),
        if trusted { "trusted" } else { "restricted" },
    ])
}

fn hash_parts(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn prune_expired(cache: &mut WebFetchCache) {
    let now = Instant::now();
    cache
        .pages
        .retain(|_, entry| now.duration_since(entry.created_at) < CACHE_TTL);
    cache
        .summaries
        .retain(|_, entry| now.duration_since(entry.created_at) < CACHE_TTL);
}

fn oldest_key<T, F>(map: &HashMap<String, T>, created_at: F) -> Option<String>
where
    F: Fn(&T) -> Instant,
{
    map.iter()
        .min_by_key(|(_, entry)| created_at(entry))
        .map(|(key, _)| key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{
        CompletionResponse, FinishReason, ModelMetadata, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use async_trait::async_trait;
    use rust_decimal::Decimal;

    struct StaticLlm {
        response: String,
    }

    #[async_trait]
    impl LlmProvider for StaticLlm {
        fn model_name(&self) -> &str {
            "test-summary-model"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            Ok(CompletionResponse {
                content: self.response.clone(),
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            Err(crate::error::LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "not used".to_string(),
            })
        }

        async fn model_metadata(&self) -> Result<ModelMetadata, crate::error::LlmError> {
            Ok(ModelMetadata {
                id: self.model_name().to_string(),
                context_length: None,
            })
        }
    }

    #[test]
    fn schema_requires_url_and_prompt() {
        let tool = WebFetchTool::new(Arc::new(StaticLlm {
            response: "summary".to_string(),
        }));
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], serde_json::json!(["url", "prompt"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn non_trusted_quote_limit_truncates_long_quotes() {
        let summary = "\"one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone twentytwo twentythree twentyfour twentyfive twentysix twentyseven\"";
        let limited = enforce_quote_limit(summary, false);
        assert!(limited.contains("[quote truncated]"));
        assert!(!limited.contains("twentysix"));
    }

    #[test]
    fn cache_keys_are_user_scoped() {
        let a = summary_cache_key("user-a", "https://example.com", "prompt", None, 800, false);
        let b = summary_cache_key("user-b", "https://example.com", "prompt", None, 800, false);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn execute_validates_output_token_limit_before_fetch() {
        let tool = WebFetchTool::new(Arc::new(StaticLlm {
            response: "Useful page summary".to_string(),
        }));
        let ctx = JobContext::with_user("test-user", "test-job", "test");
        let params = serde_json::json!({
            "url": "https://example.com/page",
            "prompt": "summarize",
            "max_output_tokens": 2001
        });

        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("max_output_tokens"));
    }
}
