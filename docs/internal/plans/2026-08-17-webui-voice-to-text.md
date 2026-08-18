# WebUI voice-to-text via NEAR AI Whisper

Status: **implemented** (2026-08-17). The ⚠ assumptions below were resolved
against live code before building; each one's answer is recorded in
"Decisions taken" at the end of this document, and the plan body is left as
written so the two can be compared.

Confirmed: NEAR AI serves `openai/whisper-large-v3` on the OpenAI-compatible
`/v1/audio/transcriptions` endpoint, reachable with the **existing**
`NEARAI_API_KEY` and the **existing** `cloud-api.near.ai` egress host. No new
credential, no new vendor, no new allowlisted destination.

> Everything below marked ⚠ was an assumption from a prior session. It has now
> been re-verified against live code — see "Decisions taken".

## Goal

A mic button in the WebUI composer: record → transcribe → insert text into the
composer for the user to edit and send. Batch (not streaming) for v1.

Explicitly **not** in v1: live interim text, voice output (TTS), wake words,
transcription of uploaded media files, non-WebUI surfaces.

## Why server-side rather than in-browser

The browser must never hold the inference credential. Recording client-side and
transcribing host-side keeps the repo's standing invariants intact:

- credentials stay host-side, injected only at mediated egress
- external HTTP goes through `ironclaw_network`
- new ingress validates and bounds the payload *before* storage or dispatch

## Architecture

Three pieces, deliberately separable.

### 1. A transcription port (new, narrow)

Do **not** extend `LlmProvider`. That trait is chat-shaped (messages in,
completion out); transcription is a different modality with a multipart body
and no conversation. A separate port keeps a self-hosted `whisper.cpp` backend
a second implementation rather than a rewrite.

```
trait TranscriptionProvider {
    async fn transcribe(&self, audio: AudioClip, opts: TranscribeOptions)
        -> Result<Transcript, TranscriptionError>;
}
```

- `AudioClip`: bytes + declared media type + duration hint, already bounded.
- `Transcript`: text + optional language + optional segments.
- Errors classify like every other boundary error (`Transient` / `Permanent` /
  `Misconfigured` / `PolicyDenied`), so the transport maps them to stable codes
  without leaking provider bodies.

⚠ Placement to confirm: likely a domain crate (its own, or alongside
`ironclaw_llm` as a sibling module) with the NEAR AI implementation reaching
egress through `ironclaw_network`. It is *not* product's and *not* the WebUI's.

### 2. Host route

`POST /api/webchat/v2/transcribe`, multipart audio, bearer-authenticated like
every other v2 route.

- Reuses the existing WebUI auth middleware and body-limit layer.
- Returns `{ "text": "..." }` or a redacted `WebUiV2HttpError`.
- ⚠ Confirm whether this should instead go through `ProductSurface` as a
  command. Argument for a plain route: transcription produces no durable
  product state and starts no turn — it is a pure transform, closer to the
  attachment-read routes than to a turn submission. Argument against: the repo
  prefers transports consume `ProductSurface`. **Decide this first** — it sets
  the shape of everything else.

### 3. Composer UI

⚠ `crates/product/ironclaw_webui/frontend/src/pages/chat/` — mic button in the
composer.

- `MediaRecorder` start/stop, visible recording state + elapsed timer.
- On stop: upload, show a pending state, insert returned text at the cursor.
- Never auto-send. The user edits and sends, so a mis-transcription is always
  correctable.
- Errors surface inline and are retryable — do **not** repeat the
  `OnboardingPairingCard` mistake of swallowing the real error in a bare
  `catch {}` and rendering one generic string.

## Bounds and safety

Set these before anything works, not after:

| Bound | Value | Why |
|---|---|---|
| Max clip duration | 2–5 min | user-facing, prevents runaway uploads |
| Max body bytes | ≤ 25 MB | whisper endpoints commonly cap here |
| Accepted media types | explicit allowlist | reject before parsing |
| Rate limit | per caller | reuses existing middleware |

**Retention:** transcribe-and-discard. Audio is not persisted; only the
resulting text, and only once the user actually sends it (at which point it is
ordinary message content). This must be an explicit decision in the PR, since
the "LLM data is never deleted" rule governs conversation data and voice blobs
are new territory.

**Redaction:** provider errors and bodies never reach the client verbatim.

## Known snag: audio format

`MediaRecorder` emits **webm/opus** on Chrome and **mp4/aac** on Safari.
Whisper endpoints differ in what they accept. Test both browsers in the first
day of work — this looks finished until someone opens Safari. If the endpoint
rejects one, options are client-side re-encode (heavy) or picking a
`mimeType` both support at `MediaRecorder` construction (preferred).

## Configuration

Nothing new required when NEAR AI is already configured. Add an optional
override for the transcription model, defaulting to `openai/whisper-large-v3`,
so a deployment can point at a different served model. Voice is **off** when no
transcription-capable backend resolves — the button is absent, not broken.

## Phasing

1. **Port + NEAR AI impl + unit tests.** No UI. Provable in isolation.
2. **Route + bounds + auth + error mapping.** Caller-level test through the
   real router; assert bounds reject before egress.
