//! Shared "not-yet-wired" command output for stubbed subcommands.
//!
//! The Reborn CLI ships several subcommands today whose substrate hasn't
//! landed yet (`config apply / diff / watch`, `harness install / list /
//! activate / deactivate`). Per epic
//! [#3036](https://github.com/nearai/ironclaw/issues/3036), these are
//! locked into the command tree from day one so that operator tooling and
//! shell completion see the final command surface immediately. Each stub
//! invocation routes through the helper below so the format stays
//! identical across commands — easy to grep, easy to script around.

use ironclaw_reborn_composition::RebornAdminError;

/// Emit a uniform "not yet wired" report. Returns a non-zero anyhow
/// error so the process exits non-zero and shell scripts can detect the
/// state.
pub(crate) fn fail_not_yet_wired(error: RebornAdminError) -> anyhow::Result<()> {
    match &error {
        RebornAdminError::NotYetWired {
            operation,
            tracking_issue,
            requires,
        } => {
            eprintln!("ironclaw-reborn: command `{operation}` is not yet wired");
            eprintln!("  tracking issue : {tracking_issue}");
            eprintln!("  requires       : {requires}");
            eprintln!();
            eprintln!(
                "The command shape (arguments, output format) is locked from epic #3036; \
                 the substrate that makes it execute lands incrementally via the \
                 issue's sub-issues. Re-run after the relevant sub-issue ships."
            );
        }
        other => {
            eprintln!("ironclaw-reborn: command failed: {other}");
        }
    }
    Err(anyhow::anyhow!(error))
}
