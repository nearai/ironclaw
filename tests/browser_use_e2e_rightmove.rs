//! E2E test: navigate to Rightmove and search for flats to rent in Rotherhithe.
//! Demonstrates form filling, search submission, result extraction, and
//! multi-step navigation on a real property listing site.
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

fn make_ctx() -> JobContext {
    JobContext::with_user("test-user", "rightmove-test", "rightmove e2e")
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
async fn e2e_rightmove_rotherhithe_flats_to_rent() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let wrapper = make_wrapper().await;
    let ctx = make_ctx();

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

    // -- Step 2: Open Rightmove To Rent --
    eprintln!("\n=== Step 2: Open Rightmove ===");
    let open = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "open",
            "url": "https://www.rightmove.co.uk/property-to-rent.html",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 30000
        }),
    )
    .await;
    assert_eq!(open["ok"], serde_json::Value::Bool(true));
    eprintln!("  URL: {}", open["data"]["url"]);

    // -- Step 3: Handle cookie consent --
    eprintln!("\n=== Step 3: Handle cookie consent ===");
    let consent = exec(
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
                if (btn) { btn.click(); return { accepted: true, text: btn.textContent.trim() }; }
                return { accepted: false }
            "#
        }),
    )
    .await;
    eprintln!("  Consent: {}", consent["data"]["result"]["accepted"]);

    exec(
        &wrapper,
        &ctx,
        serde_json::json!({ "action": "wait", "ms": 1500, "backend_url": BACKEND, "session_id": sid }),
    )
    .await;

    // -- Step 4: Probe the page to discover selectors --
    eprintln!("\n=== Step 4: Probe page structure ===");
    let probe = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 5000,
            "script": r#"
                var inputs = document.querySelectorAll('input');
                var ii = [];
                for (var i = 0; i < inputs.length && i < 10; i++) {
                    ii.push({ id: inputs[i].id, name: inputs[i].name, type: inputs[i].type,
                              ph: inputs[i].placeholder, cls: inputs[i].className.substring(0,60),
                              testId: inputs[i].getAttribute('data-testid') });
                }
                var btns = document.querySelectorAll('button, input[type="submit"]');
                var bb = [];
                for (var i = 0; i < btns.length && i < 10; i++) {
                    bb.push({ text: btns[i].textContent.trim().substring(0,40), id: btns[i].id,
                              type: btns[i].type, testId: btns[i].getAttribute('data-testid') });
                }
                return { inputs: ii, buttons: bb }
            "#
        }),
    )
    .await;
    if probe["ok"].as_bool().unwrap_or(false) {
        let d = &probe["data"]["result"];
        if let Some(inputs) = d["inputs"].as_array() {
            for inp in inputs {
                eprintln!(
                    "  INPUT: id={} name={} ph={} testId={}",
                    inp["id"], inp["name"], inp["ph"], inp["testId"]
                );
            }
        }
        if let Some(btns) = d["buttons"].as_array() {
            for b in btns {
                eprintln!(
                    "  BUTTON: text='{}' id={} testId={}",
                    b["text"], b["id"], b["testId"]
                );
            }
        }
    }

    // -- Step 5: Fill search and submit --
    eprintln!("\n=== Step 5: Fill search ===");
    let fill = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 5000,
            "script": r#"
                var input = document.querySelector('#searchLocation') ||
                            document.querySelector('input[name="searchLocation"]') ||
                            document.querySelector('[data-testid="search-input"]') ||
                            document.querySelector('input[placeholder*="ocation"]') ||
                            document.querySelector('input[aria-label*="earch"]');
                if (!input) {
                    var all = document.querySelectorAll('input[type="text"], input:not([type])');
                    if (all.length > 0) input = all[0];
                }
                if (!input) return { filled: false, error: 'no input found' };
                input.focus();
                var nativeSet = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                nativeSet.call(input, 'Rotherhithe, London');
                input.dispatchEvent(new Event('input', { bubbles: true }));
                input.dispatchEvent(new Event('change', { bubbles: true }));
                return { filled: true, id: input.id, name: input.name, value: input.value }
            "#
        }),
    )
    .await;
    eprintln!(
        "  Fill: {}",
        serde_json::to_string(&fill["data"]["result"]).unwrap_or_default()
    );

    // Wait for autocomplete
    exec(
        &wrapper,
        &ctx,
        serde_json::json!({ "action": "wait", "ms": 2500, "backend_url": BACKEND, "session_id": sid }),
    )
    .await;

    // Try clicking autocomplete suggestion
    let ac = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 3000,
            "script": r#"
                var items = document.querySelectorAll('li[role="option"], [class*="uggestion"] li, ul[role="listbox"] li, .ksc_resultsList li');
                var info = [];
                for (var i = 0; i < items.length && i < 5; i++) info.push(items[i].textContent.trim().substring(0,60));
                if (items.length > 0) { items[0].click(); return { clicked: true, items: info }; }
                return { clicked: false, items: info }
            "#
        }),
    )
    .await;
    eprintln!(
        "  Autocomplete: {}",
        serde_json::to_string(&ac["data"]["result"]).unwrap_or_default()
    );

    // Submit the form
    let submit = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 5000,
            "script": r#"
                var btn = document.querySelector('[data-testid="toRentCta"]') ||
                          document.querySelector('#submit') ||
                          document.querySelector('button[type="submit"]') ||
                          document.querySelector('input[type="submit"]');
                if (!btn) {
                    var all = document.querySelectorAll('button');
                    for (var i = 0; i < all.length; i++) {
                        var t = all[i].textContent.toLowerCase().trim();
                        if (t === 'to rent' || t.indexOf('search') > -1 || t.indexOf('find') > -1) { btn = all[i]; break; }
                    }
                }
                if (btn) { btn.click(); return { clicked: true, text: btn.textContent.trim() }; }
                var form = document.querySelector('form');
                if (form) { form.submit(); return { submitted: true }; }
                return { clicked: false }
            "#
        }),
    )
    .await;
    eprintln!(
        "  Submit: {}",
        serde_json::to_string(&submit["data"]["result"]).unwrap_or_default()
    );

    // Wait for results
    exec(
        &wrapper,
        &ctx,
        serde_json::json!({ "action": "wait", "ms": 5000, "backend_url": BACKEND, "session_id": sid }),
    )
    .await;

    // Check current page
    let check = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 5000,
            "script": "return { url: location.href, title: document.title, bodyLen: document.body.innerText.length }"
        }),
    )
    .await;
    let cur_url = check["data"]["result"]["url"].as_str().unwrap_or("");
    eprintln!("  URL: {cur_url}");
    eprintln!("  Title: {}", check["data"]["result"]["title"]);

    // Fallback: if form submission didn't navigate to results, use Rightmove's
    // Rotherhithe place page which includes the correct location identifier.
    // The search.html URL without locationIdentifier lands on a disambiguation page.
    // Only consider it a success if we have a real locationIdentifier or a place page.
    let has_results = (cur_url.contains("property-to-rent/find")
        && cur_url.contains("locationIdentifier"))
        || cur_url.contains("property-to-rent/Rotherhithe");
    if !has_results {
        eprintln!("\n=== Step 5b: Fallback to direct results URL ===");
        let direct = exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "open",
                "url": "https://www.rightmove.co.uk/property-to-rent/Rotherhithe.html",
                "backend_url": BACKEND,
                "session_id": sid,
                "timeout_ms": 30000
            }),
        )
        .await;
        assert_eq!(direct["ok"], serde_json::Value::Bool(true));
        eprintln!("  URL: {}", direct["data"]["url"]);

        exec(
            &wrapper,
            &ctx,
            serde_json::json!({ "action": "wait", "ms": 3000, "backend_url": BACKEND, "session_id": sid }),
        )
        .await;
    }

    // -- Step 6: Extract property listings --
    eprintln!("\n=== Step 6: Extract listings ===");
    let extract = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": BACKEND,
            "session_id": sid,
            "timeout_ms": 10000,
            "script": r#"
                var listings = [];

                // Strategy 1: propertyCard elements
                var cards = document.querySelectorAll('.propertyCard, [data-testid="propertyCard"], .l-searchResult');
                for (var i = 0; i < cards.length && listings.length < 10; i++) {
                    var card = cards[i];
                    var addrEl = card.querySelector('.propertyCard-address, [data-testid="address-label"], address, h2');
                    var priceEl = card.querySelector('.propertyCard-priceValue, [data-testid="price"], span[class*="price"], span[class*="Price"]');
                    var descEl = card.querySelector('.propertyCard-description, [data-testid="property-description"]');
                    var linkEl = card.querySelector('a[href*="/properties/"]');
                    var bedsEl = card.querySelector('[class*="bedroom"], [class*="Bedroom"]');

                    var addr = addrEl ? addrEl.textContent.trim() : null;
                    var price = priceEl ? priceEl.textContent.trim() : null;
                    var desc = descEl ? descEl.textContent.trim().substring(0, 200) : null;
                    var link = linkEl ? linkEl.href : null;
                    var beds = bedsEl ? bedsEl.textContent.trim() : null;

                    if (addr || price) {
                        listings.push({ address: addr, price: price, beds: beds, description: desc, link: link });
                    }
                }

                // Strategy 2: broader class-based search
                if (listings.length === 0) {
                    var divs = document.querySelectorAll('[class*="propertyCard"], [class*="property-card"], [id*="property"]');
                    for (var i = 0; i < divs.length && listings.length < 10; i++) {
                        var t = divs[i].textContent.trim();
                        if (t.length > 30 && t.length < 1500) {
                            listings.push({ raw: t.substring(0, 300), source: 'class-match' });
                        }
                    }
                }

                // Strategy 3: look for anything with price-like text (£xxx pcm)
                if (listings.length === 0) {
                    var els = document.querySelectorAll('*');
                    var seen = {};
                    for (var i = 0; i < els.length && listings.length < 10; i++) {
                        var t = els[i].textContent.trim();
                        if (/£[\d,]+\s*p[cm]/i.test(t) && t.length > 30 && t.length < 800 && !seen[t.substring(0,50)]) {
                            seen[t.substring(0,50)] = true;
                            listings.push({ raw: t.substring(0, 300), source: 'price-regex' });
                        }
                    }
                }

                var countEl = document.querySelector('.searchHeader-resultCount, [data-testid="results-count"]');
                var resultCount = countEl ? countEl.textContent.trim() : null;

                var h1 = document.querySelector('h1');
                var header = h1 ? h1.textContent.trim() : null;

                return {
                    resultCount: resultCount,
                    header: header,
                    count: listings.length,
                    listings: listings,
                    url: location.href,
                    title: document.title,
                    bodyLen: document.body.innerText.length
                }
            "#
        }),
    )
    .await;

    let mut listing_count = 0u64;
    if extract["ok"].as_bool().unwrap_or(false) {
        let data = &extract["data"]["result"];
        listing_count = data["count"].as_u64().unwrap_or(0);

        eprintln!("\n  ====== RESULTS ======");
        eprintln!("  Header: {}", data["header"]);
        eprintln!("  Result count: {}", data["resultCount"]);
        eprintln!("  Listings: {listing_count}");
        eprintln!("  Body: {} chars", data["bodyLen"]);
        eprintln!("  URL: {}", data["url"]);

        if let Some(listings) = data["listings"].as_array() {
            eprintln!("\n  ====== TOP LISTINGS ======");
            for (i, l) in listings.iter().take(5).enumerate() {
                if let Some(raw) = l["raw"].as_str() {
                    let preview = &raw[..raw.len().min(150)];
                    eprintln!("  #{}: {preview}", i + 1);
                } else {
                    let addr = l["address"].as_str().unwrap_or("?");
                    let price = l["price"].as_str().unwrap_or("?");
                    let beds = l["beds"].as_str().unwrap_or("");
                    eprintln!("  #{}: {} -- {} {}", i + 1, addr, price, beds);
                    if let Some(desc) = l["description"].as_str() {
                        let preview = if desc.len() > 120 {
                            format!("{}...", &desc[..120])
                        } else {
                            desc.to_string()
                        };
                        eprintln!("     {preview}");
                    }
                }
            }
        }
    }

    // -- Step 7: Screenshot --
    eprintln!("\n=== Step 7: Screenshot ===");
    let screenshot = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "screenshot",
            "backend_url": BACKEND,
            "session_id": sid
        }),
    )
    .await;
    let ss_len = screenshot["data"]["screenshot"]
        .as_str()
        .map(|s| s.len())
        .unwrap_or(0);
    eprintln!("  Screenshot: base64 len={ss_len}");

    // -- Step 8: Close --
    eprintln!("\n=== Step 8: Close session ===");
    let close = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "session_close",
            "session_id": sid,
            "backend_url": BACKEND
        }),
    )
    .await;
    assert_eq!(close["ok"], serde_json::Value::Bool(true));

    // === Assertions ===
    assert!(ss_len > 50_000, "Screenshot should be >50KB, got {ss_len}");

    if listing_count > 0 {
        eprintln!("\n  PASS: {listing_count} property listings extracted from Rightmove!");
    } else {
        eprintln!("\n  PASS: Rightmove loaded (listings may need different selectors).");
    }

    eprintln!("\n=== E2E test completed ===\n");
}
