//! DuckDuckGo Web Search WASM Tool for IronClaw.
//!
//! Searches the web via DuckDuckGo HTML endpoint — no API key required.
//! Parses DDG's HTML search results page to extract titles, URLs, and snippets.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

const DDG_URL: &str = "https://html.duckduckgo.com/html/";
const DEFAULT_COUNT: usize = 5;
const MAX_COUNT: usize = 20;

struct DdgSearchTool;

impl exports::near::agent::tool::Guest for DdgSearchTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
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
        "Search the web using DuckDuckGo. No API key required. \
         Returns titles, URLs, and descriptions for matching web pages."
            .to_string()
    }
}

fn execute_inner(params_json: &str) -> Result<String, String> {
    let params: serde_json::Value =
        serde_json::from_str(params_json).map_err(|e| format!("Invalid parameters: {e}"))?;

    let query = params["query"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("'query' is required and must not be empty")?;

    if query.len() > 500 {
        return Err("'query' exceeds maximum length of 500 characters".into());
    }

    let count = params["count"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_COUNT)
        .clamp(1, MAX_COUNT);

    let region = params["region"].as_str().unwrap_or("wt-wt");

    let body = format!(
        "q={}&kl={}",
        url_encode(query),
        url_encode(region),
    );

    let headers = serde_json::json!({
        "Content-Type": "application/x-www-form-urlencoded",
        "Accept": "text/html,application/xhtml+xml",
        "Accept-Language": "en-US,en;q=0.9",
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    })
    .to_string();

    let body_bytes: Vec<u8> = body.into_bytes();

    let resp = near::agent::host::http_request(
        "POST",
        DDG_URL,
        &headers,
        Some(body_bytes.as_slice()),
        Some(20_000),
    )
    .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("DuckDuckGo returned HTTP {}", resp.status));
    }

    let html = String::from_utf8(resp.body)
        .map_err(|e| format!("Invalid UTF-8 in response: {e}"))?;

    let results = parse_ddg_results(&html, count);

    if results.is_empty() {
        return Err(
            "No results found. DuckDuckGo may have returned a CAPTCHA or empty page.".into(),
        );
    }

    let output = serde_json::json!({
        "query": query,
        "result_count": results.len(),
        "results": results,
    });

    serde_json::to_string(&output).map_err(|e| format!("Serialization error: {e}"))
}

/// Parse DDG HTML search results page.
///
/// DDG HTML structure (each result):
///   <a class="result__a" href="//duckduckgo.com/l/?uddg=ENCODED_URL&...">Title</a>
///   <a class="result__snippet" ...>Snippet text</a>
fn parse_ddg_results(html: &str, max: usize) -> Vec<serde_json::Value> {
    let mut results = Vec::new();

    // Split on result__a to find each result block.
    let parts: Vec<&str> = html.split("class=\"result__a\"").collect();

    for part in parts.iter().skip(1) {
        if results.len() >= max {
            break;
        }

        // Extract href="..."
        let url = match extract_ddg_url(part) {
            Some(u) if !u.is_empty() => u,
            _ => continue,
        };

        // Skip ad results and non-http URLs
        if !url.starts_with("http://") && !url.starts_with("https://") {
            continue;
        }

        // Extract title text (between first > and </a>)
        let title = extract_tag_text(part).unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        // Find snippet in the same result block (ends before next result__a)
        let snippet = extract_snippet(part).unwrap_or_default();

        let mut entry = serde_json::json!({
            "title": html_decode(&title),
            "url": url,
        });

        if !snippet.is_empty() {
            entry["description"] = serde_json::json!(html_decode(&snippet));
        }

        if let Some(host) = extract_hostname(&url) {
            entry["site"] = serde_json::json!(host);
        }

        results.push(entry);
    }

    results
}

/// Extract and decode the real URL from a DDG redirect href.
///
/// DDG uses: href="//duckduckgo.com/l/?uddg=ENCODED_URL&rut=..."
/// or sometimes a direct href for some result types.
fn extract_ddg_url(s: &str) -> Option<String> {
    // Find href="
    let href_start = s.find("href=\"")?;
    let after_href = &s[href_start + 6..];
    let href_end = after_href.find('"')?;
    let href = &after_href[..href_end];

    // Try to extract uddg= parameter (DDG redirect URL)
    if let Some(uddg_pos) = href.find("uddg=") {
        let after_uddg = &href[uddg_pos + 5..];
        let end = after_uddg.find('&').unwrap_or(after_uddg.len());
        let encoded = &after_uddg[..end];
        return Some(url_decode(encoded));
    }

    // Direct URL (not a redirect)
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }

    None
}

