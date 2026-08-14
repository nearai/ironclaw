# ACP harness v0 findings

Issue #7624 adds a deliberately narrow evaluation lane for running Claude Code
through the official ACP client protocol. The lane is default-off and selected
by explicit `[harness]` configuration plus a list of run-profile ids. Only
standalone development profiles may route turns; hosted profiles reject
routing and also fail closed if configured with host placement.

Host and Docker placement use the same pins from
`Dockerfile.claude-code-acp`; `scripts/install-claude-code-acp.sh` installs
those pins on a developer machine. The container uses the maintained
`@agentclientprotocol/claude-agent-acp`
adapter. The deprecated `@zed-industries/claude-code-acp` 0.16.2 advertised
session loading but failed to resume a session after its adapter process exited;
the maintained package replaces that implementation.

## Implementation observations

- Run-profile selection is owned by a neutral executor router whose default is
  the canonical Rust executor. The ACP executor implements `TurnRunExecutor`
  directly and has no fallback or knowledge of other loop implementations, so
  another executor can be registered without changing the harness.
- ACP gives Ironclaw a small, typed integration surface: initialize, session
  new/load, prompt, updates, and permission requests. Keeping the protocol at
  this boundary avoided translating Claude Code internals into Ironclaw tools.
- ACP agent-message chunks are accumulated, bounded, and sanitized before they
  enter the existing cumulative `ModelTextDelta` milestone path. The WebUI can
  therefore show the reply while it is generated without a harness-specific
  transport or one durable transcript write per chunk; the completed reply is
  still finalized once through the canonical transcript boundary.
- A stable workspace derived from the typed thread id is enough to retain both
  repository changes and the ACP session id across turns without putting ACP
  state into tenant storage.
- The v0 permission policy intentionally chooses the adapter's first offered
  allow option. This is suitable only for explicitly enabled developer harness
  runs.
- Container termination, bounded update collection, and terminal scheduler
  failures are mandatory. Without all three, a hung adapter can consume a
  worker indefinitely or cause a failed turn to be re-driven.
- The ACP executor receives only an opaque process transport and kill handle.
  Host placement starts a process with an empty ambient environment and an
  explicit developer credential list; Docker placement delegates lifecycle and
  mounts to the existing sandbox lane. Customer secret stores are not reachable
  from either configuration path.

## Evaluation status

The deterministic fake-adapter host tests cover protocol compatibility,
cumulative live text updates, multi-turn session reuse, permission handling,
process death, timeout cleanup, lease release, and reply persistence without
paid API use. The same fake runs through Docker in the repository's gated
Docker lane to pin placement parity.

### Live paired evaluation (corrected 2026-08-14 run)

A developer-supplied `ANTHROPIC_API_KEY` was loaded from the macOS keychain and
passed only through the explicitly configured developer environment variable.
The corrected evaluation used commit `679ca696a3`, Claude Code 2.1.232,
`claude-agent-acp` 0.67.0, and `claude-sonnet-4-6` in both lanes. The Claude Code
model was pinned with `ANTHROPIC_MODEL`; every non-synthetic model-bearing ACP
transcript record reported `claude-sonnet-4-6`.

Each lane received the same task text, a task-local clone, and the same committed
seed regression. Every turn also named its allowed workspace root and required
`pwd` plus `git rev-parse --show-toplevel` before work. Preflight in both lanes
observed commit `679ca696a3`, origin `https://github.com/nearai/ironclaw.git`,
the first `Cargo.toml` line `[workspace]`, and a probe file written into the
evaluated clone. The ACP transcript auditor found zero tool inputs that escaped
the task checkout. No tests were run, no remote pull requests were created, and
all coding changes remained in ignored task clones.

The task set was:

1. Trace WebUI submission through profile-level executor routing.
2. Explain `credential_name` versus `extension_name` and its setup-UI effect.
3. Repair a seeded 1.5-second live-text coalescing regression, add focused
   coverage, make a local commit, and draft the pull request handoff.
4. Diagnose and repair UTF-8 truncation across two turns.
5. Diagnose and repair an incorrect profile-routing lookup across two turns.
6. Retain an architecture map across four turns, create a continuity note, and
   review that note against the original facts.

Scoring is deliberately small and auditable: 2 means the requested outcome was
correct and complete, 1 means materially useful but missing or wrong on a key
requirement, and 0 means no usable outcome. A timeout was 900 seconds per
assistant reply. Because the operator requested CI-only validation, scoring
uses response review and static diff inspection; generated tests were not
compiled locally.

