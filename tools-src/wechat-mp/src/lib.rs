//! WeChat Official Account (MP) WASM Tool for IronClaw.
//!
//! Manage WeChat Official Account materials and users (微信公众号).
//! List and retrieve materials (articles), list followers.
//!
//! # Authentication
//!
//! Store your WeChat MP access token:
//! `ironclaw secret set wechat_mp_access_token <token>`
//!
//! Get a token at: https://mp.weixin.qq.com/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://api.weixin.qq.com/cgi-bin";
const MAX_RETRIES: u32 = 3;

struct WechatMpTool;

impl exports::near::agent::tool::Guest for WechatMpTool {
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
        "Manage WeChat Official Account materials and users (微信公众号). \
         List and retrieve materials (articles), list followers. \
         Authentication is handled via the 'wechat_mp_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(rename = "type")]
    material_type: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
    media_id: Option<String>,
    next_openid: Option<String>,
}

// --- WeChat MP API response types ---

#[derive(Debug, Deserialize)]
struct MaterialListResponse {
    total_count: Option<u32>,
    item_count: Option<u32>,
    item: Option<Vec<MaterialItem>>,
    errcode: Option<i32>,
    errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MaterialItem {
    media_id: Option<String>,
    update_time: Option<u64>,
    content: Option<MaterialContent>,
}

#[derive(Debug, Deserialize)]
struct MaterialContent {
    news_item: Option<Vec<NewsItem>>,
}

#[derive(Debug, Deserialize)]
struct NewsItem {
    title: Option<String>,
    author: Option<String>,
    digest: Option<String>,
    url: Option<String>,
    thumb_media_id: Option<String>,
    content_source_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetMaterialResponse {
    news_item: Option<Vec<NewsItem>>,
    title: Option<String>,
    description: Option<String>,
    down_url: Option<String>,
    errcode: Option<i32>,
    errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserListResponse {
    total: Option<u32>,
    count: Option<u32>,
    data: Option<UserListData>,
    next_openid: Option<String>,
    errcode: Option<i32>,
    errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserListData {
    openid: Option<Vec<String>>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("wechat_mp_access_token") {
        return Err(
            "WeChat MP access token not found in secret store. Set it with: \
             ironclaw secret set wechat_mp_access_token <token>. \
             Get a token at: https://mp.weixin.qq.com/"
                .into(),
        );
    }

    match params.action.as_str() {
        "get_material_list" => get_material_list(&params),
        "get_material" => get_material(&params),
        "get_user_list" => get_user_list(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: get_material_list, get_material, get_user_list",
            params.action
        )),
    }
}

fn get_material_list(params: &Params) -> Result<String, String> {
    let material_type = params
        .material_type
        .as_deref()
        .unwrap_or("news");

    if !matches!(material_type, "news" | "image" | "voice" | "video") {
        return Err(format!(
            "Invalid material type '{}'. Expected: news, image, voice, video",
            material_type
        ));
    }

    let offset = params.offset.unwrap_or(0);
    let count = params.count.unwrap_or(20).clamp(1, 20);

    let url = format!("{BASE_URL}/material/batchget_material");
    let body = serde_json::json!({
        "type": material_type,
        "offset": offset,
        "count": count,
    });

    let resp_body = wechat_request("POST", &url, Some(&body))?;
    let resp: MaterialListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if let Some(errcode) = resp.errcode {
        if errcode != 0 {
            let errmsg = resp.errmsg.unwrap_or_default();
            return Err(format!("WeChat API error (code {}): {}", errcode, errmsg));
        }
    }

    let items = resp.item.unwrap_or_default();
    let formatted: Vec<serde_json::Value> = items
        .into_iter()
        .filter_map(|item| {
            let media_id = item.media_id?;
            let mut entry = serde_json::json!({"media_id": media_id});
            if let Some(update_time) = item.update_time {
                entry["update_time"] = serde_json::json!(update_time);
            }
            if let Some(content) = item.content {
                if let Some(news_items) = content.news_item {
                    let articles: Vec<serde_json::Value> = news_items
                        .into_iter()
                        .filter_map(|n| {
                            let title = n.title?;
                            let mut a = serde_json::json!({"title": title});
                            if let Some(author) = n.author {
                                a["author"] = serde_json::json!(author);
                            }
                            if let Some(digest) = n.digest {
                                a["digest"] = serde_json::json!(digest);
                            }
                            if let Some(url) = n.url {
                                a["url"] = serde_json::json!(url);
                            }
                            Some(a)
                        })
                        .collect();
                    entry["articles"] = serde_json::json!(articles);
                }
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "get_material_list",
        "type": material_type,
        "total_count": resp.total_count.unwrap_or(0),
        "item_count": resp.item_count.unwrap_or(0),
        "items": formatted,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_material(params: &Params) -> Result<String, String> {
    let media_id = params
        .media_id
        .as_deref()
        .ok_or("'media_id' is required for get_material")?;

    if media_id.is_empty() {
        return Err("'media_id' must not be empty".into());
    }

    let url = format!("{BASE_URL}/material/get_material");
    let body = serde_json::json!({"media_id": media_id});

    let resp_body = wechat_request("POST", &url, Some(&body))?;
    let resp: GetMaterialResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if let Some(errcode) = resp.errcode {
        if errcode != 0 {
            let errmsg = resp.errmsg.unwrap_or_default();
            return Err(format!("WeChat API error (code {}): {}", errcode, errmsg));
        }
    }

    let articles: Vec<serde_json::Value> = resp
        .news_item
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| {
            let title = n.title?;
            let mut a = serde_json::json!({"title": title});
            if let Some(author) = n.author {
                a["author"] = serde_json::json!(author);
            }
            if let Some(digest) = n.digest {
                a["digest"] = serde_json::json!(digest);
            }
            if let Some(url) = n.url {
                a["url"] = serde_json::json!(url);
            }
            if let Some(source_url) = n.content_source_url {
                a["content_source_url"] = serde_json::json!(source_url);
            }
            Some(a)
        })
        .collect();

    let output = serde_json::json!({
        "action": "get_material",
        "media_id": media_id,
        "article_count": articles.len(),
        "articles": articles,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn get_user_list(params: &Params) -> Result<String, String> {
    let next_openid = params.next_openid.as_deref().unwrap_or("");
    let url = format!("{BASE_URL}/user/get?next_openid={next_openid}");

    let resp_body = wechat_request("GET", &url, None)?;
    let resp: UserListResponse =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if let Some(errcode) = resp.errcode {
        if errcode != 0 {
            let errmsg = resp.errmsg.unwrap_or_default();
            return Err(format!("WeChat API error (code {}): {}", errcode, errmsg));
        }
    }

    let openids = resp
        .data
        .and_then(|d| d.openid)
        .unwrap_or_default();

    let output = serde_json::json!({
        "action": "get_user_list",
        "total": resp.total.unwrap_or(0),
        "count": resp.count.unwrap_or(0),
        "openids": openids,
        "next_openid": resp.next_openid,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn wechat_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-WeChatMP-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let body_bytes = body.map(|b| b.to_string().into_bytes());
        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            body_bytes.as_deref(),
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
                    "WeChat MP API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "WeChat MP API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'get_material_list' (获取素材列表), 'get_material' (获取素材内容), 'get_user_list' (获取关注者列表)",
            "enum": ["get_material_list", "get_material", "get_user_list"]
        },
        "type": {
            "type": "string",
            "description": "Material type: 'news', 'image', 'voice', 'video' (for get_material_list, default 'news')",
            "enum": ["news", "image", "voice", "video"],
            "default": "news"
        },
        "offset": {
            "type": "integer",
            "description": "Offset for pagination (for get_material_list, default 0)",
            "minimum": 0,
            "default": 0
        },
        "count": {
            "type": "integer",
            "description": "Number of items to return (1-20, default 20, for get_material_list)",
            "minimum": 1,
            "maximum": 20,
            "default": 20
        },
        "media_id": {
            "type": "string",
            "description": "Media ID of the material (required for get_material)"
        },
        "next_openid": {
            "type": "string",
            "description": "Next openid for pagination (optional for get_user_list, empty string for first page)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(WechatMpTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_material_list_response() {
        let json = r#"{
            "total_count": 10,
            "item_count": 1,
            "item": [
                {
                    "media_id": "media_123",
                    "update_time": 1700000000,
                    "content": {
                        "news_item": [
                            {
                                "title": "测试文章",
                                "author": "作者",
                                "digest": "摘要内容",
                                "url": "https://mp.weixin.qq.com/s/abc123",
                                "thumb_media_id": "thumb_123",
                                "content_source_url": "https://example.com"
                            }
                        ]
                    }
                }
            ]
        }"#;
        let resp: MaterialListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total_count, Some(10));
        assert_eq!(resp.item_count, Some(1));
        let items = resp.item.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].media_id.as_deref(), Some("media_123"));
        let news = items[0].content.as_ref().unwrap().news_item.as_ref().unwrap();
        assert_eq!(news[0].title.as_deref(), Some("测试文章"));
    }

    #[test]
    fn test_parse_get_material_response() {
        let json = r#"{
            "news_item": [
                {
                    "title": "文章标题",
                    "author": "文章作者",
                    "digest": "文章摘要",
                    "url": "https://mp.weixin.qq.com/s/xyz",
                    "content_source_url": "https://source.example.com"
                }
            ]
        }"#;
        let resp: GetMaterialResponse = serde_json::from_str(json).unwrap();
        let news = resp.news_item.unwrap();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].title.as_deref(), Some("文章标题"));
        assert_eq!(news[0].author.as_deref(), Some("文章作者"));
    }

    #[test]
    fn test_parse_user_list_response() {
        let json = r#"{
            "total": 1000,
            "count": 2,
            "data": {
                "openid": ["openid_001", "openid_002"]
            },
            "next_openid": "openid_002"
        }"#;
        let resp: UserListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, Some(1000));
        assert_eq!(resp.count, Some(2));
        let openids = resp.data.unwrap().openid.unwrap();
        assert_eq!(openids.len(), 2);
        assert_eq!(openids[0], "openid_001");
        assert_eq!(resp.next_openid.as_deref(), Some("openid_002"));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"errcode": 40001, "errmsg": "invalid credential"}"#;
        let resp: MaterialListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.errcode, Some(40001));
        assert_eq!(resp.errmsg.as_deref(), Some("invalid credential"));
    }

    #[test]
    fn test_parse_empty_material_list() {
        let json = r#"{"total_count": 0, "item_count": 0, "item": []}"#;
        let resp: MaterialListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total_count, Some(0));
        assert!(resp.item.unwrap().is_empty());
    }
}