3. **Composer UI + browser test.** Both Chrome and Safari format paths.
4. **Docs**: `.env.example` (model override only), plus a short WebUI doc note.

Each phase is independently shippable; 1–2 are useful even with no UI.

## Testing

- **Unit/contract**: port behavior, error classification, bound rejection.
- **Caller-level**: route returns text for a valid clip; oversized/wrong-type
  rejected *before* any egress attempt (assert the egress double saw nothing).
- **Browser**: mic button records, uploads, inserts text, surfaces a retryable
  error on failure.
- Hermetic throughout — the provider is a recording double; no live NEAR AI
  calls in CI.

## Open questions

1. Route vs. `ProductSurface` command (blocks the rest — decide first).
2. Which crate owns the port.
3. Does the NEAR AI endpoint accept webm/opus directly?
4. Should transcription count against the user's spend budget
   (`IRONCLAW_BUDGET_USER_DAILY_USD`)? It is billable inference, so probably
   yes — needs the budget owner's call.

---

## Decisions taken (2026-08-17, verified against live code)

### 1. Route vs. `ProductSurface` command — **command, plus a thin route**

The repo already has the precedent this question was really about:
`attachment.read` and `fs.read` are pure transforms that write no durable
state and still go through `ProductSurface` as commands, because WebUI v2
handlers hold `Arc<dyn ProductSurface>` and nothing else. Transcription is
`audio.transcribe`, invoked by a plain `POST /api/webchat/v2/transcribe`
handler that does no work of its own. Both halves of the plan's argument turn
out to be satisfied: the route is as thin as the "pure transform" reading
wanted, and the surface boundary is the one the repo prefers.

### 2. Which crate owns the port — **it already existed**

`ironclaw_llm::transcription` has carried `TranscriptionProvider`,
`AudioFormat`, and an `OpenAiWhisperProvider` (`/v1/audio/transcriptions`,
multipart, overridable base URL and model) since the WS7 crate moves. It was
unwired — exported and never constructed. So the port did not need designing,
only classifying and connecting:

- **`ironclaw_llm`** gained `TranscriptionErrorKind`
  (`Transient` / `Permanent` / `Misconfigured`) and a `ProviderStatus` variant
  carrying the HTTP status, so a transport can classify without parsing a
  message. `PolicyDenied` was folded into `Misconfigured`: from the host's side
  an unentitled account and a misconfigured one are indistinguishable and need
  the same operator action.
- **`ironclaw_product_contracts::transcription`** declares the product-facing
  port (`TranscriptionService`), its two DTOs, and the command descriptor. It
  is declared there, not in `ironclaw_assistant`, because
  `reborn_transport_product_boundary.rs` freezes the set of product symbols
  WebUI may name — a new one has to be declared at the boundary.
- **`ironclaw_composition::support::transcription`** builds the provider from
  the already-resolved `LlmConfig` and adapts it to the product port. That
  adapter is the error boundary: the classification crosses, the provider body
  is logged host-side and dropped.

### 3. Does the endpoint accept webm/opus — **no. Measured, and it forced a
re-encode.**

Answered empirically on 2026-08-17 by posting real fixtures through the live
route against NEAR AI:

| Container | Result |
|---|---|
| `audio/wav` | 200, exact transcript |
| `audio/ogg` (opus) | 200, exact transcript |
| `audio/mpeg` (mp3) | 200, exact transcript |
| `audio/flac` | 200, exact transcript |
| `audio/webm` (opus) — **Chrome/Firefox record this** | **400** `"Audio could not be processed … supported format"` |
| `audio/mp4` (aac) — **Safari records this** | **400**, same |

So the plan's preferred fix — "pick a `mimeType` both support at
`MediaRecorder` construction" — does not exist: the intersection of what
browsers record and what the endpoint decodes is empty. The first
implementation shipped exactly that assumption and failed in the browser on the
first try, with the composer showing `Invalid value (audio_base64)`.

The remaining option is a re-encode, and the plan called client-side re-encode
"heavy". It is much lighter than it sounds here, because the browser already
owns a decoder for the container it just wrote: `decodeAudioData` handles
webm/opus and mp4/aac natively, so `voice-encode.ts` only adds a downmix, a
linear resample, and a WAV header (~120 lines, no dependency). It re-encodes to
**16 kHz mono 16-bit WAV**, which is not a quality compromise — it is the rate
Whisper models work at — and it makes Chrome and Safari byte-identical from the
host's point of view. The server-side alternative (an ffmpeg/symphonia
transcode in the host) would have been genuinely heavy and would still have
needed a full AAC transcode for Safari.

Verified end to end after the fix: `webm → 16 kHz mono WAV → 200` and
`mp4 → 16 kHz mono WAV → 200`, both returning the exact spoken sentence.

**Consequence worth knowing:** the clip byte ceiling and the recorder's
duration hint are no longer independent. WAV at 16 kHz mono is a fixed
32,000 B/s, so the 300 s hint implies a 9.6 MB upload against the 10 MiB
ceiling. `voice_duration_ceiling_fits_the_byte_ceiling_as_wav` pins the two
together so raising one without the other fails the build.

