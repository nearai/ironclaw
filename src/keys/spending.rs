//! Daily spend tracking for rate-limiting value transfers.
//!
//! Tracks cumulative daily spend in yoctoNEAR to enforce `daily_spend_limit_yocto`.
//! Persisted to `~/.ironclaw/spend_tracking.json`. Resets automatically at midnight UTC.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::keys::KeyError;
use crate::keys::types::format_yocto;

/// Tracks daily cumulative spend for policy enforcement.
pub struct SpendTracker {
    path: PathBuf,
}

impl SpendTracker {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Default location: `~/.ironclaw/spend_tracking.json`
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".ironclaw").join("spend_tracking.json"))
            .unwrap_or_else(|| PathBuf::from(".ironclaw/spend_tracking.json"))
    }

    /// Get today's cumulative spend in yoctoNEAR.
    pub async fn get_daily_spend(&self) -> Result<u128, KeyError> {
        let data = self.load().await?;
        let today = Utc::now().date_naive();

        Ok(data
            .records
            .iter()
            .find(|r| r.date == today)
            .map(|r| r.total_spent_yocto)
            .unwrap_or(0))
    }

    /// Record a spend after successful transaction submission.
    pub async fn record_spend(
        &self,
        value_yocto: u128,
        description: String,
        tx_hash: Option<String>,
    ) -> Result<(), KeyError> {
        let mut data = self.load().await?;
        let today = Utc::now().date_naive();

        let record = data.records.iter_mut().find(|r| r.date == today);

        let entry = SpendEntry {
            timestamp: Utc::now(),
            tx_hash,
            value_yocto,
            description,
        };

        if let Some(record) = record {
            record.total_spent_yocto = record.total_spent_yocto.saturating_add(value_yocto);
            record.transactions.push(entry);
        } else {
            data.records.push(SpendRecord {
                date: today,
                total_spent_yocto: value_yocto,
                transactions: vec![entry],
            });
        }

        // Keep only last 30 days of records
        let cutoff = Utc::now().date_naive() - chrono::Duration::days(30);
        data.records.retain(|r| r.date >= cutoff);

        self.save(&data).await
    }

    /// Get spend history for the last N days.
    pub async fn get_history(&self, days: u32) -> Result<Vec<SpendRecord>, KeyError> {
        let data = self.load().await?;
        let cutoff = Utc::now().date_naive() - chrono::Duration::days(days as i64);

        Ok(data
            .records
            .into_iter()
            .filter(|r| r.date >= cutoff)
            .collect())
    }

    async fn load(&self) -> Result<SpendData, KeyError> {
        if !self.path.exists() {
            return Ok(SpendData::default());
        }

        let content = fs::read_to_string(&self.path).await?;
        serde_json::from_str(&content).map_err(|e| {
            KeyError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("corrupt spend tracking data: {}", e),
            ))
        })
    }

    async fn save(&self, data: &SpendData) -> Result<(), KeyError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(data).map_err(|e| {
            KeyError::SerializationFailed(format!("failed to serialize spend data: {}", e))
        })?;

        fs::write(&self.path, content).await?;
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SpendData {
    records: Vec<SpendRecord>,
}

/// A day's spend record with audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRecord {
    pub date: NaiveDate,
    pub total_spent_yocto: u128,
    pub transactions: Vec<SpendEntry>,
}

impl std::fmt::Display for SpendRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} ({} txns)",
            self.date,
            format_yocto(self.total_spent_yocto),
            self.transactions.len()
        )
    }
}

/// A single spend entry in the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendEntry {
    pub timestamp: DateTime<Utc>,
    pub tx_hash: Option<String>,
    pub value_yocto: u128,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::keys::spending::SpendTracker;

    #[tokio::test]
    async fn test_empty_spend() {
        let dir = TempDir::new().unwrap();
        let tracker = SpendTracker::new(dir.path().join("spend.json"));
        assert_eq!(tracker.get_daily_spend().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_record_and_query_spend() {
        let dir = TempDir::new().unwrap();
        let tracker = SpendTracker::new(dir.path().join("spend.json"));

        tracker
            .record_spend(
                1_000_000,
                "test transfer".to_string(),
                Some("hash1".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(tracker.get_daily_spend().await.unwrap(), 1_000_000);

        tracker
            .record_spend(2_000_000, "another transfer".to_string(), None)
            .await
            .unwrap();

        assert_eq!(tracker.get_daily_spend().await.unwrap(), 3_000_000);
    }

    #[tokio::test]
    async fn test_get_history() {
        let dir = TempDir::new().unwrap();
        let tracker = SpendTracker::new(dir.path().join("spend.json"));

        tracker
            .record_spend(100, "test".to_string(), None)
            .await
            .unwrap();

        let history = tracker.get_history(7).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].total_spent_yocto, 100);
        assert_eq!(history[0].transactions.len(), 1);
    }
}
