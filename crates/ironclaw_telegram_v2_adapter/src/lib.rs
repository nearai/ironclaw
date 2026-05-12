//! Telegram v2 ProductAdapter payload normalization (issue #3285 tracer-bullet).
//!
//! This slice only lands Telegram Bot API payload parsing and Reborn external
//! reference normalization. The ProductAdapter implementation and outbound
//! rendering land in the next slice.

#![forbid(unsafe_code)]

pub mod payload;

pub use payload::{
    GroupTriggerPolicy, PayloadParseError, TELEGRAM_API_HOST, TELEGRAM_FILE_API_HOST,
    TELEGRAM_USER_ACTOR_KIND, TelegramParsedInbound, parse_telegram_update,
};
