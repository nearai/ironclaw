//! Choose tools for a request with Jev alone: no runtime, no database.
//!
//! ```text
//! TYPESAFE_API_KEY=... cargo run -p ironclaw --example jev_select_tools -- \
//!     "email the Q3 numbers from the budget sheet to finance"
//! TYPESAFE_API_KEY=... cargo run -p ironclaw --example jev_select_tools -- \
//!     --catalog path/to/tools.json --max-tools 8 \
//!     "open a pull request for the fix"
//! ```
//!
//! A catalog file is a JSON array of `{name, description, parameters}` tools
//! (`inputSchema` is accepted for `parameters`), or of groups of them, each
//! a `namespace` with a `tools` array. The request text and every tool's name,
//! description and parameter names are sent to the endpoint.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context as _, bail};
use clap::Parser;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::{
    ConversationContext, ProviderToolDefinition, ToolSelectionCandidate, ToolSelectionClassifier,
    ToolSelectionRequest,
};
use ironclaw_tool_selection_jev::{
    DEFAULT_JEV_ENDPOINT, DEFAULT_JEV_MODEL, JevApiKey, JevEndpoint, JevToolClassifier,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API_KEY_ENV: &str = "TYPESAFE_API_KEY";

#[derive(Parser)]
#[command(about = "Choose tools for a request with the Jev classifier")]
struct Args {
    /// The user's request.
    request: String,
    /// A JSON tool catalog; the small built-in catalog when omitted.
    #[arg(long)]
    catalog: Option<PathBuf>,
    /// The decisions endpoint.
    #[arg(long, default_value = DEFAULT_JEV_ENDPOINT)]
    endpoint: String,
    /// The model to ask.
    #[arg(long, default_value = DEFAULT_JEV_MODEL)]
    model: String,
    /// Most tools to choose.
    #[arg(long, default_value_t = 5)]
    max_tools: usize,
    /// Most estimated schema tokens the chosen tools may add up to.
    #[arg(long, default_value_t = 8_000)]
    token_budget: u32,
    /// Seconds to wait for the whole classification.
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
}

/// One tool in a catalog file.
#[derive(Deserialize)]
struct CatalogTool {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, alias = "inputSchema")]
    parameters: Value,
}

/// A catalog file entry: a tool, or a namespace holding tools.
#[derive(Deserialize)]
#[serde(untagged)]
enum CatalogEntry {
    Group { tools: Vec<CatalogTool> },
    Tool(CatalogTool),
}

/// A small catalog of everyday tools, in the catalog file format.
const BUILT_IN: &str = r#"[
{"name": "gmail__send_message", "description": "Send an email message from the user's Gmail account.", "parameters": {"properties": {"to": {}, "subject": {}, "body": {}}}},
{"name": "gmail__search_messages", "description": "Search the user's Gmail mailbox and return matching messages.", "parameters": {"properties": {"query": {}, "limit": {}}}},
{"name": "calendar__create_event", "description": "Create an event on the user's Google Calendar.", "parameters": {"properties": {"title": {}, "start": {}, "end": {}, "attendees": {}}}},
{"name": "calendar__list_events", "description": "List the events on the user's calendar in a time range.", "parameters": {"properties": {"start": {}, "end": {}}}},
{"name": "sheets__read_range", "description": "Read the values of a range of cells in a Google Sheets spreadsheet.", "parameters": {"properties": {"spreadsheet_id": {}, "range": {}}}},
{"name": "sheets__append_rows", "description": "Append rows to a sheet in a Google Sheets spreadsheet.", "parameters": {"properties": {"spreadsheet_id": {}, "sheet": {}, "rows": {}}}},
{"name": "github__create_pull_request", "description": "Open a pull request in a GitHub repository.", "parameters": {"properties": {"repo": {}, "title": {}, "head": {}, "base": {}, "body": {}}}},
{"name": "github__search_issues", "description": "Search the issues and pull requests of a GitHub repository.", "parameters": {"properties": {"repo": {}, "query": {}}}},
{"name": "slack__post_message", "description": "Post a message to a Slack channel or direct message.", "parameters": {"properties": {"channel": {}, "text": {}}}},
{"name": "web__search", "description": "Search the web and return the top results.", "parameters": {"properties": {"query": {}, "limit": {}}}},
{"name": "web__fetch", "description": "Fetch a web page and return its readable text.", "parameters": {"properties": {"url": {}}}},
{"name": "drive__search_files", "description": "Search the user's Google Drive for files by name or content.", "parameters": {"properties": {"query": {}}}}
]"#;

