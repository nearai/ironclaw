# Spec: user-voice-model — voice-origin turns route like equivalent text

Sources: `lfd/_briefs/COMMON.md`, `lfd/_briefs/user-voice-model.md`,
`lfd/_shared/SCHEMA.md`, the Lane 10 addendum, and the roadmap lane goal.
Wave-1 first slice ships 10 dev cases and 3 off-repo holdout cases; the
contracts are shaped so the same profile can expand to the full 30+ case
set without changing the harness schema.

## 1. Product behavior

Voice is an input modality for the existing turn pipeline, not a separate
command path. Inbound audio attachments from WebUI, Telegram-style voice
notes, and Slack-style file events are landed as normal attachments,
transcribed through a provider-neutral STT port, wrapped in a voice-origin
turn envelope, and then submitted through the same intent, tool, approval,
workflow, memory, and reply path as equivalent typed text.

The implementation builds on `crates/ironclaw_llm/src/transcription/`; do
not add a new transcription crate. Audio landing belongs in
`crates/ironclaw_attachments`; media-to-text classification belongs with
`crates/ironclaw_extractors` or the existing attachment bridge; channel
payload normalization belongs in the Reborn channel adapters.

## 2. Supported inputs and metadata

Stage 0 supports WAV fixtures and channel payload shapes that identify
Telegram voice notes, Slack audio files, and WebUI uploads. The provider
abstraction may accept other formats already covered by `AudioFormat`, but
unsupported formats, empty audio, malformed audio, and provider failures
must fail soft.

Every voice-origin turn records:

- original attachment id, MIME, size, content hash, retained storage key,
  and source channel/thread/user metadata;
- transcript text, confidence when available, provider id, language, and
  speaker analysis;
- an explicit `source = "voice"` turn marker carried into state and audit
  records;
- privacy classification describing where raw audio and transcript text are
  authorized to persist.

## 3. Provider abstraction

Mirror the multi-provider pattern used by embeddings: a trait, provider
implementations, typed config, and deterministic tests. Dev and holdout
scripted runs use a mock provider keyed by audio content hash. The mock
transcript table belongs in pinned runner support, not product code or eval
inputs. Live provider cases, when added later, run only through the spend
wrapper with disposable keys.

Provider selection is config-driven. Unknown provider names produce typed
config errors. Dev cases forbid external STT egress; `harness/caps.json`
and contracts reject direct calls to OpenAI/OpenRouter transcription hosts
in deterministic mode.

## 4. Text-equivalence routing

For every successful voice case, the profile must run the voice-origin turn
and an equivalent text-origin control through the same production boundary.
The `route_comparison` state query returns at least:

```json
{"equivalent_to_text": true, "route_family": "reminder.create", "side_effects_equal": true}
```

The control text comes from the same transcript emitted by the configured
provider. It is not read from visible case JSON. This prevents a bypass
where voice cases inject text directly and never exercise audio ingestion.

## 5. Profile-owned state queries

The `user_voice_model` profile must implement these `state_queries` against
persisted state and recorded side effects after the scenario runs:

- `transcript` -> `{"text", "provider", "language", "confidence"}`.
- `turn_envelope` -> `{"source":"voice", "channel", "thread_id", "user_id", "language", "speaker_count"}`.
- `route_comparison` -> `{"equivalent_to_text": bool, "route_family", "side_effects_equal": bool}`.
- `attachment_record` -> `{"kind":"audio", "retained": bool, "storage_key_present": bool, "size_bytes": n}`.
- `provider_trace` -> `{"provider", "configured_provider", "fallback_used": bool}`.
- `approval_audit` -> `{"required": bool, "approved": bool, "tool_after_approval": bool}`.
- `tts_artifact` -> `{"created": bool, "mime_type", "size_bytes": n}`.
- `language_detection` -> `{"allowed": bool, "detected_language", "required_language"}`.
- `speaker_analysis` -> `{"speaker_count": n, "requires_clarification": bool}`.
- `failure_record` -> `{"kind", "user_informed": bool, "crashed": false}`.
- `redaction_audit` -> `{"secret_redacted": bool, "secret_in_reply": bool, "secret_in_events": bool}`.
- `privacy_audit` -> `{"raw_audio_authorized_store": bool, "unauthorized_transcript_leaks": n}`.

These reads must be backed by runner recorders and persisted state, not by
case-local echoes.

## 6. Failure and privacy rules

Wrong-language audio, multi-speaker ambiguity, malformed/truncated audio,
empty audio, provider timeouts, and unsupported formats must never become
empty user intent. They emit typed events, preserve the attachment record,
inform the user, and avoid tool/workflow side effects unless a human later
clarifies. Secret-bearing transcripts must be redacted from replies, logs,
events, egress, and model-visible public surfaces; the leak matcher and
state queries both price failures.

## 7. Stage-0 tests

Before optimizing the eval, make these pass and keep them green every cycle:

1. `cargo test -p ironclaw_llm transcription` for provider abstraction,
   format handling, empty audio, and provider error mapping.
2. `cargo test -p ironclaw_attachments` for audio attachment landing,
   retention, and storage-key invariants.
3. `cargo test -p ironclaw_extractors` for unsupported/malformed media
   classification where extractor code participates.
4. `cargo test --test lfd_profiles user_voice_model` or the current
   integration target that executes `tests/integration/lfd/profiles/user_voice_model.rs`.
5. `cargo fmt` and `cargo clippy --all --benches --tests --examples --all-features` with zero warnings.

The profile must execute every dev case with `status: "ran"`; the skeleton
currently returns `unsupported`, which correctly scores 0 until wired.
