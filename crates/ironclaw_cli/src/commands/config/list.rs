use super::read::build_config_list_dto;
use crate::context::IronClawCliContext;
use crate::render::{self, OutputMode};
use clap::Args;

#[derive(Debug, Args)]
pub(super) struct ConfigListCommand {
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl ConfigListCommand {
    pub(super) fn execute(self, context: IronClawCliContext) -> anyhow::Result<()> {
        let dto = build_config_list_dto(&context)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}
