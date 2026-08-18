// Voice-capture helpers for the composer's microphone.
//
// The browser records a clip with `MediaRecorder`, the host transcribes it,
// and the transcript is inserted into the composer for the user to edit. The
// audio is never uploaded as an attachment and is never persisted: it exists
// as a Blob in this tab until the transcript comes back, and is dropped.
//
// This module is the EAGER half: only what the composer needs before anyone
// presses the microphone — the server contract, the "can this browser record
// at all" probe, and the transcript/timer arithmetic the composer renders with.
// Everything that actually records (container selection, `MediaRecorder`,
// base64 encoding, the upload) lives in `voice-recorder.ts`, which
// `useVoiceInput` pulls in with a dynamic `import()` on first click. The split
// is a bundle-budget one: a session that never dictates should not pay for the
// recorder, and `check-bundle-budgets.ts` measures the /chat closure.

// Conservative defaults used only until `GET /session` resolves the server's
// voice contract. The server re-validates, so drift here only changes how
// early the recorder gives up.
export const FALLBACK_VOICE_LIMITS = {
  // `audio/webm` is Chrome/Firefox, `audio/mp4` is Safari — one list covers
  // both because the recorder picks the first entry the browser supports.
  accept: ["audio/webm", "audio/ogg", "audio/mp4"],
  maxBytes: 10 * 1024 * 1024,
  maxDurationSecs: 300,
};

// Map `session.voice` (snake_case wire shape from `VoiceCapabilities`) into
// the camelCase limits the recorder consumes.
export function voiceLimitsFromSession(session) {
  const v = session?.voice;
  if (!v) return FALLBACK_VOICE_LIMITS;
  return {
    accept: Array.isArray(v.accept)
      ? v.accept.filter((token) => typeof token === "string")
      : FALLBACK_VOICE_LIMITS.accept,
    maxBytes: Number.isFinite(v.max_bytes)
      ? v.max_bytes
      : FALLBACK_VOICE_LIMITS.maxBytes,
    maxDurationSecs: Number.isFinite(v.max_duration_secs)
      ? v.max_duration_secs
      : FALLBACK_VOICE_LIMITS.maxDurationSecs,
  };
}

/**
 * Insert transcript text into composer text at a cursor position.
 *
 * Returns the new text plus where the caret should land (just past the
 * insertion), so the user can keep typing where they left off. Spacing is
 * added only where it is missing, so dictating twice in a row does not produce
 * a double space and dictating at the start does not produce a leading one.
 *
 * A `null`/out-of-range selection (a composer that was never focused) appends
 * at the end, which is where an unfocused caret conceptually sits.
 */
export function insertTranscript(text, transcript, selectionStart, selectionEnd) {
  const existing = text || "";
  const insert = (transcript || "").trim();
  if (!insert) return { text: existing, caret: existing.length };

  const inRange = (value) =>
    Number.isInteger(value) && value >= 0 && value <= existing.length;
  const start = inRange(selectionStart) ? selectionStart : existing.length;
  const end = inRange(selectionEnd) && selectionEnd >= start ? selectionEnd : start;

  const before = existing.slice(0, start);
  const after = existing.slice(end);
  const needsLeadingSpace = before.length > 0 && !/\s$/.test(before);
  const needsTrailingSpace = after.length > 0 && !/^\s/.test(after);
  const piece = `${needsLeadingSpace ? " " : ""}${insert}${needsTrailingSpace ? " " : ""}`;

  return {
    text: `${before}${piece}${after}`,
    caret: before.length + piece.length - (needsTrailingSpace ? 1 : 0),
  };
}

/** `m:ss` elapsed-time label for the recording indicator. */
export function formatElapsed(seconds) {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

/**
 * Whether this browser can record at all.
 *
 * Both halves are required and both are absent in real deployments:
 * `MediaRecorder` is missing on older browsers, and `getUserMedia` is missing
 * on any page not served over a secure context (plain-HTTP LAN access to a
 * local deployment is the common case). Checking here is what keeps the
 * microphone button from appearing where pressing it could only fail.
 */
export function browserSupportsVoiceCapture() {
  return (
    typeof globalThis.MediaRecorder === "function" &&
    typeof globalThis.navigator?.mediaDevices?.getUserMedia === "function"
  );
}
