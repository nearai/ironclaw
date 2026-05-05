//! Natural-language → portfolio action compiler.
//!
//! Deterministic, pattern-based parsing of trade prompts into one of
//! six recommended next actions:
//!
//! - `plan_dca_schedule` — recurring buy
//!   ("DCA $100 weekly into NEAR for 6 months"),
//! - `build_intent` — one-shot rotation
//!   ("swap 0.5 BTC to USDC on near"),
//! - `backtest_suite` — paper test request
//!   ("backtest SMA 5/20 + buy-hold on NEAR/USDC"),
//! - `plan_paid_research` — premium research request
//!   ("what's catalyst risk on NEAR this week"),
//! - `format_intents_widget` — passive surveillance
//!   ("watch BTC for breakout above 70k"),
//! - `noop` — no usable interpretation; the caller should ask the
//!   user for more detail.
//!
//! The compiler returns a structured plan (extracted fields,
//! assumptions, clarifications, gates) plus the params it would
//! invoke. It never calls the action — the agent does that after
//! risk-gating. This is a deterministic UX layer, not an LLM call:
//! the same prompt always returns the same plan.
//!
//! The compiler is the "vibratrading-style" surface for intents
//! trading: chat in, structured proposal out, all unsigned. Signing
//! is always a wallet action outside the agent.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct CompileInput {
    pub prompt: String,
    /// Default chain when the prompt omits chain. Defaults to "near"
    /// because NEAR Intents is the agent's primary execution surface.
    #[serde(default = "default_chain")]
    pub default_chain: String,
    /// Default source asset for DCA when the prompt names a
    /// destination but no source. Defaults to USDC.
    #[serde(default = "default_funding_asset")]
    pub default_funding_asset: String,
    /// Optional pair the agent is currently focused on; used to
    /// resolve ambiguous backtest prompts that don't name a pair.
    #[serde(default)]
    pub focus_pair: Option<String>,
}

fn default_chain() -> String {
    "near".to_string()
}

fn default_funding_asset() -> String {
    "USDC".to_string()
}

