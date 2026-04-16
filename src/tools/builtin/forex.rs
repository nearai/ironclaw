//! Built-in tools for USD/INR forex transfer analysis.
//!
//! Three tools that wrap the Massive API for OHLCV data and Yahoo Finance for
//! DXY direction, plus all the volatility/RSI/hit-rate math so the LLM doesn't
//! have to write Python in a repl block.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::tool::{Tool, ToolDomain, ToolError, ToolOutput, require_str};

use super::abound::{REMITTANCE_BASE, abound_get};
use super::validate_currency_code;

const MASSIVE_BASE: &str = "https://api.massive.com/v2/aggs/ticker";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

fn shared_client() -> Result<Client, ToolError> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {e}")))
}

async fn massive_bearer(secrets: &dyn SecretsStore, user_id: &str) -> Result<String, ToolError> {
    let secret = secrets
        .get_decrypted(user_id, "massive_api_key")
        .await
        .map_err(|_| {
            ToolError::NotAuthorized(
                "Missing massive_api_key. Set with: ironclaw secret set massive_api_key <KEY>"
                    .into(),
            )
        })?;
    Ok(secret.expose().to_owned())
}

/// Parse and validate a YYYY-MM-DD date string. Returns the validated string.
fn validate_date(s: &str) -> Result<String, ToolError> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| {
            ToolError::InvalidParameters(format!("Invalid date (expected YYYY-MM-DD): {s}"))
        })
}

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

fn ms_to_date_str(ms: i64) -> String {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or_default();
    epoch
        .checked_add_signed(chrono::Duration::days(ms / 86_400_000))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

// ---------------------------------------------------------------------------
// Massive API response parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Bar {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

fn parse_massive_bars(resp_body: &serde_json::Value) -> Result<Vec<Bar>, ToolError> {
    let results = resp_body
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ToolError::ExternalService("Massive API: no results returned".into()))?;

    let mut bars = Vec::with_capacity(results.len());
    for r in results {
        let close = r["c"].as_f64().filter(|&c| c > 0.0).ok_or_else(|| {
            ToolError::ExternalService("Massive API: bar missing or zero close price".into())
        })?;
        bars.push(Bar {
            date: ms_to_date_str(r["t"].as_i64().unwrap_or(0)),
            open: r["o"].as_f64().unwrap_or(0.0),
            high: r["h"].as_f64().unwrap_or(0.0),
            low: r["l"].as_f64().unwrap_or(0.0),
            close,
            volume: r["v"].as_f64().unwrap_or(0.0),
        });
    }
    Ok(bars)
}

// ---------------------------------------------------------------------------
// Yahoo Finance DXY direction
// ---------------------------------------------------------------------------

fn parse_dxy_direction(body: &serde_json::Value) -> &'static str {
    const DXY_WINDOW: usize = 5;
    let closes: Vec<f64> = (|| {
        let result = body.get("chart")?.get("result")?.as_array()?;
        let quotes = result
            .first()?
            .get("indicators")?
            .get("quote")?
            .as_array()?;
        let raw = quotes.first()?.get("close")?.as_array()?;
        Some(raw.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>())
    })()
    .unwrap_or_default();

    if closes.len() < DXY_WINDOW + 1 {
        return "unknown";
    }
    let change = closes[closes.len() - 1] / closes[closes.len() - DXY_WINDOW - 1] - 1.0;
    if change >= 0.0 { "up" } else { "down" }
}

// ---------------------------------------------------------------------------
// Volatility, RSI, hit-rate cube, projection cone
// ---------------------------------------------------------------------------

fn log_returns(closes: &[f64]) -> Vec<f64> {
    closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect()
}

fn sample_std(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    variance.sqrt()
}

fn vol_bucket(vol: f64) -> &'static str {
    if vol < 0.001_31 {
        "very_low"
    } else if vol > 0.003_74 {
        "very_high"
    } else {
        "normal"
    }
}

