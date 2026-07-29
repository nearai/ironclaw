//! Compatibility re-export for product-adapter contracts.

pub use ironclaw_host_api::product_adapter::*;

#[cfg(any(test, feature = "test-support"))]
pub mod fakes;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use ironclaw_host_api::product_adapter::test_support::*;
}

#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeOutboundDeliverySink, FakeProjectionStream, FakeProtocolHttpEgress, RecordedEgressCall,
};
