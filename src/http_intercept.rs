use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::llm::recording::{HttpExchangeRequest, HttpExchangeResponse, HttpInterceptor};

#[derive(Debug)]
pub struct CompositeHttpInterceptor {
    interceptors: Vec<Arc<dyn HttpInterceptor>>,
}

impl CompositeHttpInterceptor {
    pub fn new(interceptors: Vec<Arc<dyn HttpInterceptor>>) -> Self {
        Self { interceptors }
    }
}

#[async_trait]
impl HttpInterceptor for CompositeHttpInterceptor {
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        for interceptor in &self.interceptors {
            if let Some(response) = interceptor.before_request(request).await {
                for recorder in &self.interceptors {
                    recorder.after_response(request, &response).await;
                }
                return Some(response);
            }
        }
        None
    }

    async fn after_response(&self, request: &HttpExchangeRequest, response: &HttpExchangeResponse) {
        for interceptor in &self.interceptors {
            interceptor.after_response(request, response).await;
        }
    }
}

#[derive(Debug)]
struct HostRemapHttpInterceptor {
    mappings: HashMap<String, String>,
    client: reqwest::Client,
}

impl HostRemapHttpInterceptor {
    fn from_env() -> Option<Self> {
        let raw = std::env::var("IRONCLAW_TEST_HTTP_REMAP").ok()?;
        let mappings = raw
            .split(',')
            .filter_map(|entry| {
                let (host, base_url) = entry.split_once('=')?;
                let host = host.trim().to_lowercase();
                let base_url = base_url.trim().trim_end_matches('/').to_string();
                if host.is_empty() || base_url.is_empty() {
                    return None;
                }
                Some((host, base_url))
            })
            .collect::<HashMap<_, _>>();
        if mappings.is_empty() {
            return None;
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Self { mappings, client })
    }

    fn rewrite_url(&self, url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        let base = self.mappings.get(&host)?;
        let mut rewritten = format!("{base}{}", parsed.path());
        if let Some(query) = parsed.query() {
            rewritten.push('?');
            rewritten.push_str(query);
        }
        Some(rewritten)
    }
}

#[async_trait]
impl HttpInterceptor for HostRemapHttpInterceptor {
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        let rewritten_url = self.rewrite_url(&request.url)?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes()).ok()?;
        let mut builder = self.client.request(method, rewritten_url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let response = builder.send().await.ok()?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_string()))
            })
            .collect();
        let body = response.text().await.ok()?;
        Some(HttpExchangeResponse {
            status,
            headers,
            body,
        })
    }

    async fn after_response(
        &self,
        _request: &HttpExchangeRequest,
        _response: &HttpExchangeResponse,
    ) {
    }
}

pub fn remap_from_env() -> Option<Arc<dyn HttpInterceptor>> {
    HostRemapHttpInterceptor::from_env().map(|interceptor| Arc::new(interceptor) as Arc<_>)
}

pub fn chain(
    interceptors: impl IntoIterator<Item = Arc<dyn HttpInterceptor>>,
) -> Option<Arc<dyn HttpInterceptor>> {
    let interceptors = interceptors.into_iter().collect::<Vec<_>>();
    match interceptors.len() {
        0 => None,
        1 => interceptors.into_iter().next(),
        _ => Some(Arc::new(CompositeHttpInterceptor::new(interceptors))),
    }
}
