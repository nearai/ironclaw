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

### Live paired evaluation (2026-08-14)

A developer-supplied `ANTHROPIC_API_KEY` was loaded from the macOS keychain and
passed only through the explicitly configured developer environment variable.
The evaluation used commit `9a03c1c174`, Claude Code 2.1.232, and
`claude-agent-acp` 0.67.0, with an isolated shared clone for each task and lane.
Both lanes received the same prompts and seeded regressions. No tests were run,
no remote pull requests were created, and every coding change stayed inside its
task clone.

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
assistant reply.

| Task | ACP harness | Rust loop | Observed outcome |
| --- | ---: | ---: | --- |
| Turn-path Q&A | 145 s, 2/2 | 399 s, 1/2 | Harness identified `ProfileRoutingTurnRunExecutor`; Rust cited retired top-level crate paths and missed the new executor-routing seam. |
| Extension identity Q&A | 167 s, 2/2 | 86 s, 2/2 | Both answered correctly; Rust was faster. |
| Small streaming fix + PR | 315 s, 2/2 | 902 s, 0/2 | Harness restored 16 ms behavior, added a timer-only regression test, committed it, and returned a PR handoff. Rust timed out without a reply or edit beyond the seed. |
| UTF-8 debugging, two turns | 108 s, 2/2 | 137 s, 0/2 | Harness restored boundary-safe truncation and added a focused edge case. Rust described and claimed a separate `acp_chunker` crate and 12 tests that did not exist; the seeded panic remained. |
| Profile routing, two turns | 281 s, 2/2 | 908 s, 0/2 | Harness restored claimed-profile dispatch and added caller-path coverage. Rust timed out on the diagnostic turn with the seed untouched. |
| Four-turn continuity | 512 s, 2/2 | stopped at 687 s, 0/2 | Harness retained the session, created the note, and corrected it on the fourth turn. The Rust run produced no first reply and was stopped once the earlier failures made further waiting uninformative. Four transient Anthropic HTTP errors make this last Rust observation network-contaminated. |
| **Total** | **6/6 tasks, 12/12; 1,528 s** | **1/6 strict task successes, 3/12; >3,118 s observed** | Harness median task time was 224 s. Two Rust tasks exhausted the 900-second reply timeout before the intentionally stopped continuity run. |

The harness-side Claude transcripts reported 220 uncached input tokens, 99,322
output tokens, 1,262,214 cache-creation input tokens, and 14,209,681 cache-read
input tokens across 125 provider messages. The Ironclaw ACP executor currently
records `model_usage: None`, and the live run-artifact API did not expose native
Rust-loop usage, so a symmetric token or USD comparison is not available. This
is an accounting gap, not evidence that either lane used no tokens.

This was an operational comparison, not a controlled model-quality A/B. The
Rust lane was configured for `claude-sonnet-4-6`; the ACP transcripts show that
Claude Code selected `claude-opus-5`. Each task was run once, the seeded changes
were uncommitted working-tree mutations, and the Rust continuity task was
stopped at the operator's direction after the preceding timeouts. These
limitations prevent attributing the quality delta solely to loop design.

### Stability and placement observations

- The maintained ACP adapter completed all 11 harness turns. Session resume
  worked across the four-turn continuity task, and no adapter or driver failure
  occurred.
- Host placement was used for the live paired tasks because it exposes the
  developer's installed `git`, Rust toolchain, and repository. The pinned
  Docker image built successfully but does not contain `git` or `cargo`; using
  it would have made the coding cases an image-tooling test rather than a loop
  comparison.
- Cumulative `ModelTextDelta` streaming and the 16 ms projection window were
  exercised indirectly by the streaming-fix task. The benchmark measured
  end-to-end reply latency, not time-to-first-token or per-chunk cadence.
- The native Rust loop's two clean timeouts had no provider error in their
  server logs. The stopped continuity run did show transient provider HTTP
  retries, so it must not be grouped with those clean timeouts.

### Recommendation

Keep harness routing experimental, explicit, and default-off. The v0 question
has a positive answer for host-based developer work: the off-the-shelf ACP
harness completed this task mix, including edits and session continuity, while
the current Rust loop did not.

Unlock **#7622 only**, scoped first to the breadth/image trigger demonstrated
here: a pinned coding-capable image, durable workspace/session behavior, and
usage accounting that permits a symmetric rerun. Do not unlock #7621: this eval
used only a developer credential and demonstrated no customer-credential need.
Do not unlock #7623: it exercised neither extension-tool access nor a controlled
same-model production-promotion benchmark. Before either later rung, repeat the
set with the same pinned model in both lanes, record time-to-first-text and
chunk cadence, and collect provider usage through the common run artifact.
