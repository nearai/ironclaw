//! Real-world E2E: Find 2-bed unfurnished flats to rent in Rotherhithe,
//! £3,000–£3,500/month on Rightmove. Extract first 5 listings.
//!
//! Requires: Browserless Docker on port 9222
//!   docker run -d --name browserless-test -p 9222:3000 ghcr.io/browserless/chromium:latest

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw::context::JobContext;
use ironclaw::tools::Tool;
use ironclaw::tools::wasm::{
    Capabilities, EndpointPattern, HttpCapability, WasmRuntimeConfig, WasmToolRuntime,
    WasmToolWrapper, WebSocketCapability, WebSocketEndpoint, WorkspaceCapability, WorkspaceReader,
    WorkspaceWriter,
};

fn wasm_path() -> std::path::PathBuf {
    let in_tree = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools-src/browser-use/target/wasm32-wasip2/release/browser_use_tool.wasm");
    if in_tree.exists() {
        return in_tree;
    }
    let global = std::path::PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/rust-target")
    }))
    .join("wasm32-wasip2/release/browser_use_tool.wasm");
    if global.exists() {
        return global;
    }
    panic!(
        "browser-use WASM not found. Build it first:\n  \
         cd tools-src/browser-use && cargo build --target wasm32-wasip2 --release"
    );
}

fn browserless_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:9222".parse().unwrap(),
        std::time::Duration::from_secs(2),
    )
    .is_ok()
}

#[derive(Clone)]
struct InMemoryWorkspace {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryWorkspace {
    fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl WorkspaceReader for InMemoryWorkspace {
    fn read(&self, path: &str) -> Option<String> {
        self.data.lock().unwrap().get(path).cloned()
    }
}

impl WorkspaceWriter for InMemoryWorkspace {
    fn write(&self, path: &str, content: &str) -> Result<(), String> {
        self.data
            .lock()
            .unwrap()
            .insert(path.to_string(), content.to_string());
        Ok(())
    }
}

fn make_runtime() -> Arc<WasmToolRuntime> {
    let mut config = WasmRuntimeConfig::for_testing();
    config.fuel_config.initial_fuel = 500_000_000;
    config.default_limits.memory_bytes = 10 * 1024 * 1024;
    config.default_limits.fuel = 500_000_000;
    config.default_limits.timeout = std::time::Duration::from_secs(60);
    Arc::new(WasmToolRuntime::new(config).unwrap())
}

fn make_capabilities() -> Capabilities {
    let workspace = InMemoryWorkspace::new();
    Capabilities {
        workspace_read: Some(WorkspaceCapability {
            allowed_prefixes: vec!["browser-sessions/".to_string()],
            reader: Some(Arc::new(workspace.clone()) as Arc<dyn WorkspaceReader>),
            writer: Some(Arc::new(workspace) as Arc<dyn WorkspaceWriter>),
        }),
        http: Some(HttpCapability {
            allowlist: vec![
                EndpointPattern::host("127.0.0.1"),
                EndpointPattern::host("localhost"),
            ],
            ..Default::default()
        }),
        websocket: Some(
            WebSocketCapability::new(vec![
                WebSocketEndpoint::host("127.0.0.1"),
                WebSocketEndpoint::host("localhost"),
            ])
            .with_pool(),
        ),
        ..Default::default()
    }
}

async fn make_wrapper() -> WasmToolWrapper {
    let runtime = make_runtime();
    let bytes = std::fs::read(wasm_path()).expect("read WASM");
    let prepared = runtime
        .prepare("browser-use-tool", &bytes, None)
        .await
        .expect("WASM preparation failed");
    WasmToolWrapper::new(runtime, prepared, make_capabilities())
}

async fn exec(
    wrapper: &WasmToolWrapper,
    ctx: &JobContext,
    params: serde_json::Value,
) -> serde_json::Value {
    let result = wrapper.execute(params.clone(), ctx).await;
    assert!(
        result.is_ok(),
        "execute failed for {:?}: {:?}",
        params.get("action"),
        result.err()
    );
    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);
    let action_name = params.get("action").and_then(|a| a.as_str()).unwrap_or("?");
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    eprintln!("  [{action_name}] ok={ok}");
    value
}

const BACKEND: &str = "http://127.0.0.1:9222";

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_rightmove_rotherhithe_2bed_3k_unfurnished() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let wrapper = make_wrapper().await;
    let ctx = JobContext::with_user("test-user", "rightmove-filtered", "rightmove filtered e2e");

