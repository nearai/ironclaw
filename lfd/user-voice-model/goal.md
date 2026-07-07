# Goal: make voice-origin turns transcribe, preserve metadata, and route identically to equivalent text

This is the Lane-10 launch target merged from the roadmap goal, the lane
addendum, and `lfd/_briefs/user-voice-model.md`. The branch package is a
Wave-1 first slice: 10 dev cases plus 3 off-repo holdout cases. Expand the
same schema to the full lane size only after this slice lints, probes, and
runs through the profile.

## Stage 0 — Build to spec (inner loop)

Implement `spec.md`. Do not score against the eval until the feature-relevant
suite is green. Tests stay green every cycle thereafter.

Stage-0 command list:

1. `cargo test -p ironclaw_llm transcription`
2. `cargo test -p ironclaw_attachments`
3. `cargo test -p ironclaw_extractors`
4. the LFD integration target that executes profile `user_voice_model`
5. `cargo fmt` and `cargo clippy --all --benches --tests --examples --all-features`

The profile `tests/integration/lfd/profiles/user_voice_model.rs` must execute
every dev case with `status: "ran"`; `unsupported` scores 0 and is not a skip.

## Target (outer loop)

Metric: voice parity, both directions. Successful audio is accepted,
transcribed with word error rate below the contract threshold, wrapped as a
voice-origin turn, and routed through the same side-effect path as equivalent
text. Failure cases must fail closed with typed events and user-facing
clarification. Missing required behavior starves the numerator; spurious
forbidden behavior such as external dev STT egress, raw secret leakage, wrong
tool execution, or run crashes halves or zeros the case.

Weighting intent for the full lane: 35% transcription accuracy, 30%
equivalent-text routing, 15% metadata preservation, 10% approval/tool/TTS
workflow behavior, 10% fail-soft privacy and malformed-audio behavior.

Bar: **0.90 on holdout** with zero raw-audio or transcript leakage outside
authorized stores. Score with `harness/score.sh`. A VOID result means a
constraint was violated; find and remove the violation, but the harness will
not reveal which constraint. Holdout is aggregate-only, max 3 calls per 24 h,
and acceptance is measured on holdout exclusively.

Small-eval warning (verbatim per portfolio COMMON): Per-feature evals are
30-60 dev + 10-15 holdout cases: far below the ~200 enumerability threshold.
The compensating controls are (a) contract-style scoring (satisfying a
behavioral contract usually requires the machinery, unlike data-lookup evals),
(b) probe gap as the memorization gauge, (c) feedback capped to aggregate +
<=5 worst case ids, (d) holdout answers off-repo.

This first slice is smaller still (10 dev + 3 holdout), so treat it as a
harness-validating slice, not final statistical evidence.

## Constraints

- Wall-clock budget: **12 h**. Check `harness/status.sh` every cycle; it
  shows elapsed time, score history, holdout use, spend, and trend.
- Spend ceiling: **$20** transcription/LLM. Deterministic dev/holdout cases
  should spend $0 because they use pinned mock providers. No real user audio.
- Surface allowlist for implementation: `crates/ironclaw_llm/src/transcription/**`,
  `crates/ironclaw_attachments/**`, `crates/ironclaw_extractors/**`, the
  Reborn channel/WebUI ingress and product-workflow turn submission paths,
  tests, and `lfd/user-voice-model/LOG.md`.
- Read-only during optimization: this `goal.md`, `spec.md`, `harness/**`,
  `eval/**`, `lfd/_shared/**`, and all LFD runner support except the one
  `tests/integration/lfd/profiles/user_voice_model.rs` profile file.
- BANNED: reading `lfd/user-voice-model/harness/answers.dev.json`, reading
  `$LFD_STATE_ROOT/**`, editing the scorer/harness/eval, hardcoding dev or
  holdout case ids, adding transcript lookup tables in product code, or
  adding audio fixtures under `src/**` or `crates/**`.
- Capacity caps are enforced by `harness/caps.json`: eval case id branching
  = 0; sealed transcript literals in product diffs = 0; direct external STT
  egress in deterministic dev = 0; `#[ignore]` or `#[cfg(never)]` additions
  in voice profile and voice-adjacent crates = 0; product-tree audio fixture
  additions = 0.
- Voice must not become a separate command path. It is an input modality for
  the existing turn pipeline.

## Cycle protocol

1. Score dev with `harness/score.sh --outcomes <dir>` once the profile emits
   outcomes.
2. Run `harness/probe.sh`; if the probe gap grows, the next change removes
   an eval-shaped artifact instead of adding one.
3. Write hypothesis, expected failure mode, and diagnostic in `LOG.md` before
   changing code.
4. Make one scoped change and run the Stage-0 tests that cover it.
5. Log the result.
6. Commit every cycle: `git commit -am "cycle <n>: <score>"`.

Every cycle must include at least one voice-vs-text equivalence observation
and one failure/privacy observation.

## Entropy rules

- Stall rule: if the metric did not move last cycle, the next attempt must be
  structural; same-knob-harder is banned.
- Exploration quota: every 5 cycles, try a different surface: WebUI upload,
  Telegram/Slack attachment, provider config, approval/tool routing,
  malformed audio, privacy redaction, or TTS.
- If transcript accuracy improves but routing parity does not, work on turn
  envelope integration before prompt/model tuning.
- If leakage scans fail, stop feature work and fix storage/redaction
  boundaries first.

## Cheat audit

1. Hardcode fixture transcripts -> sealed answers, answer-literal lint,
   holdout different voice, and probe filename remapping.
2. Key by fixture filename/hash -> opaque hash filenames plus caps against
   hash/case-id branching outside pinned runner support.
3. Bypass audio ingestion and inject text -> `route_comparison` must compare
   a voice-origin run to a text-origin control created after STT.
4. Separate voice command path -> equivalent-text route and side-effect
   contracts fail.
5. Drop speaker/thread/channel metadata -> `turn_envelope` and
   `attachment_record` contracts fail.
6. Store raw audio/transcripts in public logs -> leak matcher and
   `privacy_audit`/`redaction_audit` contracts fail.
7. Ignore unsupported/malformed audio -> fail-soft cases require typed
   events, retained attachment state, and user-visible clarification.
8. Skip approvals for voice turns -> ordered approval/tool contracts fail.
9. Use live STT as sole evidence -> deterministic mock holdout is
   authoritative; live cases are optional later.
10. Empty or fake TTS output -> artifact size and MIME contracts fail.
11. Mine dev miss ids -> feedback is capped to aggregate + <=5 case ids and
   holdout remains aggregate-only.
12. Edit scorer/eval/answers -> read-only surface, canaries, pins, and VOID
   semantics apply.

## Stop conditions

Stop when holdout >= 0.90 with Stage 0 green and zero leakage, any budget is
exhausted, marginal dev gain < 0.01 for 4 consecutive cycles, a raw
audio/transcript leak is found, or the scorer is found invalid and cannot be
repaired inside budget. On stop, write a final report in `LOG.md` with best
score, what generalized, what was abandoned, and highest-leverage next steps.