`MediaRecorder`'s own container choice is now a purely browser-local
preference (`RECORDER_CONTAINERS` in `voice-recorder.ts`), not the server's
`voice.accept` list — the server list describes what may be *uploaded*, and
that is always WAV.

### 4. Spend budget — **deferred, deliberately**

Transcription does not currently reserve against
`IRONCLAW_BUDGET_USER_DAILY_USD`. The model-budget accountant prices *turn*
model calls through the cost table, and a transcription call is not a turn —
wiring it in means either a second accounting path or teaching the accountant a
non-turn call shape, which is the budget owner's design call, not a side effect
of shipping a mic button. What ships instead is a hard bound on abuse: the
route is rate-limited at 20 requests per caller per minute (vs. the shared
60/min mutation budget) and capped at a 10 MiB decoded clip. Left as a
follow-up for the budget owner.

## Two places this deviates from the plan, and why

**Egress does not go through `ironclaw_network`.** The plan listed that as a
standing invariant to keep. It is satisfied differently here:
`crates/domains/CLAUDE.md` charters `ironclaw_llm` as one of three
external-service cones where direct HTTP is allowed, and every model provider
in that crate already calls `reqwest` directly through the shared hardened
client builder (connect timeout, TCP keepalive, pool bound, and a
transcription-specific 120 s request budget). The transcription provider is one
more file inside that cone, using the same builder. Routing it through
`ironclaw_network` instead would make it the only member of the cone that does,
without adding a boundary the cone does not already have. The credential still
never leaves the host, which is the invariant that was actually load-bearing.

**The clip ceiling is 10 MiB, not 25 MB.** The plan's "≤ 25 MB" came from what
Whisper endpoints commonly cap at. The binding constraint here is nearer: the
WebUI gateway's body budget is 14 MiB, and base64 inflates a payload by 4/3, so
anything above ~10.5 MiB decoded would be rejected by the body-limit layer
before a handler ever saw it — a limit the user could neither see nor act on.
10 MiB decoded is what actually fits, matches the existing per-file attachment
ceiling, and is far above a 5-minute voice clip (~1–5 MiB at typical opus/AAC
bitrates). A test in `ironclaw_attachments` pins the two together so raising one
without the other fails.

## What shipped

| Layer | Change |
|---|---|
| `ironclaw_llm` | `TranscriptionErrorKind` + `ProviderStatus`; providers now report status instead of a formatted string |
| `ironclaw_attachments` | `VoiceClipBudget` / `DEFAULT_VOICE_CLIP_BUDGET` (10 MiB, 300 s) and `voice_capabilities()` beside the attachment budgets — one home for advertised-and-enforced ceilings |
| `ironclaw_product_contracts` | `transcription` module: `TranscriptionService` port, request/response DTOs, `TRANSCRIBE_AUDIO_COMMAND` |
| `ironclaw_assistant` | `DecodeVoiceClip` (registry media-type check, encoded- and decoded-byte ceilings, blank check), `RebornServices::{with_transcription, transcription_available, transcribe_audio}`, `ProductCommandHandler::TranscribeAudio` |
| `ironclaw_composition` | `support::transcription`: provider build from `LlmConfig` + `LlmTranscriptionService` error boundary; `RebornRuntime::voice_input_enabled()` |
| `ironclaw_webui` | `POST /api/webchat/v2/transcribe` descriptor + handler; `session.features.voice_input` and `session.voice` |
| `ironclaw_cli` | Reads `runtime.voice_input_enabled()` into the serve config |
| Frontend | `lib/voice.ts` (eager: server contract, capability probe, transcript insertion, `m:ss`), `lib/voice-recorder.ts` + `lib/voice-encode.ts` (lazy: `MediaRecorder` lifecycle, WAV re-encode, upload), `hooks/useVoiceInput.ts` (thin eager state wrapper), mic button + elapsed timer + inline retryable error in the composer, `mic`/`square` icons, voice strings in all 11 locales |
| Bundle | Recording engine + encoder load via dynamic `import()` on first mic press, so the initial /chat route pays only for the button; `check-bundle-budgets.ts` raised 222.0 → 224.0 KB with the deferral measured and the residue documented |
| Docs | `.env.example` (`IRONCLAW_TRANSCRIPTION_MODEL`), WebUI `CONTRACT.md` (route row, `voice` charter sub-owner, retention paragraph), llm `CONTRACT.md`, composition `CONTRACT.md` |

### Retention, as promised

Transcribe-and-discard, stated explicitly in
`crates/product/ironclaw_webui/CONTRACT.md`: the clip lives only for the
request, is never written to a store or mount or event, and only the transcript
survives — inside the browser's composer, until the user chooses to send it. A
clip the user never sent never became conversation data, so this is an
exception in shape rather than in spirit to "LLM data is never deleted".

### Never auto-sent

The transcript is inserted at the caret with spacing fixed up, the caret lands
just past it, and the user sends. A mis-transcription is always editable.