fn rsi(closes: &[f64], period: usize) -> Option<f64> {
    if closes.len() < period + 1 {
        return None;
    }
    let mut avg_g: f64 = 0.0;
    let mut avg_l: f64 = 0.0;
    for i in 1..=period {
        let d = closes[i] - closes[i - 1];
        avg_g += d.max(0.0);
        avg_l += (-d).max(0.0);
    }
    avg_g /= period as f64;
    avg_l /= period as f64;

    for i in (period + 1)..closes.len() {
        let d = closes[i] - closes[i - 1];
        avg_g = (avg_g * (period - 1) as f64 + d.max(0.0)) / period as f64;
        avg_l = (avg_l * (period - 1) as f64 + (-d).max(0.0)) / period as f64;
    }

    if avg_l == 0.0 {
        return Some(100.0);
    }
    Some(100.0 - 100.0 / (1.0 + avg_g / avg_l))
}

fn rsi_bucket(r: f64) -> &'static str {
    if r < 50.0 {
        "low"
    } else if r <= 70.0 {
        "mid"
    } else {
        "high"
    }
}

fn hit_rate(vol: &str, rsi_b: &str, dxy: &str) -> f64 {
    let avg = |a: f64, b: f64| (a + b) / 2.0;
    match (vol, rsi_b, dxy) {
        ("very_low", "low", "up") => 43.7,
        ("very_low", "low", "down") => 32.2,
        ("very_low", "low", _) => avg(43.7, 32.2),
        ("very_low", "mid", "up") => 45.3,
        ("very_low", "mid", "down") => 34.9,
        ("very_low", "mid", _) => avg(45.3, 34.9),
        ("very_low", "high", "up") => 53.4,
        ("very_low", "high", "down") => 44.7,
        ("very_low", "high", _) => avg(53.4, 44.7),

        ("normal", "low", "up") => 35.0,
        ("normal", "low", "down") => 33.5,
        ("normal", "low", _) => avg(35.0, 33.5),
        ("normal", "mid", "up") => 44.7,
        ("normal", "mid", "down") => 36.8,
        ("normal", "mid", _) => avg(44.7, 36.8),
        ("normal", "high", "up") => 52.9,
        ("normal", "high", "down") => 47.0,
        ("normal", "high", _) => avg(52.9, 47.0),

        ("very_high", "low", "up") => 37.9,
        ("very_high", "low", "down") => 36.5,
        ("very_high", "low", _) => avg(37.9, 36.5),
        ("very_high", "mid", "up") => 40.0,
        ("very_high", "mid", "down") => 32.5,
        ("very_high", "mid", _) => avg(40.0, 32.5),
        ("very_high", "high", "up") => 53.1,
        ("very_high", "high", "down") => 41.2,
        ("very_high", "high", _) => avg(53.1, 41.2),

        _ => 40.0,
    }
}

fn regime_k(vb: &str) -> f64 {
    match vb {
        "very_low" => 1.0,
        "very_high" => 0.5,
        _ => 0.75,
    }
}

fn compute_cone(
    current_rate: f64,
    daily_vol: f64,
    vb: &str,
    today: chrono::NaiveDate,
) -> (f64, Vec<serde_json::Value>) {
    const CONE_Z: f64 = 1.645;
    const HORIZON_DAYS: i64 = 3;

    let k = regime_k(vb);
    let target_rate = current_rate * (k * daily_vol).exp();

    let mut projection = Vec::new();
    for t in 0..=HORIZON_DAYS {
        let date_str = (today + chrono::Duration::days(t)).format("%Y-%m-%d").to_string();
        let center = current_rate + (target_rate - current_rate) * t as f64 / HORIZON_DAYS as f64;
        let (upper, lower) = if t == 0 {
            (current_rate, current_rate)
        } else {
            let spread = CONE_Z * daily_vol * (t as f64).sqrt();
            (current_rate * spread.exp(), current_rate * (-spread).exp())
        };
        projection.push(json!({
            "date": date_str,
            "center": center,
            "upper": upper,
            "lower": lower,
        }));
    }
    (target_rate, projection)
}

