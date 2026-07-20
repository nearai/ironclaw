//! Telegram Bot API updates ingress for the Reborn product-adapter path.

pub mod dispatch;
mod resolver;
mod route;

pub use resolver::{
    DynamicTelegramInstallationResolver, ResolvedTelegramInstallation,
    TELEGRAM_SECRET_TOKEN_HEADER, TelegramIngressError, TelegramRevisionWorkflow,
    TelegramRevisionWorkflowBuildError, TelegramRevisionWorkflowBuilder,
    TelegramUpdatesWebhookDispatcher,
};
pub use route::{
    TELEGRAM_UPDATES_PATH, TelegramIngressService, TelegramUpdatesRouteState,
    telegram_updates_route_descriptors, telegram_updates_route_parts,
};

#[cfg(test)]
pub(crate) use route::{TELEGRAM_UPDATES_ROUTE_ID, ingress_error_response, runner_error_response};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