#[derive(Debug, Serialize)]
pub struct CompileOutput {
    pub schema_version: &'static str,
    pub prompt: String,
    pub intent_kind: String,
    pub confidence: f64,
    pub recommended_action: String,
    pub recommended_params: Value,
    pub extracted: Extracted,
    pub assumptions: Vec<String>,
    pub clarifications_needed: Vec<String>,
    pub gates: Vec<NlGate>,
    pub trace: Vec<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct Extracted {
    pub source_asset: Option<String>,
    pub destination_asset: Option<String>,
    pub source_chain: Option<String>,
    pub destination_chain: Option<String>,
    pub amount: Option<String>,
    pub amount_unit: Option<String>,
    pub cadence: Option<String>,
    pub total_periods: Option<usize>,
    pub strategy_kind: Option<String>,
    pub fast_window: Option<usize>,
    pub slow_window: Option<usize>,
    pub price_band_premium_bps: Option<f64>,
    pub price_band_discount_bps: Option<f64>,
    pub watch_threshold: Option<String>,
    pub research_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NlGate {
    pub name: String,
    pub status: String,
    pub detail: String,
}

const ASSETS: &[&str] = &[
    "NEAR", "USDC", "USDT", "DAI", "BTC", "WBTC", "ETH", "WETH", "SOL", "ARB", "OP", "MATIC",
    "AVAX", "BNB", "DOGE", "TON", "SUI", "APT", "LINK", "UNI",
];
const INTENTS_SUPPORTED: &[&str] = &["NEAR", "USDC", "USDT", "BTC", "WBTC", "ETH", "WETH"];
const CHAINS: &[&str] = &[
    "near",
    "ethereum",
    "arbitrum",
    "base",
    "optimism",
    "polygon",
    "bitcoin",
    "solana",
    "bnb",
    "avalanche",
    "ton",
];

pub fn compile(input: CompileInput) -> Result<CompileOutput, String> {
    let raw = input.prompt.clone();
    if raw.trim().is_empty() {
        return Err("prompt must be non-empty".to_string());
    }
    let prompt = raw.to_lowercase();
    let mut trace = Vec::new();
    let mut extracted = Extracted::default();
    let mut assumptions = Vec::new();
    let mut clarifications = Vec::new();

    let kind = classify(&prompt, &mut trace);
    trace.push(format!("classified: {kind}"));

    let (recommended_action, recommended_params, confidence, gates) = match kind.as_str() {
        "dca-schedule" => build_dca(
            &prompt,
            &input,
            &mut extracted,
            &mut assumptions,
            &mut clarifications,
            &mut trace,
        ),
        "swap" => build_swap(
            &prompt,
            &raw,
            &input,
            &mut extracted,
            &mut assumptions,
            &mut clarifications,
            &mut trace,
        ),
        "backtest" => build_backtest(
            &prompt,
            &input,
            &mut extracted,
            &mut assumptions,
            &mut clarifications,
            &mut trace,
        ),
        "research" => build_research(
            &raw,
            &input,
            &mut extracted,
            &mut assumptions,
            &mut clarifications,
            &mut trace,
        ),
        "watch" => build_watch(
            &prompt,
            &input,
            &mut extracted,
            &mut assumptions,
            &mut clarifications,
            &mut trace,
        ),
        _ => (
            "noop".to_string(),
            json!({}),
            0.05,
            vec![NlGate {
                name: "intent_recognized".to_string(),
                status: "fail".to_string(),
                detail:
                    "prompt did not match any supported intent kind; ask the user for cadence, asset, or strategy"
                        .to_string(),
            }],
        ),
    };

    Ok(CompileOutput {
        schema_version: "intents-nl-compile/1",
        prompt: raw,
        intent_kind: kind,
        confidence,
        recommended_action,
        recommended_params,
        extracted,
        assumptions,
        clarifications_needed: clarifications,
        gates,
        trace,
    })
}

fn classify(prompt: &str, trace: &mut Vec<String>) -> String {
    if contains_any(
        prompt,
        &[
            "dca",
            "dollar cost average",
            "dollar-cost average",
            "average into",
            "stack ",
            "accumulate",
            "buy weekly",
            "buy monthly",
            "buy daily",
        ],
    ) {
        trace.push("matched: dca keywords".to_string());
        return "dca-schedule".to_string();
    }
    if contains_any(
        prompt,
        &[
            "swap ",
            "rotate ",
            "convert ",
            "move ",
            "exchange ",
            "trade ",
            "rebalance",
        ],
    ) {
        trace.push("matched: swap keywords".to_string());
        return "swap".to_string();
    }
    if contains_any(
        prompt,
        &[
            "backtest",
            "paper test",
            "paper-test",
            "paper trade",
            "historical test",
        ],
    ) {
        trace.push("matched: backtest keywords".to_string());
        return "backtest".to_string();
    }
    if contains_any(
        prompt,
        &[
            "watch ",
            "monitor ",
            "alert ",
            "notify",
            "tell me when",
            "let me know when",
        ],
    ) {
        trace.push("matched: watch keywords".to_string());
        return "watch".to_string();
    }
    if contains_any(
        prompt,
        &[
            "research ",
            "what is the catalyst",
            "what are the catalysts",
            "catalyst",
            "due diligence",
            "what's happening with",
            "whats happening with",
            "is there news",
            "explain ",
            "summarize ",
        ],
    ) {
        trace.push("matched: research keywords".to_string());
        return "research".to_string();
    }
    "unsupported".to_string()
}

fn build_dca(
    prompt: &str,
    input: &CompileInput,
    extracted: &mut Extracted,
    assumptions: &mut Vec<String>,
    clarifications: &mut Vec<String>,
    trace: &mut Vec<String>,
) -> (String, Value, f64, Vec<NlGate>) {
    let amount = parse_amount(prompt, trace).unwrap_or(100.0);
    if !prompt_mentions_money(prompt) {
        assumptions.push(format!(
            "assumed notional ${amount} per period (no amount in prompt)"
        ));
    }
    extracted.amount = Some(format!("{amount:.2}"));
    extracted.amount_unit = Some("USD".to_string());

    let dest = find_destination(prompt, trace).unwrap_or_else(|| {
        assumptions.push(
            "no destination asset detected; defaulted to NEAR (primary intents asset)".to_string(),
        );
        "NEAR".to_string()
    });
    extracted.destination_asset = Some(dest.clone());

    let src = find_source(prompt, trace).unwrap_or_else(|| {
        assumptions.push(format!(
            "assumed source asset {} (typical funding stable)",
            input.default_funding_asset
        ));
        input.default_funding_asset.clone()
    });
    extracted.source_asset = Some(src.clone());

    let cadence = parse_cadence(prompt, trace).unwrap_or_else(|| {
        assumptions.push("assumed weekly cadence (no cadence in prompt)".to_string());
        "weekly".to_string()
    });
    extracted.cadence = Some(cadence.clone());

    let total_periods = parse_total_periods(prompt, &cadence, trace).unwrap_or_else(|| {
        let default = match cadence.as_str() {
            "daily" => 30,
            "weekly" => 26,
            "biweekly" => 13,
            "monthly" => 12,
            _ => 12,
        };
        assumptions.push(format!(
            "assumed {default} periods (no duration in prompt; cadence={cadence})"
        ));
        default
    });
    extracted.total_periods = Some(total_periods);

    let chain = parse_chain(prompt).unwrap_or_else(|| {
        assumptions.push(format!("assumed chain {} (default)", input.default_chain));
        input.default_chain.clone()
    });
    extracted.source_chain = Some(chain.clone());
    extracted.destination_chain = Some(chain.clone());

    if let Some(bps) = parse_price_floor(prompt) {
        extracted.price_band_discount_bps = Some(bps);
        trace.push(format!("matched: price floor → discount {bps}bps"));
    }
    if let Some(bps) = parse_price_ceiling(prompt) {
        extracted.price_band_premium_bps = Some(bps);
        trace.push(format!("matched: price ceiling → premium {bps}bps"));
    }

    let pair = format!("{dest}/{src}");
    let intents_supported = INTENTS_SUPPORTED
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&dest));
    if !intents_supported {
        clarifications.push(format!(
            "destination asset '{dest}' is outside the default NEAR Intents allowlist; confirm route"
        ));
    }

