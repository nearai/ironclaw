//! Product-neutral live and triggered channel delivery.

#![forbid(unsafe_code)]

mod actionable;
mod hooks;
mod observer;
mod routing;
mod services;
mod triggered;

pub use hooks::{
    CompositePostSubmitDeliveryHook, NoopPostSubmitDeliveryHook, PostSubmitDeliveryError,
    PostSubmitDeliveryHook,
};
pub use observer::FinalReplyDeliveryObserver;
pub use services::{
    ChannelDeliveryProtocol, FinalReplyDeliveryError, FinalReplyDeliveryServices,
    FinalReplyDeliverySettings, PostedChannelMessage,
};
pub use triggered::TriggeredRunDeliveryDriver;

#[cfg(test)]
pub(crate) use actionable::*;
#[cfg(test)]
pub(crate) use routing::*;
#[cfg(test)]
pub(crate) use services::*;
#[cfg(test)]
pub(crate) use triggered::*;

#[cfg(test)]
include!("tests.rs");