// ---------------------------------------------------------------------------
// Return percentile table (USD/INR calibrated)
// ---------------------------------------------------------------------------

type PercentileRow = (f64, f64); // (percentile, return)

struct HorizonTable {
    days: i32,
    rows: &'static [PercentileRow],
}

static RETURN_PERCENTILES: &[HorizonTable] = &[
    HorizonTable {
        days: 3,
        rows: &[
            (0.01, -0.016_899),
            (0.05, -0.007_893),
            (0.10, -0.005_139),
            (0.25, -0.001_845),
            (0.50, 0.000_113),
            (0.75, 0.002_414),
            (0.90, 0.005_981),
            (0.95, 0.009_035),
            (0.99, 0.019_432),
        ],
    },
    HorizonTable {
        days: 7,
        rows: &[
            (0.01, -0.023_974),
            (0.05, -0.012_192),
            (0.10, -0.007_996),
            (0.25, -0.002_875),
            (0.50, 0.000_299),
            (0.75, 0.004_106),
            (0.90, 0.009_496),
            (0.95, 0.014_416),
            (0.99, 0.028_761),
        ],
    },
    HorizonTable {
        days: 30,
        rows: &[
            (0.01, -0.053_051),
            (0.05, -0.026_386),
            (0.10, -0.017_315),
            (0.25, -0.006_388),
            (0.50, 0.001_127),
            (0.75, 0.011_128),
            (0.90, 0.023_445),
            (0.95, 0.036_908),
            (0.99, 0.069_213),
        ],
    },
    HorizonTable {
        days: 90,
        rows: &[
            (0.01, -0.072_050),
            (0.05, -0.041_678),
            (0.10, -0.029_833),
            (0.25, -0.011_556),
            (0.50, 0.005_127),
            (0.75, 0.022_799),
            (0.90, 0.044_823),
            (0.95, 0.070_926),
            (0.99, 0.140_583),
        ],
    },
    HorizonTable {
        days: 180,
        rows: &[
            (0.01, -0.109_389),
            (0.05, -0.056_501),
            (0.10, -0.041_380),
            (0.25, -0.013_188),
            (0.50, 0.012_416),
            (0.75, 0.038_021),
            (0.90, 0.073_889),
            (0.95, 0.114_706),
            (0.99, 0.180_774),
        ],
    },
    HorizonTable {
        days: 365,
        rows: &[
            (0.01, -0.150_317),
            (0.05, -0.077_316),
            (0.10, -0.054_906),
            (0.25, -0.017_094),
            (0.50, 0.030_325),
            (0.75, 0.073_258),
            (0.90, 0.111_315),
            (0.95, 0.169_161),
            (0.99, 0.225_594),
        ],
    },
];

fn hit_rate_from_percentiles(required_move: f64, table: &[PercentileRow]) -> f64 {
    let idx = table.iter().position(|&(_, ret)| ret >= required_move);
    match idx {
        None => 0.0,
        Some(0) => 100.0,
        Some(i) => {
            let (lo_pct, lo_ret) = table[i - 1];
            let (hi_pct, hi_ret) = table[i];
            let frac = if (hi_ret - lo_ret).abs() > f64::EPSILON {
                (required_move - lo_ret) / (hi_ret - lo_ret)
            } else {
                0.0
            };
            let at_pct = lo_pct + frac * (hi_pct - lo_pct);
            ((1.0 - at_pct) * 1000.0).round() / 10.0
        }
    }
}

// ===========================================================================
// forex_historical_data
// ===========================================================================

pub struct ForexHistoricalDataTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl ForexHistoricalDataTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for ForexHistoricalDataTool {
    fn name(&self) -> &str {
        "forex_historical_data"
    }