    let mut params = json!({
        "action": "plan_dca_schedule",
        "pair": pair,
        "source_asset": src,
        "destination_asset": dest,
        "source_chain": chain,
        "destination_chain": chain,
        "notional_per_period_usd": amount,
        "cadence": cadence,
        "total_periods": total_periods,
        "max_slippage_bps": 50.0,
        "mode": "paper",
        "notional_currency": input.default_funding_asset,
    });
    if let Some(p) = extracted.price_band_premium_bps {
        params["skip_above_premium_bps"] = json!(p);
    }
    if let Some(d) = extracted.price_band_discount_bps {
        params["opportunistic_below_discount_bps"] = json!(d);
    }

    let gates = vec![
        NlGate {
            name: "intents_supported_destination".to_string(),
            status: if intents_supported { "pass" } else { "warn" }.to_string(),
            detail: format!("destination={} (allowlist={intents_supported})", dest),
        },
        NlGate {
            name: "duration_within_year".to_string(),
            status: if total_periods <= 365 { "pass" } else { "warn" }.to_string(),
            detail: format!("{total_periods} periods at {cadence} cadence"),
        },
        NlGate {
            name: "unsigned_only".to_string(),
            status: "pass".to_string(),
            detail: "agent never signs; per-period intents must be re-quoted before user signs"
                .to_string(),
        },
    ];

    let confidence = match assumptions.len() {
        0 => 0.95,
        1..=2 => 0.85,
        3..=4 => 0.7,
        _ => 0.5,
    };
    ("plan_dca_schedule".to_string(), params, confidence, gates)
}

