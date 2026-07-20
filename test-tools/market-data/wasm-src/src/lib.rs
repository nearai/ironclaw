//! Market Data WASM tool for IronClaw (#5459 test fixture).
//!
//! Returns a *fake* S&P 500 (SPX) snapshot. It exists to demonstrate the
//! admin-install → shared-tool → user-trigger flow: an admin imports this tool
//! via the WebUI "Install Tool" button, activates it, and then any user can ask
//! the agent about the market and have it call this capability.
//!
//! Its manifest declares `network` + a REQUIRED `market_data_api_key` runtime
//! credential (host-injected as an `x-api-key` header at egress), so dispatch
//! gates with AuthRequired until a key is provisioned — personal or
//! tenant-shared (env-seeded via `IRONCLAW_REBORN_DEV_SECRET__…` or the admin
//! API). The wasm itself still returns canned data and performs no real
//! egress: the credential exists to exercise the host-side obligation
//! pipeline (pre-flight, gating, injection), not to authenticate anything.

mod types;

use types::Snp500Snapshot;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct MarketDataTool;

impl exports::near::agent::tool::Guest for MarketDataTool {
    fn execute(_req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        crate::near::agent::host::log(
            crate::near::agent::host::LogLevel::Info,
            "market-data.snp500: returning fake SPX snapshot",
        );

        let snapshot = Snp500Snapshot {
            symbol: "SPX".to_string(),
            name: "S&P 500".to_string(),
            price: 5_487.03,
            change: 12.45,
            change_percent: 0.23,
            previous_close: 5_474.58,
            day_high: 5_492.10,
            day_low: 5_468.77,
            as_of: "2026-06-30T20:00:00Z".to_string(),
            data_source: "market_data_api (fake fixture data)".to_string(),
        };

        match serde_json::to_string(&snapshot) {
            Ok(output) => exports::near::agent::tool::Response {
                output: Some(output),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(format!("failed to serialize snapshot: {error}")),
            },
        }
    }

    fn schema() -> String {
        r#"{"type":"object","properties":{},"additionalProperties":false}"#.to_string()
    }

    fn description() -> String {
        "Get the current S&P 500 (SPX) market snapshot — index level, daily change, and percent \
         change. Returns fixture data (no live feed). Takes no arguments. Use this whenever the \
         user asks about the S&P 500, SPX, or the stock market level."
            .to_string()
    }
}

export!(MarketDataTool);
