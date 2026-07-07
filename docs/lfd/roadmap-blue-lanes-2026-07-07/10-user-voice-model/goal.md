# Goal: make voice input a first-class turn source

Source page: https://app.notion.com/p/36e29a6526bf80049af1f1de049292f3

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for voice attachment ingestion and transcription parity. Real-time low-latency capture is out of scope unless a human explicitly expands the lane.

The spec must define:

- Supported audio input sources and formats.
- Transcription provider abstraction and fake provider behavior.
- How transcript, confidence, language, speaker/channel/thread metadata, and original attachment are represented.
- Routing through the same intent, tool, approval, and workflow pipeline as text.
- Privacy, TEE, storage, redaction, unsupported format, noisy/empty audio, and failure behavior.

## Target (outer loop)

Optimize voice parity:

- 35% supported audio is accepted and transcribed correctly enough for intent.
- 30% transcribed intent routes identically to equivalent text.
- 15% speaker, channel, thread, language, and attachment metadata are preserved.
- 10% approval and tool workflows work from voice-origin turns.
- 10% unsupported, noisy, empty, or failed transcription cases fail safely.

Bar: at least 0.90 holdout, zero raw audio or transcript leakage outside authorized stores.

## Eval design

Create 80 dev and 180 holdout fixtures. Use a mix of small audio files where available and synthetic transcript-provider fixtures. Include accents, noise labels, unsupported formats, empty audio, long audio truncation, Slack/channel-origin voice, WebUI attachment voice, and text-equivalent expected behavior.

Answers include transcript or normalized intent, metadata envelope, route, tool/approval side effects, storage/redaction expectations, and failure state.

## Harness design

Use transcription fakes for deterministic CI and optional live transcription canaries only with disposable input. The scorer must compare voice-origin turns to equivalent text-origin turns through the normal product workflow.

`harness/score.sh` must:

- Submit audio attachments or fake transcription events through channel/WebUI ingress.
- Assert transcript metadata and attachment handling.
- Run equivalent text tasks and compare route/side effects.
- Assert raw audio and transcript storage obey policy.
- Scan logs/events for unauthorized transcript/audio leakage.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $20 transcription/LLM spend; no real user audio.
- Surface allowlist: `crates/ironclaw_llm/src/transcription`, attachment context, channel adapters, WebUI upload/ingress, product workflow turn submission, storage/redaction, tests, and docs.
- Capacity caps: shared caps; fixture transcript examples capped at 30.
- Voice must not become a separate command path. It is an input modality for the same turn pipeline.

## Cycle protocol

Follow the common cycle protocol. Each cycle must compare at least one voice-origin task to its text-equivalent task and include one failure or privacy case.

## Entropy rules

- Rotate source surfaces every 3 cycles: WebUI attachment, Slack/channel attachment, unsupported format, and transcription failure.
- If transcript accuracy improves but routing parity does not, target turn envelope integration before model or prompt tuning.
- If leakage scans fail, stop feature work and fix storage/redaction boundaries.

## Cheat audit

Lane-specific cheap wins to block:

1. Key behavior by fixture filename; probe randomizes filenames and metadata.
2. Bypass audio ingestion and inject transcript directly; scorer submits through ingress.
3. Treat voice as a separate command path; text-equivalence comparison fails.
4. Drop speaker/thread metadata; metadata score fails.
5. Store raw audio in unauthorized logs; leakage scan fails.
6. Ignore unsupported formats; negative cases fail.
7. Skip approvals for voice-origin turns; approval cases are scored.
8. Use live transcription as sole evidence; deterministic fake-provider holdout is authoritative.
9. Overfit exact transcript text; scorer normalizes intent and probes paraphrases.
10. Hide failed transcription as empty user intent; failure-state answers catch it.

## Stop conditions

Stop when holdout is at least 0.90 with zero leakage and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or raw audio/transcript can escape authorized storage.

