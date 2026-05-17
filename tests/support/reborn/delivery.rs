use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_product_adapters::{DeliveryStatus, OutboundDeliverySink};

#[derive(Debug, Clone, Default)]
pub struct RecordingOutboundDeliverySink {
    statuses: Arc<Mutex<Vec<DeliveryStatus>>>,
}

impl RecordingOutboundDeliverySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn statuses(&self) -> Vec<DeliveryStatus> {
        self.statuses
            .lock()
            .expect("delivery sink lock poisoned")
            .clone()
    }
}

#[async_trait]
impl OutboundDeliverySink for RecordingOutboundDeliverySink {
    async fn record(&self, status: DeliveryStatus) {
        self.statuses
            .lock()
            .expect("delivery sink lock poisoned")
            .push(status);
    }
}