    // -- Step 1: Create session --
    eprintln!("\n=== Step 1: Create session ===");
    let session_result = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "session_create",
            "backend_url": BACKEND,
        }),
    )
    .await;
    assert_eq!(session_result["ok"], serde_json::Value::Bool(true));
    let sid = session_result["data"]["sessionId"]
        .as_str()
        .expect("missing sessionId");
    eprintln!("  Session: {sid}");

    // -- Step 2: Open Rightmove with filters via URL --
    // Rightmove search URL parameters:
    //   locationIdentifier=REGION%5E93917 (Rotherhithe, London)
    //   minBedrooms=2 maxBedrooms=2
    //   minPrice=3000 maxPrice=3500
    //   furnishTypes=unfurnished
    //   propertyTypes=flat
    eprintln!("\n=== Step 2: Open Rightmove with filters ===");
    let search_url = "https://www.rightmove.co.uk/property-to-rent/Rotherhithe.html?\
        minBedrooms=2&maxBedrooms=2\
        &minPrice=3000&maxPrice=3500\
        &furnishTypes=unfurnished\
        &propertyTypes=flat\
        &includeLetAgreed=false\
        &dontShow=\
        &keywords=";

    let open = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "open",
            "url": search_url,
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 30000
        }),
    )
    .await;
    assert_eq!(open["ok"], serde_json::Value::Bool(true));

    // -- Step 3: Handle cookie consent + wait for page --
    eprintln!("\n=== Step 3: Handle cookie consent ===");
    exec(
        &wrapper,
        &ctx,
        serde_json::json!({ "action": "wait", "ms": 2000, "backend_url": BACKEND, "session_id": sid }),
    )
    .await;

    exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 5000,
            "script": r#"
                var btn = document.querySelector('#onetrust-accept-btn-handler');
                if (!btn) {
                    var all = document.querySelectorAll('button');
                    for (var i = 0; i < all.length; i++) {
                        var t = all[i].textContent;
                        if (t.indexOf('Accept') > -1 || t.indexOf('Agree') > -1) { btn = all[i]; break; }
                    }
                }
                if (btn) { btn.click(); return { accepted: true }; }
                return { accepted: false }
            "#
        }),
    )
    .await;

    exec(
        &wrapper,
        &ctx,
        serde_json::json!({ "action": "wait", "ms": 3000, "backend_url": BACKEND, "session_id": sid }),
    )
    .await;

    // -- Step 4a: Probe DOM structure --
    eprintln!("\n=== Step 4a: Probe DOM structure ===");
    let probe = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 10000,
            "script": r#"
                var probes = {};

                // Check various known selectors
                var selectors = [
                    '.l-searchResult', '.propertyCard', '[data-testid="propertyCard"]',
                    '[id^="property-"]', '.l-searchResults', '.searchResults-wrapper',
                    '[class*="PropertyCard"]', '[class*="propertyCard"]',
                    '[class*="listing"]', '[class*="Listing"]',
                    'article', '.search-result',
                    '[data-test]', '[data-testid]'
                ];
                for (var i = 0; i < selectors.length; i++) {
                    var els = document.querySelectorAll(selectors[i]);
                    if (els.length > 0) {
                        probes[selectors[i]] = {
                            count: els.length,
                            firstTag: els[0].tagName,
                            firstClasses: els[0].className.toString().substring(0, 120),
                            firstId: els[0].id || '',
                            childCount: els[0].children.length
                        };
                    }
                }

                // Find all elements with "pcm" in their text (price markers)
                var priceEls = [];
                var all = document.querySelectorAll('*');
                for (var i = 0; i < all.length && priceEls.length < 5; i++) {
                    var t = all[i].textContent || '';
                    if (/£[\d,]+\s*pcm/i.test(t) && t.length < 200) {
                        priceEls.push({
                            tag: all[i].tagName,
                            cls: all[i].className.toString().substring(0, 100),
                            text: t.trim().substring(0, 150),
                            parentTag: all[i].parentElement ? all[i].parentElement.tagName : '',
                            parentCls: all[i].parentElement ? all[i].parentElement.className.toString().substring(0, 100) : ''
                        });
                    }
                }

                // Get data-testid elements
                var testIds = [];
                var tels = document.querySelectorAll('[data-testid]');
                for (var i = 0; i < tels.length && testIds.length < 30; i++) {
                    var tid = tels[i].getAttribute('data-testid');
                    if (testIds.indexOf(tid) === -1) testIds.push(tid);
                }

                return {
                    probes: probes,
                    priceElements: priceEls,
                    testIds: testIds,
                    bodyLen: document.body.innerText.length
                }
            "#
        }),
    )
    .await;
    if probe["ok"].as_bool().unwrap_or(false) {
        let d = &probe["data"]["result"];
        eprintln!("  Body length: {} chars", d["bodyLen"]);
        eprintln!("  Matching selectors:");
        if let Some(probes) = d["probes"].as_object() {
            for (sel, info) in probes {
                eprintln!("    {sel}: count={}, tag={}, classes={}", info["count"], info["firstTag"], info["firstClasses"]);
            }
        }
        eprintln!("  Price elements:");
        if let Some(pels) = d["priceElements"].as_array() {
            for p in pels {
                eprintln!("    <{}> cls='{}' text='{}'", p["tag"], p["cls"], p["text"]);
                eprintln!("      parent: <{}> cls='{}'", p["parentTag"], p["parentCls"]);
            }
        }
        eprintln!("  data-testid values:");
        if let Some(tids) = d["testIds"].as_array() {
            for tid in tids {
                eprint!("    {} ", tid);
            }
            eprintln!();
        }
    }

    // -- Step 4b: Probe a single property card structure --
    eprintln!("\n=== Step 4b: Probe property card inner structure ===");
    let card_probe = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 10000,
            "script": r#"
                var card = document.querySelector('[data-testid="propertyCard-0"]');
                if (!card) return { error: 'no propertyCard-0 found' };

                // Get all child data-testid values
                var childTestIds = [];
                var children = card.querySelectorAll('[data-testid]');
                for (var i = 0; i < children.length; i++) {
                    childTestIds.push({
                        testid: children[i].getAttribute('data-testid'),
                        tag: children[i].tagName,
                        cls: children[i].className.toString().substring(0, 80),
                        text: children[i].textContent.trim().substring(0, 100)
                    });
                }

                // Get direct text content breakdown
                var spans = card.querySelectorAll('span, a, h2, h3, address, p, div');
                var textEls = [];
                for (var i = 0; i < spans.length && textEls.length < 30; i++) {
                    var t = spans[i].textContent.trim();
                    if (t.length > 2 && t.length < 200) {
                        textEls.push({
                            tag: spans[i].tagName,
                            cls: spans[i].className.toString().substring(0, 80),
                            text: t.substring(0, 120)
                        });
                    }
                }

                // Get links
                var links = card.querySelectorAll('a');
                var linkInfo = [];
                for (var i = 0; i < links.length && i < 5; i++) {
                    linkInfo.push({
                        href: links[i].href.substring(0, 120),
                        text: links[i].textContent.trim().substring(0, 80)
                    });
                }

                return {
                    outerHTML_length: card.outerHTML.length,
                    innerText: card.innerText.substring(0, 500),
                    childTestIds: childTestIds,
                    textElements: textEls,
                    links: linkInfo
                }
            "#
        }),
    )
    .await;
    if card_probe["ok"].as_bool().unwrap_or(false) {
        let d = &card_probe["data"]["result"];
        eprintln!("  Card HTML length: {}", d["outerHTML_length"]);
        eprintln!("  Card innerText:\n{}", d["innerText"]);
        eprintln!("\n  Child data-testid elements:");
        if let Some(arr) = d["childTestIds"].as_array() {
            for item in arr {
                eprintln!("    [{}] <{}> cls='{}' text='{}'",
                    item["testid"], item["tag"], item["cls"],
                    item["text"].as_str().unwrap_or("").chars().take(80).collect::<String>());
            }
        }
        eprintln!("\n  Links:");
        if let Some(arr) = d["links"].as_array() {
            for item in arr {
                eprintln!("    href={} text='{}'", item["href"], item["text"]);
            }
        }
    }

    // -- Step 4c: Extract property listings --
    eprintln!("\n=== Step 4c: Extract listings ===");
    let extract = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 15000,
            "script": r#"
                var listings = [];
                var seen = {};

                // Rightmove 2025/2026: cards use data-testid="propertyCard-N"
                for (var idx = 0; idx < 30 && listings.length < 5; idx++) {
                    var card = document.querySelector('[data-testid="propertyCard-' + idx + '"]');
                    if (!card) continue;

                    // Deduplicate by property link
                    var linkEl = card.querySelector('a[href*="/properties/"]');
                    var link = linkEl ? linkEl.href.split('#')[0].split('?')[0] : null;
                    if (link && seen[link]) continue;
                    if (link) seen[link] = true;

                    // Address
                    var addrEl = card.querySelector('[data-testid="property-address"]');
                    var addr = addrEl ? addrEl.textContent.trim() : null;

                    // Price -- extract just the pcm figure
                    var priceEl = card.querySelector('[data-testid="property-price"]');
                    var price = null;
                    if (priceEl) {
                        var pm = priceEl.textContent.match(/£[\d,]+\s*pcm/i);
                        price = pm ? pm[0] : priceEl.textContent.trim();
                    }

                    // Property type and bedrooms from property-information
                    var infoEl = card.querySelector('[data-testid="property-information"]');
                    var ptype = null;
                    var beds = null;
                    var baths = null;
                    if (infoEl) {
                        var infoText = infoEl.textContent.trim();
                        // Format is like "Flat22" or "Apartment21"
                        var tm = infoText.match(/^([A-Za-z\s-]+?)(\d)(\d)?$/);
                        if (tm) {
                            ptype = tm[1].trim();
                            beds = tm[2];
                            baths = tm[3] || null;
                        } else {
                            ptype = infoText;
                        }
                    }

                    // Description
                    var descEl = card.querySelector('[data-testid="property-description"]');
                    var desc = descEl ? descEl.textContent.trim().substring(0, 300) : null;

                    // Agent / marketed by
                    var agentEl = card.querySelector('[data-testid="marketed-by-text"]');
                    var agent = null;
                    if (agentEl) {
                        var at = agentEl.textContent.trim();
                        // Format: "Added X ago by AgentName, BranchAdded X ago"
                        var am = at.match(/by\s+(.+?)(?:Added|$)/);
                        agent = am ? am[1].trim() : at;
                    }

                    // Tags / features
                    var tags = [];
                    var tagEls = card.querySelectorAll('[data-testid^="property-tag-"]');
                    for (var t = 0; t < tagEls.length && t < 10; t++) {
                        tags.push(tagEls[t].textContent.trim());
                    }

                    if (addr || price) {
                        listings.push({
                            address: addr,
                            price: price,
                            property_type: ptype,
                            bedrooms: beds,
                            bathrooms: baths,
                            description: desc,
                            agent: agent,
                            features: tags,
                            link: link
                        });
                    }
                }

                var h1 = document.querySelector('h1');
                var header = h1 ? h1.textContent.trim() : null;
                var descEl = document.querySelector('[data-testid="search-description"]');
                var resultCount = descEl ? descEl.textContent.trim() : null;

                return {
                    resultCount: resultCount,
                    header: header,
                    count: listings.length,
                    listings: listings,
                    url: location.href,
                    title: document.title
                }
            "#
        }),
    )
    .await;

    // -- Print results --
    if extract["ok"].as_bool().unwrap_or(false) {
        let data = &extract["data"]["result"];
        eprintln!("\n  ====== SEARCH RESULTS ======");
        eprintln!("  Header: {}", data["header"]);
        eprintln!("  Total results: {}", data["resultCount"]);
        eprintln!("  Extracted: {} listings", data["count"]);

        if let Some(listings) = data["listings"].as_array() {
            for (i, l) in listings.iter().enumerate() {
                eprintln!("\n  --- Listing #{} ---", i + 1);
                eprintln!("  Address:  {}", l["address"].as_str().unwrap_or("N/A"));
                eprintln!("  Price:    {}", l["price"].as_str().unwrap_or("N/A"));
                if let Some(pt) = l["property_type"].as_str()
                    && !pt.is_empty()
                {
                    eprintln!("  Type:     {pt}");
                }
                if let Some(beds) = l["bedrooms"].as_str() {
                    eprintln!("  Beds:     {beds}");
                }
                if let Some(baths) = l["bathrooms"].as_str() {
                    eprintln!("  Baths:    {baths}");
                }
                if let Some(agent) = l["agent"].as_str()
                    && !agent.is_empty()
                {
                    eprintln!("  Agent:    {agent}");
                }
                if let Some(features) = l["features"].as_array()
                    && !features.is_empty()
                {
                    let tags: Vec<&str> = features
                        .iter()
                        .filter_map(|f| f.as_str())
                        .collect();
                    eprintln!("  Features: {}", tags.join(", "));
                }
                if let Some(desc) = l["description"].as_str()
                    && !desc.is_empty()
                {
                    eprintln!(
                        "  Desc:     {}",
                        &desc[..desc.len().min(200)]
                    );
                }
                if let Some(link) = l["link"].as_str() {
                    eprintln!("  Link:     {link}");
                }
            }
        }
    } else {
        eprintln!("  Extract failed: {}", extract);
    }

    // -- Step 5: Close session --
    eprintln!("\n=== Step 5: Close session ===");
    exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "session_close",
            "session_id": sid,
            "backend_url": BACKEND
        }),
    )
    .await;

    eprintln!("\n=== E2E test completed ===\n");
}
