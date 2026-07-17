//! Telegram outbound delivery ownership.

mod protocol;
mod targets;
mod triggered;

pub use protocol::TelegramDeliveryProtocol;
pub use targets::TelegramOutboundTargetProvider;
pub use triggered::DynamicTelegramTriggeredRunDeliveryHook;