    fn description(&self) -> &str {
        "Fetch historical OHLCV forex data for a currency pair from the Massive API."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "from_currency": {
                    "type": "string",
                    "description": "Source currency code, e.g. USD"
                },
                "to_currency": {
                    "type": "string",
                    "description": "Target currency code, e.g. INR"
                },
                "start_date": {
                    "type": "string",
                    "description": "Start date in YYYY-MM-DD format"
                },
                "end_date": {
                    "type": "string",
                    "description": "End date in YYYY-MM-DD format (defaults to today)"
                }
            },
            "required": ["from_currency", "to_currency", "start_date"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let from = validate_currency_code(require_str(&params, "from_currency")?)?;
        let to = validate_currency_code(require_str(&params, "to_currency")?)?;
        let start_date = validate_date(require_str(&params, "start_date")?)?;
        let end_date = match params.get("end_date").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => validate_date(s)?,
            _ => chrono::Utc::now().format("%Y-%m-%d").to_string(),
        };

        let pair = format!("{from}{to}");
        let url = format!(
            "{MASSIVE_BASE}/C:{pair}/range/1/day/{start_date}/{end_date}?sort=asc&limit=5000"
        );

        let bearer = massive_bearer(&*self.secrets, &ctx.user_id).await?;

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::ExternalService(format!(
                "Massive API HTTP {status}: {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;

        let bars = parse_massive_bars(&body)?;
        let bars_json: Vec<serde_json::Value> = bars
            .iter()
            .map(|b| {
                json!({
                    "date": b.date,
                    "open": b.open,
                    "high": b.high,
                    "low": b.low,
                    "close": b.close,
                    "volume": b.volume,
                })
            })
            .collect();

        Ok(ToolOutput::success(json!(bars_json), start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

// ===========================================================================
// analyze_transfer
// ===========================================================================

pub struct AnalyzeTransferTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl AnalyzeTransferTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

/// Core analysis logic shared by `AnalyzeTransferTool` and `AboundSendWireTool`.
///
/// Fetches Massive + DXY data, computes indicators, and returns the structured
/// `{"message": "...", "plot": {...}}` result.
pub async fn run_transfer_analysis(
    client: &Client,
    secrets: &dyn SecretsStore,
    user_id: &str,
    amount: Option<f64>,
    for_wire: bool,
) -> Result<serde_json::Value, ToolError> {
    let now = chrono::Utc::now();
    let today = now.date_naive();
    let start_str = (today - chrono::Duration::days(220)).format("%Y-%m-%d").to_string();
    let end_str = today.format("%Y-%m-%d").to_string();

    let massive_url = format!(
        "{MASSIVE_BASE}/C:USDINR/range/1/day/{start_str}/{end_str}?sort=asc&limit=5000"
    );
    let now_unix = now.timestamp();
    let dxy_url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/DX-Y.NYB?interval=1d&period1={}&period2={now_unix}",
        now_unix - 35 * 86400,
    );

    let bearer = massive_bearer(secrets, user_id).await?;

    let abound_rate_url = format!(
        "{REMITTANCE_BASE}/exchange-rate?from_currency=USD&to_currency=INR"
    );
    let abound_rate_fut = async {
        let result = abound_get(client, secrets, user_id, &abound_rate_url).await.ok()?;
        result
            .get("body")
            .and_then(|b| b.get("data"))
            .and_then(|d| d.get("effective_exchange_rate"))
            .and_then(|r| {
                r.get("value")
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        r.get("formatted_value")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<f64>().ok())
                    })
            })
            .filter(|&r| r > 0.0)
    };

    let (massive_resp, dxy_resp, abound_effective_rate) = tokio::join!(
        client
            .get(&massive_url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send(),
        client
            .get(&dxy_url)
            .header("User-Agent", "Mozilla/5.0")
            .send(),
        abound_rate_fut,
    );

    let massive_resp = massive_resp.map_err(|e| ToolError::ExternalService(e.to_string()))?;
    if massive_resp.status().as_u16() != 200 {
        return Err(ToolError::ExternalService(format!(
            "Massive API HTTP {}",
            massive_resp.status()
        )));
    }
    let massive_body: serde_json::Value = massive_resp
        .json()
        .await
        .map_err(|e| ToolError::ExternalService(e.to_string()))?;
    let bars = parse_massive_bars(&massive_body)?;
    if bars.len() < 23 {
        return Err(ToolError::ExternalService(format!(
            "Insufficient data: need 23 bars, got {}",
            bars.len()
        )));
    }

    let dxy_dir = match dxy_resp {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(body) => parse_dxy_direction(&body),
            Err(_) => "unknown",
        },
        Err(_) => "unknown",
    };

    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let vol_window = 20;
    let vol_slice = &closes[closes.len().saturating_sub(vol_window)..];
    let daily_vol = sample_std(&log_returns(vol_slice));
    let vb = vol_bucket(daily_vol);

    let rsi_val = rsi(&closes, 14);
    let rsi_rounded = rsi_val.map(|r| (r * 10.0).round() / 10.0);
    let rb = rsi_val.map(rsi_bucket).unwrap_or("mid");

    let hr = hit_rate(vb, rb, dxy_dir);
    let market_rate = closes[closes.len() - 1];

    let (market_target, projection) = compute_cone(market_rate, daily_vol, vb, today);
    let pct_move = market_target / market_rate - 1.0;

    let current_rate = abound_effective_rate.unwrap_or(market_rate);
    let target_rate = current_rate * (1.0 + pct_move);
    let recommend = if hr < 45.0 { "now" } else { "wait" };

    let historical: Vec<serde_json::Value> = bars[bars.len().saturating_sub(30)..]
        .iter()
        .map(|b| json!({"date": b.date, "close": b.close}))
        .collect();

    let could_save = amount.map(|a| ((target_rate - current_rate) * a * 100.0).round() / 100.0);

    let action_verb = if recommend == "now" {
        "Transfer now"
    } else {
        "Wait — hold off"
    };
    let mut message = format!(
        "{action_verb}. USD/INR is at {current_rate:.4}; target {:.4}. \
         Regime: {vb} volatility, RSI {} ({rb}), DXY {dxy_dir}. \
         Hit-rate for this regime: {:.1}%.",
        (target_rate * 10000.0).round() / 10000.0,
        rsi_rounded
            .map(|r| format!("{r}"))
            .unwrap_or_else(|| "N/A".into()),
        (hr * 10.0).round() / 10.0,
    );
    if recommend == "wait" {
        if let Some(save) = could_save
            && save > 0.0
        {
            message.push_str(&format!(
                " If you wait, you could get ₹{save:.2} more on your transfer."
            ));
        }
    }
    if for_wire {
        message.push_str(
            " — Send now or wait? Reply **send now** to proceed with the transfer, \
             or **wait** to hold off for a better rate.",
        );
    }

    Ok(json!({
        "message": message,
        "plot": {
            "historical": historical,
            "projection": projection,
            "current_rate": current_rate,
            "target_rate": (target_rate * 10000.0).round() / 10000.0,
            "vol_regime": vb,
            "daily_vol": (daily_vol * 1_000_000.0).round() / 1_000_000.0,
            "rsi": rsi_rounded,
            "dxy_direction": dxy_dir,
            "hit_rate_pct": (hr * 10.0).round() / 10.0,
            "recommend": recommend,
            "could_save": could_save.map(|s| format!("{s:.2} INR")),
        }
    }))
}

#[async_trait]
impl Tool for AnalyzeTransferTool {
    fn name(&self) -> &str {
        "analyze_transfer"
    }