fn build_swap(
    prompt: &str,
    raw: &str,
    input: &CompileInput,
    extracted: &mut Extracted,
    assumptions: &mut Vec<String>,
    clarifications: &mut Vec<String>,
    trace: &mut Vec<String>,
) -> (String, Value, f64, Vec<NlGate>) {
    // Look for "<num> <asset> to <asset>"
    let tokens: Vec<&str> = prompt.split_whitespace().collect();
    let mut amount: Option<f64> = None;
    let mut from_asset: Option<String> = None;
    let mut to_asset: Option<String> = None;

    for (i, tok) in tokens.iter().enumerate() {
        if let Ok(n) = tok.trim_start_matches('$').parse::<f64>() {
            if amount.is_none() {
                amount = Some(n);
                if let Some(next) = tokens.get(i + 1) {
                    if let Some(a) = match_asset(next) {
                        from_asset = Some(a);
                    }
                }
            }
        }
    }
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(*tok, "to" | "into" | "for") {
            if let Some(next) = tokens.get(i + 1) {
                if let Some(a) = match_asset(next) {
                    to_asset = Some(a);
                }
            }
        }
    }
    if from_asset.is_none() {
        for tok in &tokens {
            if let Some(a) = match_asset(tok) {
                if Some(&a) != to_asset.as_ref() {
                    from_asset = Some(a);
                    break;
                }
            }
        }
    }

    extracted.amount = amount.map(|n| format!("{n}"));
    extracted.source_asset = from_asset.clone();
    extracted.destination_asset = to_asset.clone();

    let chain = parse_chain(prompt).unwrap_or_else(|| {
        assumptions.push(format!("assumed chain {} (default)", input.default_chain));
        input.default_chain.clone()
    });
    extracted.source_chain = Some(chain.clone());
    extracted.destination_chain = Some(chain.clone());

    if amount.is_none() {
        clarifications.push("amount not detected; specify how much to swap".to_string());
    }
    if from_asset.is_none() {
        clarifications.push("source asset not detected".to_string());
    }
    if to_asset.is_none() {
        clarifications.push("destination asset not detected".to_string());
    }

    let intents_supported_src = from_asset
        .as_ref()
        .map(|a| INTENTS_SUPPORTED.iter().any(|s| s.eq_ignore_ascii_case(a)))
        .unwrap_or(false);
    let intents_supported_dst = to_asset
        .as_ref()
        .map(|a| INTENTS_SUPPORTED.iter().any(|s| s.eq_ignore_ascii_case(a)))
        .unwrap_or(false);

    let proposal_id = format!(
        "nl-swap-{}-{}-{}",
        from_asset.as_deref().unwrap_or("?"),
        to_asset.as_deref().unwrap_or("?"),
        amount
            .map(|n| format!("{n}"))
            .unwrap_or_else(|| "0".to_string()),
    );

    let plan = json!({
        "proposal_id": proposal_id.to_lowercase(),
        "legs": [{
            "kind": "swap",
            "chain": chain,
            "from_token": {
                "symbol": from_asset.clone().unwrap_or_default(),
                "address": null,
                "chain": chain,
                "amount": amount.map(|n| format!("{n}")).unwrap_or_default(),
                "value_usd": ""
            },
            "to_token": {
                "symbol": to_asset.clone().unwrap_or_default(),
                "address": null,
                "chain": chain,
                "amount": "TBD",
                "value_usd": ""
            },
            "description": format!(
                "Swap {} {} -> {} via NEAR Intents (compiled from prompt)",
                amount.map(|n| format!("{n}")).unwrap_or_else(|| "?".to_string()),
                from_asset.as_deref().unwrap_or("?"),
                to_asset.as_deref().unwrap_or("?")
            )
        }],
        "expected_out": {
            "symbol": to_asset.clone().unwrap_or_default(),
            "address": null,
            "chain": chain,
            "amount": "TBD",
            "value_usd": ""
        },
        "expected_cost_usd": ""
    });

    let params = json!({
        "action": "build_intent",
        "solver": "fixture",
        "plan": plan,
    });

    let gates = vec![
        NlGate {
            name: "asset_pair_recognized".to_string(),
            status: if from_asset.is_some() && to_asset.is_some() {
                "pass"
            } else {
                "warn"
            }
            .to_string(),
            detail: format!(
                "from={:?} to={:?}",
                from_asset.as_deref().unwrap_or("?"),
                to_asset.as_deref().unwrap_or("?")
            ),
        },
        NlGate {
            name: "intents_supported".to_string(),
            status: if intents_supported_src && intents_supported_dst {
                "pass"
            } else {
                "warn"
            }
            .to_string(),
            detail: format!(
                "src_in_allowlist={intents_supported_src} dst_in_allowlist={intents_supported_dst}"
            ),
        },
        NlGate {
            name: "unsigned_only".to_string(),
            status: "pass".to_string(),
            detail: "agent never signs; quote must be re-fetched before user signs".to_string(),
        },
    ];

    let confidence = if amount.is_some() && from_asset.is_some() && to_asset.is_some() {
        0.8
    } else {
        0.4
    };
    let _ = raw;
    let _ = trace;
    ("build_intent".to_string(), params, confidence, gates)
}

