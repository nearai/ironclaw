//! Privacy-preserving trace contribution envelopes.
//!
//! This module is intentionally separate from replay traces. Replay fixtures
//! capture enough behavior to drive tests; contribution envelopes capture the
//! consent, privacy, replayability, scoring, and revocation metadata needed
//! before a trace can leave a user's machine.
//!
//! # Module charter
//!
//! The pipeline runs left to right: a turn is **captured**, **redacted**,
//! **classified**, **scored**, **queued**, and finally **submitted**. Each
//! stage owns one **module**, and the table below is the rule for where new
//! code goes. A module is usually one file; `remote` is a directory module
//! (`remote/{claim,profile,account,client}.rs`) because its four concerns
//! would otherwise exceed the file-size budget in `.claude/rules/architecture.md`
//! §5. Every item is re-exported from this module, so `contribution::X` stays
//! the single public path for callers outside the crate — the submodules are
//! private and are never named from outside.
//!
//! | Module | Owns | Never contains |
//! |---|---|---|
//! | `envelope` | The wire schema: [`TraceContributionEnvelope`] and every metadata, event, and card type serialized with it | Behavior — construction, I/O, or policy decisions |
//! | `policy` | [`StandingTraceContributionPolicy`] and the preflight accept/reject decision | Transport, or where the policy is stored |
//! | `credit` | Value scoring and the credit estimate/summary/report views | Remote credit reconciliation (that is `submission`) |
//! | `capture` | [`RawTraceContribution`] and turning conversations or recorded LLM traces into one | Redaction rules |
//! | `privacy` | The deterministic redactor, the external privacy-filter adapter and its canary, rescrub | Per-tool field rules (those are `tool_payloads`) |
//! | `tool_payloads` | Per-tool sensitive-field redaction profiles, rule tables, and value sanitizers | Anything not keyed off a tool name |
//! | `classification` | Trace cards, allowed uses, retention policy, dataset eligibility, side-effect labelling | Redaction |
//! | `canonical` | Canonical text representations used for embedding and dedupe hashing | Anything that reads or writes the queue |
//! | `queue` | The on-disk queue: record, hold, telemetry and diagnostics types, the per-scope directory layout, policy and credential resolution | Network calls |
//! | `remote` | The pinned HTTP sink, upload-claim minting and its issuer SSRF guards, community-profile and account APIs | Queue state transitions |
//! | `submission` | Orchestration: submit, flush, status sync, revoke, and scoped credit views | New transport or new schema |
//! | `notice` | The trace-credit notice state machine and its delivery outbox | Credit *computation* (that is `credit`) |
//! | `maintenance` | Compaction, warnings, telemetry accounting, hold sidecars, path helpers, quarantine | Submission decisions |
//!
//! Two rules keep the charter honest:
//!
//! - **Redaction is split by key, not by stage.** `privacy` redacts by
//!   *pattern* (emails, paths, secret-shaped strings) over any text;
//!   `tool_payloads` redacts by *tool name and field name*. A new rule
//!   belongs to whichever input it keys off.
//! - **`queue` owns state, `remote` owns bytes on the wire, and `submission`
//!   is the only module that calls both.** That is what keeps a transport
//!   change from silently becoming a queue-semantics change.

mod canonical;
mod capture;
mod classification;
mod credit;
mod envelope;
mod maintenance;
mod notice;
mod policy;
mod privacy;
mod queue;
mod remote;
mod submission;
mod tool_payloads;

pub use canonical::*;
pub use capture::*;
pub use classification::*;
pub use credit::*;
pub use envelope::*;
pub use maintenance::*;
pub(crate) use notice::*;
pub use policy::*;
pub use privacy::*;
pub use queue::*;
pub use remote::*;
pub use submission::*;
pub(crate) use tool_payloads::*;

#[cfg(test)]
mod tests;