    fn description(&self) -> &str {
        "Analyze whether to transfer USD to INR now or wait. Uses volatility regime, RSI(14), \
         and DXY momentum to compute a hit rate, target rate, and 3-day projection cone. \
         Returns {\"message\": \"...\", \"plot\": {...}}. USD/INR only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "amount": {
                    "type": "number",
                    "description": "Optional USD amount the user intends to send. Used to compute potential INR savings."
                },
                "for_wire": {
                    "type": "boolean",
                    "description": "Set to true when called as part of a wire transfer flow. The message will include an explicit send-or-wait approval prompt."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let timer = Instant::now();
        let amount = params.get("amount").and_then(|v| v.as_f64());
        let for_wire = params
            .get("for_wire")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let result =
            run_transfer_analysis(&self.client, &*self.secrets, &ctx.user_id, amount, for_wire)
                .await?;

        Ok(ToolOutput::success(result, timer.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

// ===========================================================================
// validate_transfer_target
// ===========================================================================

pub struct ValidateTransferTargetTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    client: Client,
}

impl ValidateTransferTargetTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Result<Self, ToolError> {
        Ok(Self {
            secrets,
            client: shared_client()?,
        })
    }
}

#[async_trait]
impl Tool for ValidateTransferTargetTool {
    fn name(&self) -> &str {
        "validate_transfer_target"
    }

    fn description(&self) -> &str {
        "Given a desired USD/INR rate, compute the probability of hitting it across \
         6 time horizons (3d, 7d, 30d, 90d, 180d, 365d). \
         Returns {\"message\": \"...\", \"plot\": {...}}. USD/INR only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "target_rate": {
                    "type": "number",
                    "description": "The desired USD/INR exchange rate"
                }
            },
            "required": ["target_rate"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let timer = Instant::now();
        let target_rate_input = params
            .get("target_rate")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParameters("target_rate must be a number".into()))?;

        let now = chrono::Utc::now();
        let today = now.date_naive();
        let start_str = (today - chrono::Duration::days(5)).format("%Y-%m-%d").to_string();
        let today_str = today.format("%Y-%m-%d").to_string();

        let url = format!(
            "{MASSIVE_BASE}/C:USDINR/range/1/day/{start_str}/{today_str}?sort=asc&limit=10"
        );

        let bearer = massive_bearer(&*self.secrets, &ctx.user_id).await?;

        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;

        if resp.status().as_u16() != 200 {
            return Err(ToolError::ExternalService(format!(
                "Massive API HTTP {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;
        let bars = parse_massive_bars(&body)?;
        if bars.is_empty() {
            return Err(ToolError::ExternalService(
                "Massive API returned no bars".into(),
            ));
        }

        // close is guaranteed > 0.0 by parse_massive_bars; is_empty() check above ensures last() is Some
        let current_rate = bars
            .last()
            .ok_or_else(|| ToolError::ExternalService("Massive API returned no bars".into()))?
            .close;
        let required_move = (target_rate_input / current_rate).ln();

        let mut horizons = Vec::new();
        let mut rec_horizon: Option<i32> = None;
        for entry in RETURN_PERCENTILES {
            let hr = hit_rate_from_percentiles(required_move, entry.rows);
            horizons.push(json!({
                "horizon_days": entry.days,
                "hit_rate_pct": hr,
            }));
            if rec_horizon.is_none() && hr >= 20.0 {
                rec_horizon = Some(entry.days);
            }
        }

        let horizon_note = match rec_horizon {
            Some(d) => format!("earliest horizon with ≥20% probability: {d}d"),
            None => "no horizon reaches 20% probability".into(),
        };

        let message = format!(
            "USD/INR is at {current_rate:.4}. Hitting {target_rate_input:.4} needs a \
             {:.4}% move ({horizon_note}).",
            (required_move * 100.0 * 10000.0).round() / 10000.0,
        );

        let result = json!({
            "message": message,
            "plot": {
                "current_rate": current_rate,
                "target_rate": (target_rate_input * 10000.0).round() / 10000.0,
                "required_move_pct": (required_move * 100.0 * 10000.0).round() / 10000.0,
                "horizons": horizons,
                "recommended_horizon_days": rec_horizon,
            }
        });

        Ok(ToolOutput::success(result, timer.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ms_to_date_str() {
        assert_eq!(ms_to_date_str(0), "1970-01-01");
        assert_eq!(ms_to_date_str(86_400_000), "1970-01-02");
        assert_eq!(ms_to_date_str(1_775_606_400_000), "2026-04-08");
    }

    #[test]
    fn test_vol_bucket_boundaries() {
        assert_eq!(vol_bucket(0.001), "very_low");
        assert_eq!(vol_bucket(0.002), "normal");
        assert_eq!(vol_bucket(0.004), "very_high");
    }

    #[test]
    fn test_rsi_all_gains() {
        // Monotonically increasing closes → RSI should be 100
        let closes: Vec<f64> = (0..20).map(|i| 80.0 + i as f64).collect();
        let r = rsi(&closes, 14).unwrap();
        assert!((r - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_rsi_insufficient_data() {
        let closes = vec![1.0, 2.0, 3.0];
        assert!(rsi(&closes, 14).is_none());
    }

    #[test]
    fn test_hit_rate_cube_lookup() {
        assert!((hit_rate("normal", "mid", "up") - 44.7).abs() < f64::EPSILON);
        assert!((hit_rate("very_low", "high", "down") - 44.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hit_rate_from_percentiles_extremes() {
        let table: &[PercentileRow] = &[(0.01, -0.016_899), (0.50, 0.000_113), (0.99, 0.019_432)];
        // Very negative move → 100% chance of exceeding
        assert!((hit_rate_from_percentiles(-1.0, table) - 100.0).abs() < f64::EPSILON);
        // Very large move → 0% chance
        assert!((hit_rate_from_percentiles(1.0, table)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dxy_direction_up() {
        let body = json!({
            "chart": {
                "result": [{
                    "indicators": {
                        "quote": [{
                            "close": [100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0]
                        }]
                    }
                }]
            }
        });
        assert_eq!(parse_dxy_direction(&body), "up");
    }

    #[test]
    fn test_dxy_direction_down() {
        let body = json!({
            "chart": {
                "result": [{
                    "indicators": {
                        "quote": [{
                            "close": [106.0, 105.0, 104.0, 103.0, 102.0, 101.0, 100.0]
                        }]
                    }
                }]
            }
        });
        assert_eq!(parse_dxy_direction(&body), "down");
    }

    #[test]
    fn test_dxy_direction_insufficient_data() {
        let body = json!({"chart": {"result": [{"indicators": {"quote": [{"close": [100.0]}]}}]}});
        assert_eq!(parse_dxy_direction(&body), "unknown");
    }

    #[test]
    fn test_log_returns() {
        let closes = vec![100.0, 110.0, 121.0];
        let rets = log_returns(&closes);
        assert_eq!(rets.len(), 2);
        assert!((rets[0] - (1.1_f64).ln()).abs() < 1e-10);
    }

    #[test]
    fn test_sample_std_single() {
        assert!((sample_std(&[42.0])).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_cone_day_zero() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 4, 8).unwrap();
        let (_, projection) = compute_cone(85.0, 0.002, "normal", today);
        let day0 = &projection[0];
        assert!((day0["upper"].as_f64().unwrap() - 85.0).abs() < f64::EPSILON);
        assert!((day0["lower"].as_f64().unwrap() - 85.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_validate_currency_code() {
        assert_eq!(validate_currency_code("USD").unwrap(), "USD");
        assert_eq!(validate_currency_code("inr").unwrap(), "INR");
        assert!(validate_currency_code("USD/../../admin").is_err());
        assert!(validate_currency_code("AB").is_err());
        assert!(validate_currency_code("").is_err());
    }

    #[test]
    fn test_validate_date() {
        assert_eq!(validate_date("2026-04-08").unwrap(), "2026-04-08");
        assert!(validate_date("not-a-date").is_err());
        assert!(validate_date("2026-01-01/../../admin").is_err());
    }

    #[test]
    fn test_parse_massive_bars_rejects_zero_close() {
        let body = json!({
            "results": [{"t": 1000000000000_i64, "o": 1.0, "h": 2.0, "l": 0.5, "c": 0.0, "v": 100.0}]
        });
        assert!(parse_massive_bars(&body).is_err());
    }
}
