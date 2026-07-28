use serde::Serialize;

/// A fake S&P 500 snapshot returned by `market-data.snp500`.
#[derive(Debug, Serialize)]
pub struct Snp500Snapshot {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub change: f64,
    pub change_percent: f64,
    pub previous_close: f64,
    pub day_high: f64,
    pub day_low: f64,
    pub as_of: String,
    /// Nominal data feed label.
    pub data_source: String,
}
