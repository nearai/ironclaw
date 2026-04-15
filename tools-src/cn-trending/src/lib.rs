//! Chinese Trending Topics Aggregator WASM Tool for IronClaw.
//!
//! Fetches trending topics from Chinese social platforms including
//! Zhihu, Weibo, Baidu, Douyin, and Bilibili using free public APIs.
//!
//! No authentication required.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const VVHAN_BASE: &str = "https://api.vvhan.com/api/hotlist";
const TENAPI_BASE: &str = "https://tenapi.cn/v2";
const MAX_RETRIES: u32 = 3;

const VALID_PLATFORMS: &[&str] = &["zhihu", "weibo", "baidu", "douyin", "bilibili"];

struct CnTrendingTool;

impl exports::near::agent::tool::Guest for CnTrendingTool {
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
        "国内热搜聚合 — 知乎、微博、百度、抖音、B站多平台热门话题。\
         No authentication required — uses free public APIs."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    platform: Option<String>,
}

// --- vvhan response ---
#[derive(Debug, Deserialize)]
struct VvhanResponse {
    success: Option<bool>,
    data: Option<Vec<VvhanItem>>,
}

#[derive(Debug, Deserialize)]
struct VvhanItem {
    title: Option<String>,
    url: Option<String>,
    hot: Option<serde_json::Value>,
    desc: Option<String>,
    index: Option<u32>,
}

// --- tenapi fallback response ---
#[derive(Debug, Deserialize)]
struct TenapiResponse {
    code: Option<i32>,
    data: Option<TenapiData>,
}

#[derive(Debug, Deserialize)]
struct TenapiData {
    list: Option<Vec<TenapiItem>>,
}

#[derive(Debug, Deserialize)]
struct TenapiItem {
    name: Option<String>,
    url: Option<String>,
    hot: Option<serde_json::Value>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    match params.action.as_str() {
        "get_trending" => action_get_trending(&params),
        other => Err(format!(
            "Unknown action '{other}'. Valid actions: get_trending"
        )),
    }
}

fn action_get_trending(params: &Params) -> Result<String, String> {
    let platform = params
        .platform
        .as_deref()
        .ok_or("'platform' is required for get_trending action")?;

    if !VALID_PLATFORMS.contains(&platform) {
        return Err(format!(
            "Invalid 'platform': expected one of {:?}, got '{platform}'",
            VALID_PLATFORMS
        ));
    }

    // Try primary API (vvhan)
    let vvhan_url = format!("{VVHAN_BASE}/{platform}");
    match try_vvhan(&vvhan_url, platform) {
        Ok(result) => return Ok(result),
        Err(primary_err) => {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!("vvhan API failed: {primary_err}. Trying tenapi fallback..."),
            );
        }
    }

    // Fallback to tenapi
    let tenapi_url = format!("{TENAPI_BASE}/{platform}hot");
    try_tenapi(&tenapi_url, platform)
}

fn try_vvhan(url: &str, platform: &str) -> Result<String, String> {
    let body = do_get(url)?;
    let resp: VvhanResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse vvhan response: {e}"))?;

    if resp.success != Some(true) {
        return Err("vvhan API returned unsuccessful response".into());
    }

    let items = resp.data.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = items
        .into_iter()
        .filter_map(|item| {
            let title = item.title?;
            let mut entry = serde_json::json!({"title": title});
            if let Some(url) = item.url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(hot) = item.hot {
                entry["hot"] = hot;
            }
            if let Some(desc) = item.desc {
                if !desc.is_empty() {
                    entry["desc"] = serde_json::json!(desc);
                }
            }
            if let Some(index) = item.index {
                entry["rank"] = serde_json::json!(index);
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "get_trending",
        "platform": platform,
        "source": "vvhan",
        "result_count": formatted.len(),
        "items": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn try_tenapi(url: &str, platform: &str) -> Result<String, String> {
    let body = do_get(url)?;
    let resp: TenapiResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse tenapi response: {e}"))?;

    if resp.code != Some(200) {
        return Err(format!(
            "tenapi API error (code {:?})",
            resp.code
        ));
    }

    let items = resp
        .data
        .and_then(|d| d.list)
        .unwrap_or_default();

    let formatted: Vec<serde_json::Value> = items
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let name = item.name?;
            let mut entry = serde_json::json!({
                "title": name,
                "rank": i + 1,
            });
            if let Some(url) = item.url {
                entry["url"] = serde_json::json!(url);
            }
            if let Some(hot) = item.hot {
                entry["hot"] = hot;
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "get_trending",
        "platform": platform,
        "source": "tenapi",
        "result_count": formatted.len(),
        "items": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn do_get(url: &str) -> Result<String, String> {
    let headers = serde_json::json!({
        "User-Agent": "IronClaw-CnTrending-Tool/0.1",
        "Accept": "application/json"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp =
            near::agent::host::http_request("GET", url, &headers.to_string(), None, None)
                .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Trending API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!("Trending API error (HTTP {}): {}", resp.status, body));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform",
            "enum": ["get_trending"]
        },
        "platform": {
            "type": "string",
            "description": "The platform to fetch trending topics from",
            "enum": ["zhihu", "weibo", "baidu", "douyin", "bilibili"]
        }
    },
    "required": ["action", "platform"],
    "additionalProperties": false
}"#;

export!(CnTrendingTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_platforms() {
        for p in VALID_PLATFORMS {
            assert!(VALID_PLATFORMS.contains(p));
        }
        assert!(!VALID_PLATFORMS.contains(&"twitter"));
        assert!(!VALID_PLATFORMS.contains(&""));
    }

    #[test]
    fn test_parse_vvhan_response() {
        let json = r#"{
            "success": true,
            "data": [
                {
                    "title": "AI 大模型最新进展",
                    "url": "https://www.zhihu.com/question/123",
                    "hot": "1234万热度",
                    "desc": "关于大模型的讨论",
                    "index": 1
                },
                {
                    "title": "SpaceX 星舰发射",
                    "url": "https://www.zhihu.com/question/456",
                    "hot": 9876543,
                    "desc": "",
                    "index": 2
                }
            ]
        }"#;
        let resp: VvhanResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.success, Some(true));
        let items = resp.data.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title.as_deref(), Some("AI 大模型最新进展"));
        assert_eq!(items[0].index, Some(1));
        assert_eq!(items[1].title.as_deref(), Some("SpaceX 星舰发射"));
    }

    #[test]
    fn test_parse_vvhan_empty() {
        let json = r#"{"success": true, "data": []}"#;
        let resp: VvhanResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap().is_empty());
    }

    #[test]
    fn test_parse_vvhan_failure() {
        let json = r#"{"success": false}"#;
        let resp: VvhanResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.success, Some(false));
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_parse_tenapi_response() {
        let json = r#"{
            "code": 200,
            "data": {
                "list": [
                    {
                        "name": "热搜话题一",
                        "url": "https://weibo.com/topic/1",
                        "hot": "100万"
                    }
                ]
            }
        }"#;
        let resp: TenapiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, Some(200));
        let list = resp.data.unwrap().list.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name.as_deref(), Some("热搜话题一"));
    }

    #[test]
    fn test_parse_tenapi_empty() {
        let json = r#"{"code": 200, "data": {"list": []}}"#;
        let resp: TenapiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.unwrap().list.unwrap().is_empty());
    }

    #[test]
    fn test_parse_tenapi_error() {
        let json = r#"{"code": 500}"#;
        let resp: TenapiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, Some(500));
        assert!(resp.data.is_none());
    }
}
