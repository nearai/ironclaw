//! Whole-journey Telegram scenario through the PRODUCTION composition
//! (`build_reborn_runtime` + `build_telegram_host_runtime_mounts` +
//! `build_webui_services_with_telegram_host_mounts`), asserting at every
//! seam the contract names (`docs/reborn/contracts/telegram-v2.md`):
//!
//! 1. **Admin setup** — the operator PUTs the bot token to the real
//!    protected route; the save pipeline's `getMe` + `setWebhook` are
//!    captured at the network boundary (the URL carries the SUBSTITUTED
//!    `/bot<token>/` segment — the placeholder never leaks), the registered
//!    webhook URL derives from the public base, and `GET` returns the
//!    redacted status.
//! 2. **In-chat activation parks** — a WebChat turn calls
//!    `builtin.extension_install` + `builtin.extension_activate` for
//!    `telegram`; the unpaired caller parks the run as
//!    `TurnStatus::BlockedAuth` (the pairing gate).
//! 3. **Pairing consume resumes** — the pairing route mints a code; the
//!    webhook (verified `X-Telegram-Bot-Api-Secret-Token`, read from the
//!    captured `setWebhook` body exactly where Telegram would hold it)
//!    delivers `/start <CODE>`; consume binds the account (pairing status
//!    facade flips to connected over the durable binding), records the DM
//!    target (the production outbound-target provider lists the
//!    `telegram:dm:…` entry), replies with the paired confirmation, and
//!    dispatches the auth continuation — the parked run RESUMES to
//!    `Completed` and the post-resume model reply lands on the WebChat
//!    timeline.
//! 4. **DM turn renders through the revision workflow** — a subsequent DM
//!    webhook produces a real turn whose final reply is rendered by the
//!    per-revision adapter and egresses as `sendMessage` to the DM chat,
//!    captured at the network boundary with the substituted bot path.
//!
//! Model scripting preserves the single-fake-at-the-vendor-SDK-seam
//! invariant: a scripted `TraceLlm` sits under the REAL
//! `provider_chain_over` + `LlmProviderModelGateway`, routed uniformly to
//! every scope by a `resolve_for_scope` adapter (`scope_gateway.rs`'s
//! pattern for runtimes whose thread scopes are minted at bind time).
//!
//! Manual-QA catalog rows this bin covers (coverage map:
//! `docs/qa/telegram-coverage-map.md`): qa-telegram admin-setup happy path,
//! unpaired-activation pairing gate, `/start <CODE>` consume + blocked-run
//! resume, paired-DM turn + outbound render, webhook secret verification on
//! the live route, and the in-DM extension-install gate feedback regression
//! (see `telegram_dm_slack_install_gates_with_action_needed_notice_not_silence`).
//!
//! One scenario file per user journey (catalog row ids in each
//! scenario's doc-comment); the shared stack lives in `harness.rs`.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod harness;

mod scenario_admin_setup_pair_resume_reply;
mod scenario_decline_in_chat;
mod scenario_delivery_honesty;
mod scenario_gated_install_deny_arm;
mod scenario_gated_install_oauth_link;
mod scenario_multiuser_isolation;
mod scenario_unpair_repair_fresh_slate;