| Task | ACP harness | Rust loop | Semantic observation |
| --- | ---: | ---: | --- |
| Turn-path Q&A | 242 s, 2/2 | 904 s, 0/2 | Harness traced the current handler, coordinator, scheduler, and `ProfileRoutingTurnRunExecutor` path. Rust exhausted the reply deadline without a final message. Its log contained four transient Anthropic HTTP retries, so this timeout is network-contaminated. |
| Extension identity Q&A | 90 s, 2/2 | 475 s, 1/2 | Harness cited the `CredentialName`/`ExtensionName` newtypes and the shared setup-routing rule. Rust explained the storage-vs-UI distinction but incorrectly treated `extension_name` as the manifest display string and did not identify the canonical identity contract. |
| Small streaming fix + PR | 263 s, 2/2 | 229 s, 2/2 | Both restored the 16 ms window, added caller-facing timer coverage, created a local commit, and returned a usable PR handoff. Rust was faster. |
| UTF-8 debugging, two turns | 135 s, 2/2 | 189 s, 2/2 | Both diagnosed the byte-boundary panic and restored boundary-safe truncation in the evaluated checkout. Harness reused the existing exact regression; Rust expanded it with two edge cases. |
| Profile routing, two turns | 704 s, 1/2 | 1,090 s, 1/2 | Both restored dispatch through `claimed.resolved_run_profile.profile_id`. Harness completed both replies, but its `NoopTransitionPort` implementation omitted the required `#[async_trait]`, so the generated coverage is statically invalid. Rust edited during the diagnose-only turn and then timed out before its second reply. This Rust timeout had no provider error. |
| Four-turn continuity | 775 s, 2/2 | 1,343 s, 1/2 | Harness completed all four turns, wrote the note, and reviewed it against source. Rust retained the map through two turns but timed out on turn three before writing the note; its log contained nine transient Anthropic HTTP retries. |
| **Total** | **5/6 strict; 11/12 points; 11/11 turns; 2,208 s** | **2/6 strict; 7/12 points; 7/11 turns; 4,230 s** | Harness median task time was 252 s; Rust median was 689 s. The corrected result still favors the harness, but it is not the original 12/12-vs-3/12 headline. |

The harness-side Claude transcripts reported 167 uncached input tokens, 102,381
output tokens, 954,588 cache-creation input tokens, and 12,419,380 cache-read
input tokens across 144 provider messages. The Ironclaw ACP executor currently
records `model_usage: None`, and the live run-artifact API did not expose native
Rust-loop usage, so a symmetric token or USD comparison remains unavailable.
This is an accounting gap, not evidence that either lane used no tokens.

This is a same-model, same-task operational comparison, but not a repeated
statistical benchmark. Each task ran once. Claude Code's built-in `Agent` tool
was disabled after an ACP run showed a completed Claude subagent whose result
never returned to the parent session; both measured lanes therefore ran as
single-agent loops. The 13 Rust provider retries contaminate its turn-path and
continuity timings, while its clean profile-routing timeout still provides a
non-network example of loop inefficiency.

### Evaluation-isolation findings

The first paired run is superseded for loop-quality conclusions. It had two
material confounders: Claude Code selected `claude-opus-5` while Rust used
Sonnet 4.6, and the Rust file tools wrote to the durable virtual `/workspace`
instead of the checked-out repository. The latter explains the apparent
`acp_chunker` fix that never existed in the clone.

A second attempted run exposed a different leak: although Claude Code's ACP
session `cwd` was the task clone, read-only QA commands selected the source
worktree by absolute path. Rewriting the clone's origin to GitHub alone did not
prevent it. The corrected run therefore supplies the task workspace explicitly
on every turn and rejects ACP results whose tool inputs reference the source
worktree outside the task clone. The measured run produced zero such leaks.

Host placement was retained because the coding tasks require the developer's
installed `git` and Rust toolchain. The benchmark measured end-to-end reply
latency, not time-to-first-text or per-chunk cadence. Cumulative
`ModelTextDelta` streaming and the 16 ms projection window were exercised by
the streaming-fix task but were not separately timed.

### Recommendation

Keep harness routing experimental, explicit, and default-off. The corrected
same-model run still supports the core hypothesis: the ACP harness completed
more of this repository-work task mix, preserved long-session continuity, and
used substantially less wall time than the current Rust loop. It also found a
real harness-side weakness in generated regression coverage, so promotion
should remain evidence-gated rather than automatic.

Unlock **#7622 only**, scoped first to the breadth/image trigger demonstrated
here: a pinned coding-capable image, durable workspace/session behavior,
workspace-escape detection, and usage accounting that permits symmetric cost
measurement. Do not unlock #7621: this eval used only a developer credential
and demonstrated no customer-credential need. Do not unlock #7623: it exercised
neither extension-tool access nor repeated production-promotion evidence.
Before either later rung, repeat the corrected set, record time-to-first-text
and chunk cadence, compile generated task diffs in CI, and collect provider
usage through the common run artifact.
