# LFD Brief: user-voice-model — User-voice model

**State**: early — `crates/ironclaw_llm/src/transcription/` already holds
an STT provider seam (OpenAI-shaped + chat-completions transcription), and
`ironclaw_silk_decoder` (WeChat SILK→WAV) exists; no channel voice-note
ingestion, no provider-neutral abstraction, no TTS. Build on the existing
transcription module — do not add a new crate. **Bar**: 0.85 holdout.
**Profile**: `voice`. See also blue-lane 10 addendum (LANE-ADDENDA.md):
its text-equivalence contract group (voice routes identically to
equivalent text) is adopted into this eval.

## Outcome

Voice as a first-class input (and optional output): inbound audio
attachments (Telegram/Slack voice-note shapes, WAV/OGG fixtures) are
transcribed through a provider-abstracted STT port (multi-provider pattern
like `ironclaw_embeddings`), the transcript enters the turn as user text
(attributed as voice-sourced), provider failures fail soft, and an optional
TTS reply path renders replies to audio behind a per-channel flag.

## Spec sources

- `crates/ironclaw_embeddings/` + its AGENTS.md (THE pattern to copy for a
  multi-provider port: trait + provider impls + config + caching)
- `crates/ironclaw_attachments/`, `crates/ironclaw_extractors/` (where
  media→text extraction belongs), `crates/ironclaw_silk_decoder/`
- Channel payload shapes: `crates/ironclaw_telegram_v2_adapter/`,
  `crates/ironclaw_slack_v2_adapter/`
- Config conventions: `src/config/embeddings.rs` shape as reference

## Stage 0 inner suite

New `ironclaw_voice` (or extractor-integrated) crate tests + attachments
tests. MOCK provider (deterministic: fixture-path → canned transcript
table lives in the PINNED profile support, keyed by audio content hash)
carries the dev suite; a real provider impl (e.g. OpenAI-compatible STT)
ships behind config for live mode.

## Eval fixtures (generator instruction)

Generate WAV fixtures at LFD-creation time with macOS `say -o` (AIFF→WAV
via afconvert), transcripts = the exact input sentences. Dev: 15 utterances,
one voice. Holdout: different sentences, different voice, plus one
noise-mixed and one truncated-file case. Fixtures live in
`lfd/user-voice-model/eval/fixtures/` (dev) and the holdout dir. Reference
transcripts are ANSWERS (sealed); the audio files are inputs.

## Eval themes (dev ~30 / holdout ~10)

1. Transcription accuracy (10): audio attachment → turn text; mock-provider
   cases use exact contracts; real-audio cases use `transcript_wer`
   (max 0.15) against sealed references via state query `transcript`.
2. Attachment plumbing (6): voice-note payload (telegram/slack shapes) →
   attachment ingested, transcription attributed as voice-sourced input
   (state_pred on turn source metadata); original audio retained (retention
   invariant — forbidden: attachment deletion).
3. Provider abstraction (5): provider selected by config (state_pred);
   unknown provider → typed config error; NO provider hardcoding (holdout
   flips the configured provider name to a second mock).
4. Fail-soft (5): provider timeout/error → turn proceeds with typed
   "transcription unavailable" event (required), run never crashes
   (forbidden: run-level error), user informed (reply contract).
5. TTS out (4): flag on → reply rendered to audio artifact (state query on
   outbound attachment record); flag off → no audio egress (forbidden).

## Feature-specific cheats → fences

- **Hardcode fixture transcripts** → transcripts are sealed answers;
  answer-literal lint catches them in diff; holdout audio is unseen and
  differently-voiced; mock-provider table is pinned (content-hash keyed, so
  new audio can't hit it).
- **Read transcript from fixture filename/metadata** → generator strips
  metadata and names files by opaque hash; probe remaps filenames.
- **Bypass STT port** (shell out to `say --reverse`-style tricks or cloud
  calls in dev) → dev runs offline: forbidden egress to any non-stub host;
  provider port required in call path (event contract).
- **WER gaming by echoing common words** → WER is symmetric
  (insertions+deletions+substitutions priced); max 0.15 needs real
  transcription; live acceptance re-checks on real provider.
- **TTS theater** (empty audio artifact) → state_pred on artifact size >
  threshold + format header.

## caps.json extras

Sealed transcript literals in diff: max 0 (answer-literal lint covers; also
add explicit patterns for the 5 longest reference sentences). New binary
fixtures in `crates/**` or `src/**`: max 0 (fixtures live in lfd/ only).

## Live mode

4 live cases: real STT provider on holdout-style audio (spend-capped);
`transcript_wer` max 0.20 (looser for provider variance) + fail-soft
timeout case against a black-holed endpoint.
