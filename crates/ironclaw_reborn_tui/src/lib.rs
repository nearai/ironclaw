//! Reborn TUI — a thin ratatui client of `ironclaw-reborn serve`'s WebChat v2
//! HTTP + SSE API (`/api/webchat/v2/*`). See
//! `docs/plans/2026-07-15-reborn-tui-service-install-design.md` for the
//! architecture. This crate must never depend on `ironclaw_webui_v2` (the
//! route/handler crate); wire types come from
//! `ironclaw_product_workflow::webchat_schema` — see
//! `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.

#![forbid(unsafe_code)]

pub mod app;
pub mod client;
pub mod spawn;

pub use spawn::ProcessInvocation;

/// Startup configuration for [`run_tui`].
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub base_url: String,
    pub token: String,
    pub spawn: Option<ProcessInvocation>,
}

/// Entry point the CLI's `tui` subcommand calls. Not yet implemented — the
/// terminal event loop, app state, and render layer land in a later task
/// (`app/`, `ui/`, `spawn/`). This stub exists so the crate compiles and is
/// wireable now; every field of `TuiConfig` is used here only to avoid an
/// unused-field warning until the real loop lands.
pub async fn run_tui(cfg: TuiConfig) -> anyhow::Result<()> {
    let _ = (&cfg.base_url, &cfg.token, &cfg.spawn);
    anyhow::bail!("ironclaw_reborn_tui::run_tui is not implemented yet (crate skeleton only)")
}
