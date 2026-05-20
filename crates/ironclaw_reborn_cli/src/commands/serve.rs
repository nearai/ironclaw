use std::net::{IpAddr, SocketAddr};

use clap::Args;

use crate::context::RebornCliContext;

const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
const DEFAULT_SERVE_PORT: u16 = 3000;

#[derive(Debug, Args)]
pub(crate) struct ServeCommand {
    /// Host interface for the future Reborn WebUI HTTP listener.
    #[arg(long, default_value = DEFAULT_SERVE_HOST)]
    host: IpAddr,

    /// Port for the future Reborn WebUI HTTP listener.
    #[arg(long, default_value_t = DEFAULT_SERVE_PORT)]
    port: u16,
}

impl ServeCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        if self.port == 0 {
            anyhow::bail!("--port must be greater than 0");
        }

        let boot_config = context.boot_config();
        println!("IronClaw Reborn WebUI service");
        println!("binary: ironclaw-reborn");
        println!("version: {}", env!("CARGO_PKG_VERSION"));
        println!("reborn_home: {}", boot_config.home().path().display());
        println!("home_source: {}", boot_config.home().source_label());
        println!("profile: {}", boot_config.profile());
        let listen_addr = SocketAddr::new(self.host, self.port);
        println!("listen_url: http://{listen_addr}");
        println!("v1_state: not-used");

        anyhow::bail!("Reborn WebUI server composition is not linked yet");
    }
}
