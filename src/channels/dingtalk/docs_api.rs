//! DingTalk Document API — CRUD operations for DingTalk Docs (P2 scaffolding).

use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::error::ChannelError;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsCreateResponse {
    pub document_id: Option<String>,
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsAppendResponse {
    pub partial_success: Option<bool>,
    pub append_error: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsSearchResult {
    pub document_id: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsListItem {
    pub document_id: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

fn send_failed(reason: impl Into<String>) -> ChannelError {
    ChannelError::SendFailed {
        name: "dingtalk".to_string(),
        reason: reason.into(),
    }
}

// ---------------------------------------------------------------------------
// Public API functions
// ---------------------------------------------------------------------------

/// Create a new DingTalk document.
///
/// POST `https://api.dingtalk.com/v1.0/doc/documents`
pub async fn docs_create(
    client: &Client,
    token: &str,
    title: &str,
    parent_id: Option<&str>,
) -> Result<DocsCreateResponse, ChannelError> {
    let mut body = serde_json::json!({ "title": title });
    if let Some(pid) = parent_id {
        body["parentId"] = serde_json::Value::String(pid.to_string());
    }

    debug!(title = title, parent_id = ?parent_id, "docs_create: creating DingTalk document");

    let resp = client
        .post("https://api.dingtalk.com/v1.0/doc/documents")
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| send_failed(format!("request error: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(send_failed(format!(
            "docs_create returned {status}: {text}"
        )));
    }

    let result: DocsCreateResponse = resp
        .json()
        .await
        .map_err(|e| send_failed(format!("parse docs_create response: {e}")))?;

    debug!(document_id = ?result.document_id, "docs_create: document created");
    Ok(result)
}

/// Append content blocks to an existing DingTalk document.
///
/// POST `https://api.dingtalk.com/v1.0/doc/documents/{document_id}/blocks/batch`
pub async fn docs_append(
    client: &Client,
    token: &str,
    document_id: &str,
    content: &str,
) -> Result<DocsAppendResponse, ChannelError> {
    let url = format!(
        "https://api.dingtalk.com/v1.0/doc/documents/{}/blocks/batch",
        document_id
    );

    let body = serde_json::json!({
        "blocks": [
            {
                "blockType": "paragraph",
                "paragraph": {
                    "elements": [
                        {
                            "type": "text",
                            "textRun": {
                                "text": content
                            }
                        }
                    ]
                }
            }
        ]
    });

    debug!(
        document_id = document_id,
        content_len = content.len(),
        "docs_append: appending to document"
    );

    let resp = client
        .post(&url)
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| send_failed(format!("request error: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(send_failed(format!(
            "docs_append returned {status}: {text}"
        )));
    }

    let result: DocsAppendResponse = resp
        .json()
        .await
        .map_err(|e| send_failed(format!("parse docs_append response: {e}")))?;

    debug!(partial_success = ?result.partial_success, "docs_append: blocks appended");
    Ok(result)
}

/// Search DingTalk documents by keyword.
///
/// POST `https://api.dingtalk.com/v1.0/doc/documents/search`
pub async fn docs_search(
    client: &Client,
    token: &str,
    query: &str,
) -> Result<Vec<DocsSearchResult>, ChannelError> {
    let body = serde_json::json!({ "keyword": query });

    debug!(query = query, "docs_search: searching documents");

    let resp = client
        .post("https://api.dingtalk.com/v1.0/doc/documents/search")
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| send_failed(format!("request error: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(send_failed(format!(
            "docs_search returned {status}: {text}"
        )));
    }

    // The API may wrap results in a top-level object; try both a direct array
    // and a `result` / `documents` field for forward compatibility.
    let raw: Value = resp
        .json()
        .await
        .map_err(|e| send_failed(format!("parse docs_search response: {e}")))?;

    let items_value = if raw.is_array() {
        raw
    } else {
        raw.get("result")
            .or_else(|| raw.get("documents"))
            .cloned()
            .unwrap_or(Value::Array(vec![]))
    };

    let results: Vec<DocsSearchResult> = serde_json::from_value(items_value)
        .map_err(|e| send_failed(format!("deserialize docs_search results: {e}")))?;

    debug!(count = results.len(), "docs_search: found documents");
    Ok(results)
}

/// List all DingTalk documents accessible to the token.
///
/// GET `https://api.dingtalk.com/v1.0/doc/documents`
pub async fn docs_list(client: &Client, token: &str) -> Result<Vec<DocsListItem>, ChannelError> {
    debug!("docs_list: listing documents");

    let resp = client
        .get("https://api.dingtalk.com/v1.0/doc/documents")
        .header("x-acs-dingtalk-access-token", token)
        .send()
        .await
        .map_err(|e| send_failed(format!("request error: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(send_failed(format!("docs_list returned {status}: {text}")));
    }

    // Same envelope handling as docs_search.
    let raw: Value = resp
        .json()
        .await
        .map_err(|e| send_failed(format!("parse docs_list response: {e}")))?;

    let items_value = if raw.is_array() {
        raw
    } else {
        raw.get("result")
            .or_else(|| raw.get("documents"))
            .cloned()
            .unwrap_or(Value::Array(vec![]))
    };

    let items: Vec<DocsListItem> = serde_json::from_value(items_value)
        .map_err(|e| send_failed(format!("deserialize docs_list items: {e}")))?;

    debug!(count = items.len(), "docs_list: retrieved documents");
    Ok(items)
}
