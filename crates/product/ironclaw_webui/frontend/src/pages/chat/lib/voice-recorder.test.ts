// @ts-nocheck
// Unit tests for the lazily-loaded recording engine's pure helpers.
//
//   pnpm test -- pages/chat/lib/voice-recorder.test.ts
//
// These live apart from `voice.test.ts` because the module does: the recorder
// is dynamically imported on first microphone press so it stays out of the
// initial /chat bundle. `blobToBase64` reads through the DOM `FileReader`,
// which Node does not provide, so it gets a small stub.

import assert from "node:assert/strict";
import { beforeAll as before, test } from "vitest";

import { blobToBase64, pickRecorderMimeType } from "./voice-recorder";

// --- container selection ---------------------------------------------------

// The cross-browser story in one test: Chrome supports webm, Safari supports
// mp4, and the same accept list serves both because the choice is made by
// asking the browser rather than by sniffing it.
test("pickRecorderMimeType picks the first container this browser supports", () => {
  const accept = ["audio/webm", "audio/ogg", "audio/mp4"];
  const chrome = { isTypeSupported: (t) => t === "audio/webm" || t === "audio/ogg" };
  const safari = { isTypeSupported: (t) => t === "audio/mp4" };

  assert.equal(pickRecorderMimeType(accept, chrome), "audio/webm");
  assert.equal(pickRecorderMimeType(accept, safari), "audio/mp4");
});

// "No supported container" must be distinguishable from "picked one", because
// the caller turns it into "voice unavailable" rather than recording a
// container the server would reject.
test("pickRecorderMimeType returns empty when nothing matches", () => {
  const recorder = { isTypeSupported: () => false };
  assert.equal(pickRecorderMimeType(["audio/webm"], recorder), "");
  assert.equal(pickRecorderMimeType([], recorder), "");
  assert.equal(pickRecorderMimeType(undefined, recorder), "");
});

// Non-string / non-MIME entries in the server list must not be offered to the
// recorder — an extension token would construct a MediaRecorder that throws.
test("pickRecorderMimeType ignores tokens that are not media types", () => {
  const recorder = { isTypeSupported: () => true };
  assert.equal(pickRecorderMimeType([".webm", null, 7, "audio/webm"], recorder), "audio/webm");
});

// Older implementations lack `isTypeSupported`. Falling back to the first
// accepted type lets the recorder's own constructor be the judge, instead of
// silently reporting voice as unavailable everywhere.
test("pickRecorderMimeType falls back when isTypeSupported is missing", () => {
  assert.equal(pickRecorderMimeType(["audio/mp4", "audio/webm"], {}), "audio/mp4");
});

// --- blob encoding ---------------------------------------------------------

before(() => {
  // Minimal FileReader: `readAsDataURL` echoes whatever data URL the fake blob
  // carries, so the split-on-comma contract can be pinned without a browser.
  globalThis.FileReader = class {
    readAsDataURL(blob) {
      queueMicrotask(() => {
        if (blob.__fail) {
          this.error = new Error("read failed");
          this.onerror?.();
          return;
        }
        this.result = blob.__dataUrl;
        this.onload?.();
      });
    }
  };
});

test("blobToBase64 returns the payload without the data: prefix", async () => {
  const base64 = await blobToBase64({ __dataUrl: "data:audio/webm;base64,AQIDBA==" });
  assert.equal(base64, "AQIDBA==");
});

// A broken read must reject, not resolve empty: an empty payload would upload
// as a blank clip and come back as a confusing validation error.
test("blobToBase64 rejects a failed or malformed read", async () => {
  await assert.rejects(() => blobToBase64({ __fail: true }));
  await assert.rejects(() => blobToBase64({ __dataUrl: "not-a-data-url" }));
  await assert.rejects(() => blobToBase64({ __dataUrl: undefined }));
});