fn build_backtest(
    prompt: &str,
    input: &CompileInput,
    extracted: &mut Extracted,
    assumptions: &mut Vec<String>,
    clarifications: &mut Vec<String>,
    trace: &mut Vec<String>,
) -> (String, Value, f64, Vec<NlGate>) {
    let pair = parse_pair(prompt)
        .or_else(|| input.focus_pair.clone())
        .unwrap_or_else(|| {
            assumptions.push(
                "no pair detected; defaulted to NEAR/USDC (primary intents pair)".to_string(),
            );
            "NEAR/USDC".to_string()
        });
    let kind = parse_strategy_kind(prompt);
    extracted.strategy_kind = Some(kind.clone());

    let (fast, slow) = match kind.as_str() {
        "sma-cross" => parse_sma_windows(prompt, trace),
        _ => (None, None),
    };
    extracted.fast_window = fast;
    extracted.slow_window = slow;

    let mut candidates = vec![json!({
        "id": "buy_hold",
        "strategy": { "kind": "buy-hold" }
    })];
    let candidate_strategy = match kind.as_str() {
        "sma-cross" => json!({
            "kind": "sma-cross",
            "fast_window": fast.unwrap_or(5),
            "slow_window": slow.unwrap_or(20),
        }),
        "breakout" => json!({
            "kind": "breakout",
            "lookback_window": 20
        }),
        "momentum" => json!({
            "kind": "momentum",
            "lookback_window": 20
        }),
        "mean-reversion" => json!({
            "kind": "mean-reversion",
            "lookback_window": 20,
            "threshold_bps": 200
        }),
        "rsi-mean-reversion" => json!({
            "kind": "rsi-mean-reversion",
            "lookback_window": 14,
            "entry_threshold": 30,
            "exit_threshold": 50
        }),
        _ => json!({"kind": "buy-hold"}),
    };
    if kind != "buy-hold" {
        candidates.push(json!({
            "id": kind.replace('-', "_"),
            "strategy": candidate_strategy
        }));
    }

    clarifications
        .push("backtest requires OHLCV candles; provide oldest-to-newest candle array".to_string());

    let params = json!({
        "action": "backtest_suite",
        "candidates": candidates,
        "candles": [],
        "fee_bps": 10,
        "slippage_bps": 5,
        "initial_cash_usd": 1000,
        "_pair_hint": pair,
    });

    let gates = vec![
        NlGate {
            name: "strategy_recognized".to_string(),
            status: "pass".to_string(),
            detail: format!("strategy={kind}"),
        },
        NlGate {
            name: "candles_required".to_string(),
            status: "warn".to_string(),
            detail:
                "candles must be supplied separately; this is a deterministic paper test, not live"
                    .to_string(),
        },
    ];

    ("backtest_suite".to_string(), params, 0.7, gates)
}

fn build_research(
    raw: &str,
    _input: &CompileInput,
    extracted: &mut Extracted,
    _assumptions: &mut Vec<String>,
    clarifications: &mut Vec<String>,
    _trace: &mut Vec<String>,
) -> (String, Value, f64, Vec<NlGate>) {
    extracted.research_query = Some(raw.to_string());
    let pair = parse_pair(&raw.to_lowercase());
    let mut params = json!({
        "action": "plan_paid_research",
        "query": raw,
        "budget_usd": 0.0,
        "max_sources": 4,
        "spending_mode": "paper",
    });
    if let Some(p) = pair {
        params["pair"] = json!(p);
    }
    clarifications.push(
        "research mode defaults to free sources only; raise budget_usd to allow paid sources"
            .to_string(),
    );
    let gates = vec![NlGate {
        name: "free_sources_only".to_string(),
        status: "pass".to_string(),
        detail: "default budget_usd=0 — paid sources require explicit user opt-in".to_string(),
    }];
    ("plan_paid_research".to_string(), params, 0.6, gates)
}

fn build_watch(
    prompt: &str,
    _input: &CompileInput,
    extracted: &mut Extracted,
    _assumptions: &mut Vec<String>,
    clarifications: &mut Vec<String>,
    trace: &mut Vec<String>,
) -> (String, Value, f64, Vec<NlGate>) {
    let pair = parse_pair(prompt).unwrap_or_else(|| "NEAR/USDC".to_string());
    let threshold = parse_threshold(prompt, trace);
    extracted.watch_threshold = threshold.clone();

    if threshold.is_none() {
        clarifications.push(
            "watch threshold not detected; specify a price or condition (e.g. 'breakout above 70000')"
                .to_string(),
        );
    }

    let params = json!({
        "action": "format_intents_widget",
        "pair": pair,
        "mode": "paper",
        "stance": "watch",
        "risk_gates": [{
            "name": "passive-watch",
            "status": "pass",
            "detail": format!(
                "compiled from prompt; threshold={}",
                threshold.clone().unwrap_or_else(|| "unspecified".to_string())
            )
        }]
    });
    let gates = vec![NlGate {
        name: "passive".to_string(),
        status: "pass".to_string(),
        detail: "watch mode does not produce intents; only widget state".to_string(),
    }];
    ("format_intents_widget".to_string(), params, 0.55, gates)
}

