//! Hacker News WASM tool for IronClaw (#5459 test fixture).
//!
//! The "network + no key" case: an admin imports this tool, activates it, and
//! any user can ask the agent for the Hacker News top stories. The capability
//! declares the `network` effect plus its egress host (`news.ycombinator.com`)
//! via the manifest `network_targets` allowlist, with NO runtime credential —
//! so it gets an `ApplyNetworkPolicy` obligation but never a secret injection.
//!
//! The data is CANNED (no real Hacker News request is made); the egress
//! declaration exists to exercise the keyless-networked obligation path.

mod types;

use types::{Story, TopStories, TopStoriesInput};

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct HackerNewsTool;

fn canned_stories() -> Vec<Story> {
    let raw = [
        (
            "Show HN: I built a sandboxed WASM tool runtime for personal AI agents",
            "https://news.ycombinator.com/item?id=40000001",
            842,
            "claw_dev",
            311,
        ),
        (
            "The case for capability-based security in local-first software",
            "https://news.ycombinator.com/item?id=40000002",
            627,
            "polytope",
            198,
        ),
        (
            "Rust in the browser via WASM components: a 2026 status report",
            "https://news.ycombinator.com/item?id=40000003",
            514,
            "ferris_fan",
            240,
        ),
        (
            "Ask HN: How do you manage secrets for self-hosted agents?",
            "https://news.ycombinator.com/item?id=40000004",
            403,
            "opsgremlin",
            176,
        ),
        (
            "A tiny deterministic scheduler that fits in your head",
            "https://news.ycombinator.com/item?id=40000005",
            366,
            "tickless",
            87,
        ),
        (
            "Why we moved our egress allowlist into the manifest",
            "https://news.ycombinator.com/item?id=40000006",
            298,
            "netpolicy",
            64,
        ),
        (
            "Show HN: ASCII art rendering as a first-class tool",
            "https://news.ycombinator.com/item?id=40000007",
            255,
            "pixelpoet",
            41,
        ),
    ];
    raw.iter()
        .enumerate()
        .map(|(index, (title, url, score, by, comments))| Story {
            rank: (index as u32) + 1,
            title: (*title).to_string(),
            url: (*url).to_string(),
            score: *score,
            by: (*by).to_string(),
            comments: *comments,
        })
        .collect()
}

impl exports::near::agent::tool::Guest for HackerNewsTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        crate::near::agent::host::log(
            crate::near::agent::host::LogLevel::Info,
            "hacker-news.top_stories: returning canned top-stories fixture (no live call)",
        );

        // Lenient parse: empty/blank params or an unexpected shape fall back to
        // the default limit.
        let params = req.params.trim();
        let input: TopStoriesInput = if params.is_empty() {
            TopStoriesInput::default()
        } else {
            serde_json::from_str(params).unwrap_or_default()
        };

        let limit = input.limit.unwrap_or(5).clamp(1, 10) as usize;
        let mut stories = canned_stories();
        stories.truncate(limit);

        let response = TopStories {
            stories,
            as_of: "2026-07-01T00:00:00Z".to_string(),
            data_source: "hacker_news (canned fixture data)".to_string(),
        };

        match serde_json::to_string(&response) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(format!("failed to serialize top stories: {error}")),
            },
        }
    }

    fn schema() -> String {
        r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":10}},"additionalProperties":false}"#
            .to_string()
    }

    fn description() -> String {
        "Return the current top Hacker News stories — title, url, score, author, and \
         comment count. Optional `limit` (1-10, default 5). Use this whenever the user \
         asks what's on Hacker News, HN, or the tech front page. Returns fixture data \
         (no live feed)."
            .to_string()
    }
}

export!(HackerNewsTool);