fn candidate(tool: CatalogTool) -> anyhow::Result<ToolSelectionCandidate> {
    let name = tool.name;
    let id = name.replace("__", ".").to_ascii_lowercase();
    // A capability id needs two segments; a plain name gets a `catalog.` one.
    let id = if id.contains('.') {
        id
    } else {
        format!("catalog.{id}")
    };
    let id = CapabilityId::new(id).with_context(|| format!("tool `{name}`: unusable name"))?;
    let parameters = match tool.parameters {
        Value::Null => json!({"type": "object", "properties": {}}),
        schema => schema,
    };
    // A rough schema cost for the demo: four bytes of definition a token.
    let bytes = name.len() + tool.description.len() + parameters.to_string().len();
    let definition =
        ProviderToolDefinition::from_parts(id, name.as_str(), tool.description, parameters)
            .with_context(|| format!("tool `{name}`: not a valid provider tool"))?;
    Ok(ToolSelectionCandidate {
        definition,
        est_schema_tokens: u32::try_from(bytes / 4).unwrap_or(u32::MAX),
    })
}

/// The tools of a catalog: an array of tools, or of namespaces of tools.
fn parse_catalog(text: &str) -> serde_json::Result<Vec<CatalogTool>> {
    let entries: Vec<CatalogEntry> = serde_json::from_str(text)?;
    let tools = entries.into_iter().flat_map(|entry| match entry {
        CatalogEntry::Group { tools } => tools,
        CatalogEntry::Tool(tool) => vec![tool],
    });
    Ok(tools.collect())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let Ok(key) = std::env::var(API_KEY_ENV) else {
        bail!("{API_KEY_ENV} is not set: export the API key of the Jev provider and run again");
    };
    let classifier = JevToolClassifier::new(
        JevEndpoint::parse(&args.endpoint)?,
        args.model,
        JevApiKey::new(key).with_context(|| format!("{API_KEY_ENV} is empty"))?,
        Duration::from_secs(args.timeout_secs),
    )?;

    let tools = match &args.catalog {
        None => parse_catalog(BUILT_IN).context("the built-in catalog is invalid")?,
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the catalog {}", path.display()))?;
            parse_catalog(&text)
                .with_context(|| format!("{} is not a tool catalog", path.display()))?
        }
    };
    let candidates = tools
        .into_iter()
        .map(candidate)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let request = ToolSelectionRequest {
        context: ConversationContext::new(vec![args.request]),
        candidates,
        max_tools: args.max_tools,
        token_budget: args.token_budget,
    };
    println!(
        "asking {} about {} tools in {} request(s)",
        classifier.model(),
        request.candidates.len(),
        classifier.slice_count(&request),
    );

    let started = Instant::now();
    let selection = classifier.classify(&request).await?;
    let latency = started.elapsed();

    println!("chosen by {} in {latency:.2?}:", selection.scorer);
    for tool in &selection.chosen {
        let tokens = request
            .candidates
            .iter()
            .find(|candidate| candidate.name() == tool.name)
            .map_or(0, |candidate| candidate.est_schema_tokens);
        println!(
            "  {:.2}  {}  (~{tokens} schema tokens)",
            tool.score, tool.name
        );
    }
    if selection.chosen.is_empty() {
        println!("  (nothing fits --max-tools and --token-budget)");
    }
    Ok(())
}
