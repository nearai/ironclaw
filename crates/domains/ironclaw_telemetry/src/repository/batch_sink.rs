//! Private worker seam for routing one already-scoped batch into persistence.

use super::*;
use async_trait::async_trait;

#[async_trait]
impl<F> TelemetryBatchSink for FilesystemTelemetryRepository<F>
where
    F: ironclaw_filesystem::RootFilesystem + ?Sized,
{
    async fn apply_batch(
        &self,
        batch: ScopedTelemetryBatch,
    ) -> Result<BatchApplyReport, TelemetryRepositoryError> {
        FilesystemTelemetryRepository::apply_batch(self, batch).await
    }
}
