#[cfg(feature = "webui-v2-beta")]
mod agent_link;
mod capabilities;
mod catalog;
mod errors;
#[cfg(feature = "webui-v2-beta")]
mod link_service;
mod model;
mod package;
mod render;
mod service;

#[cfg(test)]
mod tests;

pub(crate) use capabilities::{extend_builtin_first_party_package, insert_handlers};
#[cfg(feature = "webui-v2-beta")]
pub(crate) use link_service::RebornIronhubLinkService;
#[cfg(test)]
pub(crate) use model::{
    IRONHUB_INFO_CAPABILITY_ID, IRONHUB_INSTALL_CAPABILITY_ID, IRONHUB_SEARCH_CAPABILITY_ID,
};
pub use model::{IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions};
pub use render::render_reborn_ironhub_response;
pub use service::execute_reborn_ironhub_command;
