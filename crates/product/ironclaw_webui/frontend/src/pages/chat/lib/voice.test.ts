// @ts-nocheck
// Unit tests for the composer's voice-capture helpers.
//
//   pnpm test -- pages/chat/lib/voice.test.ts
//
// This covers the EAGER half only — the server contract, the capability probe,
// and the transcript/timer arithmetic. The recording engine's helpers are
// tested in `voice-recorder.test.ts`, matching the module split that keeps the
// recorder out of the initial /chat bundle.

import assert from "node:assert/strict";
import { test } from "vitest";

import {
  FALLBACK_VOICE_LIMITS,
  browserSupportsVoiceCapture,
  formatElapsed,
  insertTranscript,
  voiceLimitsFromSession,
} from "./voice";

// --- server contract -------------------------------------------------------

test("voiceLimitsFromSession maps the wire shape and falls back cleanly", () => {
  assert.deepEqual(voiceLimitsFromSession(undefined), FALLBACK_VOICE_LIMITS);
  assert.deepEqual(voiceLimitsFromSession({}), FALLBACK_VOICE_LIMITS);

  assert.deepEqual(
    voiceLimitsFromSession({
      voice: { accept: ["audio/mp4"], max_bytes: 1024, max_duration_secs: 60 },
    }),
    { accept: ["audio/mp4"], maxBytes: 1024, maxDurationSecs: 60 },
  );
});

// A malformed field must not poison the whole contract: the recorder should
// keep the good half and fall back only on what is actually broken.
test("voiceLimitsFromSession keeps good fields when others are malformed", () => {
  const limits = voiceLimitsFromSession({
    voice: { accept: "audio/mp4", max_bytes: null, max_duration_secs: 45 },
  });
  assert.deepEqual(limits.accept, FALLBACK_VOICE_LIMITS.accept);
  assert.equal(limits.maxBytes, FALLBACK_VOICE_LIMITS.maxBytes);
  assert.equal(limits.maxDurationSecs, 45);
});

// --- transcript insertion --------------------------------------------------

test("insertTranscript inserts at the caret and reports where it ends", () => {
  const { text, caret } = insertTranscript("hello world", "there", 6, 6);
  assert.equal(text, "hello there world");
  assert.equal(caret, "hello there".length);
});

// Spacing is added only where it is missing, so dictating twice does not
// accumulate double spaces and dictating into an empty composer produces no
// leading one.
test("insertTranscript adds spacing only where it is missing", () => {
  assert.equal(insertTranscript("", "hello", 0, 0).text, "hello");
  assert.equal(insertTranscript("hello ", "world", 6, 6).text, "hello world");
  assert.equal(insertTranscript("hello", "world", 5, 5).text, "hello world");
  assert.equal(insertTranscript(" world", "hello", 0, 0).text, "hello world");
});

test("insertTranscript replaces a selection", () => {
  const { text } = insertTranscript("keep this bit", "that", 5, 9);
  assert.equal(text, "keep that bit");
});

// An unfocused composer reports no usable selection; appending is where an
// absent caret conceptually sits, and it must never throw or truncate.
test("insertTranscript appends when the selection is absent or out of range", () => {
  assert.equal(insertTranscript("draft", "more", null, null).text, "draft more");
  assert.equal(insertTranscript("draft", "more", 99, 99).text, "draft more");
  assert.equal(insertTranscript("draft", "more", -1, -1).text, "draft more");
});

// A whitespace-only transcript is a no-op rather than an inserted blank: the
// composer's text and caret must survive a recording that produced nothing.
test("insertTranscript leaves the draft untouched for an empty transcript", () => {
  const { text, caret } = insertTranscript("draft", "   ", 2, 2);
  assert.equal(text, "draft");
  assert.equal(caret, 5);
});

// --- elapsed label ---------------------------------------------------------

test("formatElapsed renders m:ss and clamps nonsense to zero", () => {
  assert.equal(formatElapsed(0), "0:00");
  assert.equal(formatElapsed(9), "0:09");
  assert.equal(formatElapsed(65), "1:05");
  assert.equal(formatElapsed(600), "10:00");
  assert.equal(formatElapsed(-5), "0:00");
  assert.equal(formatElapsed(NaN), "0:00");
  assert.equal(formatElapsed(undefined), "0:00");
});

// --- capability probe ------------------------------------------------------

// Both halves are required. `getUserMedia` in particular is absent on any
// non-secure context (plain-HTTP LAN access to a local deployment), which is
// a real deployment shape, not a hypothetical.
test("browserSupportsVoiceCapture requires both MediaRecorder and getUserMedia", () => {
  const originalRecorder = globalThis.MediaRecorder;
  const originalNavigator = globalThis.navigator;
  const setNavigator = (value) =>
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value,
    });

  try {
    globalThis.MediaRecorder = undefined;
    setNavigator({ mediaDevices: { getUserMedia: () => {} } });
    assert.equal(browserSupportsVoiceCapture(), false);

    globalThis.MediaRecorder = function () {};
    setNavigator({});
    assert.equal(browserSupportsVoiceCapture(), false);

    setNavigator({ mediaDevices: {} });
    assert.equal(browserSupportsVoiceCapture(), false);

    setNavigator({ mediaDevices: { getUserMedia: () => {} } });
    assert.equal(browserSupportsVoiceCapture(), true);
  } finally {
    globalThis.MediaRecorder = originalRecorder;
    setNavigator(originalNavigator);
  }
});
