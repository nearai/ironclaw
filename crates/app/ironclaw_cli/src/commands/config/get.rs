use super::read::build_config_get_dto;
use crate::context::RebornCliContext;
use crate::render::{self, OutputMode};
use clap::Args;

#[derive(Debug, Args)]
pub(super) struct ConfigGetCommand {
    /// Dot-separated config key (e.g. boot.profile, llm.default.model).
    key: String,
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl ConfigGetCommand {
    pub(super) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_config_get_dto(&context, &self.key)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}
