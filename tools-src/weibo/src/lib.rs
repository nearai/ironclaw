//! Weibo Social Media WASM Tool for IronClaw.
//!
//! Read timelines and search topics on Weibo (微博).
//! Supports home timeline, user timeline, and topic search.
//!
//! # Authentication
//!
//! Store your Weibo access token:
//! `ironclaw secret set weibo_access_token <token>`
//!
//! Get a token at: https://open.weibo.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://api.weibo.com/2";
const MAX_RETRIES: u32 = 3;

struct WeiboTool;

impl exports::near::agent::tool::Guest for WeiboTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Read Weibo timelines and search topics (微博). \
         View home timeline, user timeline, and search trending topics. \
         Authentication is handled via the 'weibo_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    uid: Option<String>,
    query: Option<String>,
    count: Option<u32>,
}

// --- Weibo API response types ---

#[derive(Debug, Deserialize)]
struct TimelineResponse {
    statuses: Option<Vec<StatusItem>>,
    total_number: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StatusItem {
    id: Option<u64>,
    text: Option<String>,
    user: Option<WeiboUser>,
    created_at: Option<String>,
    reposts_count: Option<u32>,
    comments_count: Option<u32>,
    attitudes_count: Option<u32>,
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeiboUser {
    screen_name: Option<String>,
    followers_count: Option<u32>,
    verified: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TopicSearchResponse {
    statuses: Option<Vec<StatusItem>>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("weibo_access_token") {
        return Err(
            "Weibo access token not found in secret store. Set it with: \
             ironclaw secret set weibo_access_token <token>. \
             Get a token at: https://open.weibo.com/"
                .into(),
        );
    }

    match params.action.as_str() {
        "home_timeline" => home_timeline(&params),
        "user_timeline" => user_timeline(&params),
        "search_topics" => search_topics(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: home_timeline, user_timeline, search_topics",
            params.action
        )),
    }
}

fn home_timeline(params: &Params) -> Result<String, String> {
    let count = params.count.unwrap_or(20).clamp(1, 100);
    let url = format!("{BASE_URL}/statuses/home_timeline.json?count={count}");
    let resp_body = weibo_request("GET", &url)?;

    let resp: TimelineResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let statuses = resp.statuses.unwrap_or_default();
    let formatted = format_statuses(statuses);

    let output = serde_json::json!({
        "action": "home_timeline",
        "total_number": resp.total_number.unwrap_or(0),
        "result_count": formatted.len(),
        "statuses": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn user_timeline(params: &Params) -> Result<String, String> {
    let uid = params.uid.as_deref().ok_or("'uid' is required for user_timeline")?;

    if uid.is_empty() {
        return Err("'uid' must not be empty".into());
    }

    let count = params.count.unwrap_or(20).clamp(1, 100);
    let url = format!("{BASE_URL}/statuses/user_timeline.json?uid={uid}&count={count}");
    let resp_body = weibo_request("GET", &url)?;

    let resp: TimelineResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let statuses = resp.statuses.unwrap_or_default();
    let formatted = format_statuses(statuses);

    let output = serde_json::json!({
        "action": "user_timeline",
        "uid": uid,
        "total_number": resp.total_number.unwrap_or(0),
        "result_count": formatted.len(),
        "statuses": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn search_topics(params: &Params) -> Result<String, String> {
    let query = params.query.as_deref().ok_or("'query' is required for search_topics")?;

    if query.is_empty() {
        return Err("'query' must not be empty".into());
    }

    let count = params.count.unwrap_or(20).clamp(1, 50);
    let encoded_query = simple_url_encode(query);
    let url = format!("{BASE_URL}/search/topics.json?q={encoded_query}&count={count}");
    let resp_body = weibo_request("GET", &url)?;

    let resp: TopicSearchResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    let statuses = resp.statuses.unwrap_or_default();
    let formatted = format_statuses(statuses);

    let output = serde_json::json!({
        "action": "search_topics",
        "query": query,
        "result_count": formatted.len(),
        "statuses": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn format_statuses(statuses: Vec<StatusItem>) -> Vec<serde_json::Value> {
    statuses
        .into_iter()
        .filter_map(|s| {
            let text = s.text?;
            let mut entry = serde_json::json!({"text": text});
            if let Some(id) = s.id {
                entry["id"] = serde_json::json!(id);
            }
            if let Some(user) = s.user {
                if let Some(name) = user.screen_name {
                    entry["screen_name"] = serde_json::json!(name);
                }
                if let Some(verified) = user.verified {
                    entry["verified"] = serde_json::json!(verified);
                }
            }
            if let Some(created) = s.created_at {
                entry["created_at"] = serde_json::json!(created);
            }
            if let Some(reposts) = s.reposts_count {
                entry["reposts_count"] = serde_json::json!(reposts);
            }
            if let Some(comments) = s.comments_count {
                entry["comments_count"] = serde_json::json!(comments);
            }
            if let Some(attitudes) = s.attitudes_count {
                entry["attitudes_count"] = serde_json::json!(attitudes);
            }
            Some(entry)
        })
        .collect()
}

fn weibo_request(method: &str, url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-Weibo-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            None,
            None,
        )
        .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Weibo API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Weibo API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

/// Simple URL encoding for query parameters.
fn simple_url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'home_timeline' (首页时间线), 'user_timeline' (用户时间线), 'search_topics' (搜索话题)",
            "enum": ["home_timeline", "user_timeline", "search_topics"]
        },
        "uid": {
            "type": "string",
            "description": "User ID (required for user_timeline)"
        },
        "query": {
            "type": "string",
            "description": "Search query (required for search_topics)"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-100, default 20)",
            "minimum": 1,
            "maximum": 100,
            "default": 20
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(WeiboTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timeline_response() {
        let json = r#"{
            "statuses": [
                {
                    "id": 123456789,
                    "text": "今天天气真好",
                    "user": {
                        "screen_name": "测试用户",
                        "followers_count": 1000,
                        "verified": true
                    },
                    "created_at": "Mon Jan 01 12:00:00 +0800 2025",
                    "reposts_count": 10,
                    "comments_count": 5,
                    "attitudes_count": 20,
                    "source": "微博 weibo.com"
                }
            ],
            "total_number": 100
        }"#;
        let resp: TimelineResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total_number, Some(100));
        let statuses = resp.statuses.unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].text.as_deref(), Some("今天天气真好"));
        assert_eq!(statuses[0].id, Some(123456789));
        assert_eq!(statuses[0].reposts_count, Some(10));
        let user = statuses[0].user.as_ref().unwrap();
        assert_eq!(user.screen_name.as_deref(), Some("测试用户"));
        assert_eq!(user.verified, Some(true));
    }

    #[test]
    fn test_parse_empty_timeline() {
        let json = r#"{"statuses": [], "total_number": 0}"#;
        let resp: TimelineResponse = serde_json::from_str(json).unwrap();
        assert!(resp.statuses.unwrap().is_empty());
        assert_eq!(resp.total_number, Some(0));
    }

    #[test]
    fn test_parse_topic_search_response() {
        let json = r##"{
            "statuses": [
                {
                    "id": 987654321,
                    "text": "#热门话题# 讨论内容",
                    "user": {"screen_name": "话题达人", "followers_count": 500, "verified": false},
                    "created_at": "Tue Jan 02 10:00:00 +0800 2025",
                    "reposts_count": 100,
                    "comments_count": 50,
                    "attitudes_count": 200
                }
            ]
        }"##;
        let resp: TopicSearchResponse = serde_json::from_str(json).unwrap();
        let statuses = resp.statuses.unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].reposts_count, Some(100));
    }

    #[test]
    fn test_format_statuses() {
        let statuses = vec![StatusItem {
            id: Some(1),
            text: Some("Hello".into()),
            user: Some(WeiboUser {
                screen_name: Some("User".into()),
                followers_count: Some(100),
                verified: Some(false),
            }),
            created_at: Some("2025-01-01".into()),
            reposts_count: Some(5),
            comments_count: Some(3),
            attitudes_count: Some(10),
            source: None,
        }];
        let formatted = format_statuses(statuses);
        assert_eq!(formatted.len(), 1);
        assert_eq!(formatted[0]["text"], "Hello");
        assert_eq!(formatted[0]["screen_name"], "User");
    }

    #[test]
    fn test_simple_url_encode() {
        assert_eq!(simple_url_encode("hello world"), "hello%20world");
        assert_eq!(simple_url_encode("test"), "test");
    }
}
