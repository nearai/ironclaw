//! Telegram outbound delivery ownership.

mod protocol;
mod targets;

pub use protocol::TelegramDeliveryProtocol;
pub use targets::TelegramOutboundTargetProvider;