// -------------------- helpers --------------------

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn match_asset(token: &str) -> Option<String> {
    let cleaned: String = token
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase();
    if cleaned.is_empty() {
        return None;
    }
    ASSETS
        .iter()
        .find(|a| **a == cleaned)
        .map(|a| (*a).to_string())
}

fn find_destination(prompt: &str, trace: &mut Vec<String>) -> Option<String> {
    for keyword in &["into ", " in ", " to ", "buy ", "stack ", "accumulate "] {
        if let Some(pos) = prompt.find(keyword) {
            let tail = &prompt[pos + keyword.len()..];
            if let Some(asset) = first_asset(tail) {
                trace.push(format!("dest matched after '{}': {asset}", keyword.trim()));
                return Some(asset);
            }
        }
    }
    first_asset(prompt)
}

fn find_source(prompt: &str, trace: &mut Vec<String>) -> Option<String> {
    for keyword in &["from ", "with ", "using "] {
        if let Some(pos) = prompt.find(keyword) {
            let tail = &prompt[pos + keyword.len()..];
            if let Some(asset) = first_asset(tail) {
                trace.push(format!(
                    "source matched after '{}': {asset}",
                    keyword.trim()
                ));
                return Some(asset);
            }
        }
    }
    None
}

fn first_asset(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .find_map(match_asset)
}

fn prompt_mentions_money(prompt: &str) -> bool {
    prompt.contains('$')
        || contains_any(
            prompt,
            &["usd", "usdc", "usdt", "dollar", "dollars", "bucks"],
        )
}