/// Extract text content of the first tag (between > and </a>).
fn extract_tag_text(s: &str) -> Option<String> {
    let start = s.find('>')?;
    let after = &s[start + 1..];
    let end = after.find('<')?;
    let text = after[..end].trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Extract snippet text from the same result block.
fn extract_snippet(block: &str) -> Option<String> {
    // Snippet is in class="result__snippet" within this block
    let marker = "result__snippet";
    let start = block.find(marker)?;
    let after_marker = &block[start..];
    // Skip to the > that closes the opening tag
    let tag_end = after_marker.find('>')?;
    let after_tag = &after_marker[tag_end + 1..];
    // Extract until </
    let text_end = after_tag.find('<')?;
    let text = after_tag[..text_end].trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Decode common HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Percent-encode a string for URL query parameters (space → +).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

/// Percent-decode a URL-encoded string.
///
/// Collects decoded bytes first, then converts to UTF-8, so multibyte
/// sequences like %C3%A9 (é) are handled correctly.
fn url_decode(s: &str) -> String {
    let mut buf: Vec<u8> = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_digit(bytes[i + 1]),
                hex_digit(bytes[i + 2]),
            ) {
                buf.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            buf.push(b' ');
        } else {
            buf.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(buf).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn extract_hostname(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() { None } else { Some(host.to_string()) }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "The search query"
        },
        "count": {
            "type": "integer",
            "description": "Number of results to return (1-20, default 5)",
            "minimum": 1,
            "maximum": 20
        },
        "region": {
            "type": "string",
            "description": "Region code to bias results (e.g. 'th-th' for Thailand, 'us-en' for US). Default: wt-wt (no region)"
        }
    },
    "required": ["query"],
    "additionalProperties": false
}"#;

export!(DdgSearchTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("gold price today"), "gold+price+today");
        assert_eq!(url_encode("café"), "caf%C3%A9");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(url_decode("caf%C3%A9"), "café");
        assert_eq!(url_decode("https%3A%2F%2Fexample.com"), "https://example.com");
    }

    #[test]
    fn test_html_decode() {
        assert_eq!(html_decode("AT&amp;T"), "AT&T");
        assert_eq!(html_decode("&lt;b&gt;bold&lt;/b&gt;"), "<b>bold</b>");
        assert_eq!(html_decode("&quot;hello&quot;"), "\"hello\"");
    }

    #[test]
    fn test_extract_hostname() {
        assert_eq!(extract_hostname("https://example.com/path"), Some("example.com".into()));
        assert_eq!(extract_hostname("http://sub.example.com:8080/"), Some("sub.example.com".into()));
        assert_eq!(extract_hostname("ftp://nope"), None);
        assert_eq!(extract_hostname("not-a-url"), None);
    }

    #[test]
    fn test_extract_ddg_url_from_redirect() {
        let s = r#" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&rut=abc">Title</a>"#;
        assert_eq!(extract_ddg_url(s), Some("https://example.com/path".into()));
    }

    #[test]
    fn test_extract_ddg_url_direct() {
        let s = r#" href="https://example.com">Title</a>"#;
        assert_eq!(extract_ddg_url(s), Some("https://example.com".into()));
    }

    #[test]
    fn test_parse_ddg_results_empty() {
        let results = parse_ddg_results("<html><body>No results</body></html>", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_ddg_results_extracts_entries() {
        // Use r##"..."## because the HTML contains `"#` in href="#" which would
        // terminate a r#"..."# raw string early.
        let html = r##"
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgold.org%2F">World Gold Council</a>
            <a class="result__snippet" href="#">Live gold price per ounce.</a>
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fkitco.com%2F">Kitco Gold</a>
            <a class="result__snippet" href="#">Live gold spot price and charts.</a>
        "##;
        let results = parse_ddg_results(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["title"], "World Gold Council");
        assert_eq!(results[0]["url"], "https://gold.org/");
        assert_eq!(results[0]["site"], "gold.org");
        assert_eq!(results[1]["title"], "Kitco Gold");
    }

    #[test]
    fn test_parse_ddg_results_respects_max() {
        let mut html = String::new();
        for i in 0..10 {
            html.push_str(&format!(
                r#"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample{i}.com">Title {i}</a>"#
            ));
        }
        let results = parse_ddg_results(&html, 3);
        assert_eq!(results.len(), 3);
    }
}
