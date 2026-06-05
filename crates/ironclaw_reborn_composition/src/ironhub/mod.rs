mod capabilities;
mod catalog;
mod errors;
mod model;
mod package;
mod render;
mod service;

#[cfg(test)]
mod tests;

pub(crate) use capabilities::{extend_builtin_first_party_package, insert_handlers};
#[cfg(test)]
pub(crate) use model::{
    IRONHUB_INFO_CAPABILITY_ID, IRONHUB_INSTALL_CAPABILITY_ID, IRONHUB_SEARCH_CAPABILITY_ID,
};
pub use model::{IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions};
pub use render::render_reborn_ironhub_response;
pub use service::execute_reborn_ironhub_command;
