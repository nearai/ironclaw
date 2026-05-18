use clap::Args;
use ironclaw_reborn_config::RebornDoctorReport;

use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct DoctorCommand;

impl DoctorCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let report = RebornDoctorReport::from_config(context.boot_config().clone());

        println!("IronClaw Reborn doctor");
        println!("reborn_home: {}", report.home_path().display());
        println!("home_source: {}", report.home_source_label());
        println!("profile: {}", report.profile());
        println!("v1_state: {}", report.v1_state());
        println!("driver_registry: initialized");
        Ok(())
    }
}