fn parse_amount(prompt: &str, trace: &mut Vec<String>) -> Option<f64> {
    let bytes = prompt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '$' || c.is_ascii_digit() {
            let start = if c == '$' { i + 1 } else { i };
            let mut end = start;
            while end < bytes.len() {
                let ch = bytes[end] as char;
                if ch.is_ascii_digit() || ch == '.' || ch == ',' {
                    end += 1;
                } else {
                    break;
                }
            }
            if end > start {
                let raw: String = prompt[start..end].chars().filter(|c| *c != ',').collect();
                if let Ok(n) = raw.parse::<f64>() {
                    let suffix = prompt[end..].trim_start();
                    let value = if suffix.starts_with('k') || suffix.starts_with('K') {
                        n * 1_000.0
                    } else if suffix.starts_with('m') || suffix.starts_with('M') {
                        n * 1_000_000.0
                    } else {
                        n
                    };
                    if value > 0.0 {
                        trace.push(format!("amount matched: {value}"));
                        return Some(value);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn parse_cadence(prompt: &str, trace: &mut Vec<String>) -> Option<String> {
    let pairs = [
        ("daily", "daily"),
        ("each day", "daily"),
        ("every day", "daily"),
        ("a day", "daily"),
        ("per day", "daily"),
        ("weekly", "weekly"),
        ("each week", "weekly"),
        ("every week", "weekly"),
        ("a week", "weekly"),
        ("per week", "weekly"),
        ("biweekly", "biweekly"),
        ("fortnightly", "biweekly"),
        ("every two weeks", "biweekly"),
        ("every 2 weeks", "biweekly"),
        ("monthly", "monthly"),
        ("each month", "monthly"),
        ("every month", "monthly"),
        ("a month", "monthly"),
        ("per month", "monthly"),
    ];
    for (needle, normalized) in pairs {
        if prompt.contains(needle) {
            trace.push(format!("cadence matched: {needle} → {normalized}"));
            return Some(normalized.to_string());
        }
    }
    None
}

fn parse_total_periods(prompt: &str, cadence: &str, trace: &mut Vec<String>) -> Option<usize> {
    // Look for "for N <unit>", "over N <unit>", or "N <unit>".
    let units = [
        ("day", 1usize),
        ("days", 1),
        ("week", 7),
        ("weeks", 7),
        ("month", 30),
        ("months", 30),
        ("year", 365),
        ("years", 365),
    ];
    let bytes = prompt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] as char == ',') {
                i += 1;
            }
            let raw: String = prompt[start..i].chars().filter(|c| *c != ',').collect();
            if let Ok(n) = raw.parse::<usize>() {
                let tail = prompt[i..].trim_start();
                for (unit, days) in units {
                    if word_starts_with(tail, unit) {
                        let total_days = n * days;
                        let periods = match cadence {
                            "daily" => total_days,
                            "weekly" => total_days / 7,
                            "biweekly" => total_days / 14,
                            "monthly" => total_days / 30,
                            _ => total_days,
                        };
                        if periods > 0 {
                            trace.push(format!(
                                "duration matched: {n} {unit} → {periods} {cadence} periods"
                            ));
                            return Some(periods);
                        }
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn word_starts_with(haystack: &str, needle: &str) -> bool {
    if !haystack.starts_with(needle) {
        return false;
    }
    match haystack.as_bytes().get(needle.len()) {
        None => true,
        Some(b) => !(*b as char).is_ascii_alphabetic(),
    }
}

fn parse_chain(prompt: &str) -> Option<String> {
    for chain in CHAINS {
        let needle = format!(" on {chain}");
        if prompt.contains(&needle) {
            return Some((*chain).to_string());
        }
    }
    None
}

fn parse_price_floor(prompt: &str) -> Option<f64> {
    if contains_any(
        prompt,
        &[
            "below ",
            "under ",
            "if it dips",
            "if discounted",
            "on the dip",
        ],
    ) {
        Some(2_000.0)
    } else {
        None
    }
}

fn parse_price_ceiling(prompt: &str) -> Option<f64> {
    if contains_any(
        prompt,
        &[
            "above ",
            "over ",
            "if stretched",
            "skip when stretched",
            "at a premium",
        ],
    ) {
        Some(1_500.0)
    } else {
        None
    }
}

fn parse_pair(prompt: &str) -> Option<String> {
    let upper = prompt.to_uppercase();
    let separators = ['/', '-'];
    for sep in separators {
        for window in upper.split_whitespace() {
            if window.contains(sep) {
                let parts: Vec<&str> = window.split(sep).collect();
                if parts.len() == 2 {
                    let a = match_asset(parts[0]);
                    let b = match_asset(parts[1]);
                    if let (Some(a), Some(b)) = (a, b) {
                        return Some(format!("{a}/{b}"));
                    }
                }
            }
        }
    }
    None
}

fn parse_strategy_kind(prompt: &str) -> String {
    if prompt.contains("rsi") {
        "rsi-mean-reversion".to_string()
    } else if prompt.contains("sma") || prompt.contains("moving average") || prompt.contains(" ma ")
    {
        "sma-cross".to_string()
    } else if prompt.contains("breakout") {
        "breakout".to_string()
    } else if prompt.contains("momentum") {
        "momentum".to_string()
    } else if prompt.contains("mean reversion") || prompt.contains("mean-reversion") {
        "mean-reversion".to_string()
    } else if prompt.contains("buy and hold") || prompt.contains("buy-and-hold") {
        "buy-hold".to_string()
    } else if prompt.contains("dca") {
        "dca".to_string()
    } else {
        "sma-cross".to_string()
    }
}

fn parse_sma_windows(prompt: &str, trace: &mut Vec<String>) -> (Option<usize>, Option<usize>) {
    // Look for "<n>/<n>" pattern.
    for window in prompt.split_whitespace() {
        if let Some((a, b)) = window.split_once('/') {
            if let (Ok(fa), Ok(sa)) = (a.parse::<usize>(), b.parse::<usize>()) {
                if fa < sa && fa > 0 && sa > 0 && sa < 500 {
                    trace.push(format!("sma windows matched: {fa}/{sa}"));
                    return (Some(fa), Some(sa));
                }
            }
        }
    }
    (None, None)
}

fn parse_threshold(prompt: &str, trace: &mut Vec<String>) -> Option<String> {
    let keywords = [
        "above ",
        "below ",
        "breakout above ",
        "drop below ",
        "cross ",
        "reaches ",
    ];
    for k in keywords {
        if let Some(pos) = prompt.find(k) {
            let tail = &prompt[pos..];
            let snippet: String = tail.chars().take(40).collect();
            trace.push(format!("threshold matched: '{snippet}'"));
            return Some(snippet);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_str(s: &str) -> CompileOutput {
        compile(CompileInput {
            prompt: s.to_string(),
            default_chain: "near".to_string(),
            default_funding_asset: "USDC".to_string(),
            focus_pair: None,
        })
        .unwrap()
    }

    #[test]
    fn dca_six_months_into_near() {
        let out = compile_str("DCA $100 weekly into NEAR for 6 months");
        assert_eq!(out.intent_kind, "dca-schedule");
        assert_eq!(out.recommended_action, "plan_dca_schedule");
        assert_eq!(out.recommended_params["pair"], "NEAR/USDC");
        assert_eq!(out.recommended_params["cadence"], "weekly");
        assert_eq!(
            out.recommended_params["total_periods"].as_u64().unwrap(),
            (6 * 30) / 7
        );
        assert_eq!(
            out.recommended_params["notional_per_period_usd"]
                .as_f64()
                .unwrap(),
            100.0
        );
        assert!(out.confidence >= 0.7);
    }

    #[test]
    fn dca_with_price_floor_sets_band() {
        let out = compile_str("dca $50 daily into NEAR if it dips below market");
        assert_eq!(out.intent_kind, "dca-schedule");
        assert!(out
            .recommended_params
            .get("opportunistic_below_discount_bps")
            .is_some());
    }

    #[test]
    fn swap_btc_to_usdc() {
        let out = compile_str("swap 0.5 BTC to USDC on near");
        assert_eq!(out.intent_kind, "swap");
        assert_eq!(out.recommended_action, "build_intent");
        let from = out.recommended_params["plan"]["legs"][0]["from_token"]["symbol"].clone();
        let to = out.recommended_params["plan"]["legs"][0]["to_token"]["symbol"].clone();
        assert_eq!(from, "BTC");
        assert_eq!(to, "USDC");
    }

    #[test]
    fn backtest_sma_5_20() {
        let out = compile_str("backtest sma 5/20 on NEAR/USDC");
        assert_eq!(out.intent_kind, "backtest");
        assert_eq!(out.recommended_action, "backtest_suite");
        let cands = out.recommended_params["candidates"].as_array().unwrap();
        assert!(cands.iter().any(|c| c["id"] == "buy_hold"));
        assert!(cands.iter().any(|c| c["strategy"]["kind"] == "sma-cross"));
        assert_eq!(out.extracted.fast_window, Some(5));
        assert_eq!(out.extracted.slow_window, Some(20));
    }

    #[test]
    fn research_extracts_pair_and_query() {
        let out = compile_str("research catalyst risk on NEAR/USDC this week");
        assert_eq!(out.intent_kind, "research");
        assert_eq!(out.recommended_params["pair"], "NEAR/USDC");
        assert_eq!(out.recommended_params["budget_usd"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn watch_keeps_threshold() {
        let out = compile_str("watch BTC for breakout above 70000");
        assert_eq!(out.intent_kind, "watch");
        assert!(out.extracted.watch_threshold.is_some());
    }

    #[test]
    fn unsupported_prompt_returns_noop() {
        let out = compile_str("hi how are you");
        assert_eq!(out.intent_kind, "unsupported");
        assert_eq!(out.recommended_action, "noop");
    }

    #[test]
    fn empty_prompt_errors() {
        let err = compile(CompileInput {
            prompt: "   ".to_string(),
            default_chain: "near".to_string(),
            default_funding_asset: "USDC".to_string(),
            focus_pair: None,
        })
        .unwrap_err();
        assert!(err.contains("non-empty"));
    }

    #[test]
    fn dca_default_assumptions_when_minimal() {
        let out = compile_str("dca into ETH");
        assert_eq!(out.intent_kind, "dca-schedule");
        assert!(out.assumptions.len() >= 2);
        assert_eq!(out.recommended_params["destination_asset"], "ETH");
    }

    #[test]
    fn dca_thousand_dollar_amount() {
        let out = compile_str("DCA $1.5k monthly into BTC for 1 year");
        assert_eq!(out.intent_kind, "dca-schedule");
        assert_eq!(
            out.recommended_params["notional_per_period_usd"]
                .as_f64()
                .unwrap(),
            1_500.0
        );
        assert_eq!(out.recommended_params["cadence"], "monthly");
        assert_eq!(
            out.recommended_params["total_periods"].as_u64().unwrap(),
            12
        );
    }
}
