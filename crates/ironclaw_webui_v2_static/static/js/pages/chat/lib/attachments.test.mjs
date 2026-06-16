// Unit tests for the composer's attachment staging helpers.
//
//   node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/attachments.test.mjs
//
// `stageFiles` reads bytes via the DOM `FileReader`, which Node does not
// provide, so we install a tiny stub that echoes a data URL the fake file
// carries. Everything else is pure.

import assert from "node:assert/strict";
import test, { before } from "node:test";

import {
  attachmentKindFromMime,
  attachmentPreviewMode,
  formatBytes,
  isAcceptedFile,
  normalizeStagedAttachment,
  stageFiles,
  toRenderAttachment,
  toWireAttachment,
} from "./attachments.js";

before(() => {
  globalThis.FileReader = class {
    readAsDataURL(file) {
      queueMicrotask(() => {
        this.result = file.__dataUrl;
        if (this.onload) this.onload();
      });
    }
  };
});

// A fake `File` carrying its own data URL so the stubbed reader is
// deterministic. Only `name`, `type`, `size` are touched by the helpers.
function fakeFile({ name, type, size, dataUrl }) {
  return { name, type, size, __dataUrl: dataUrl };
}

// Collect i18n keys instead of rendering them, so error assertions are
// stable against copy changes.
const tKey = (key) => key;

const LIMITS = {
  accept: ["image/*", "audio/*", ".pdf", ".txt", ".csv"],
  maxCount: 3,
  maxFileBytes: 1000,
  maxTotalBytes: 1500,
};

test("formatBytes renders human units", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1024), "1 KB");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(5 * 1024 * 1024), "5 MB");
  assert.equal(formatBytes(-1), "");
});

test("attachmentKindFromMime classifies by prefix", () => {
  assert.equal(attachmentKindFromMime("image/png"), "image");
  assert.equal(attachmentKindFromMime("AUDIO/MPEG"), "audio");
  assert.equal(attachmentKindFromMime("application/pdf"), "document");
  assert.equal(attachmentKindFromMime(""), "document");
});

test("attachmentPreviewMode maps each MIME family to its render mode", () => {
  assert.equal(attachmentPreviewMode("image/png"), "image");
  assert.equal(attachmentPreviewMode("AUDIO/MPEG"), "audio");
  assert.equal(attachmentPreviewMode("video/mp4"), "video");
  assert.equal(attachmentPreviewMode("application/pdf"), "pdf");
  // Text-like: text/*, JSON/XML/CSV and their +json/+xml suffixes.
  assert.equal(attachmentPreviewMode("text/plain"), "text");
  assert.equal(attachmentPreviewMode("text/markdown"), "text");
  assert.equal(attachmentPreviewMode("application/json"), "text");
  assert.equal(attachmentPreviewMode("application/csv"), "text");
  assert.equal(attachmentPreviewMode("application/vnd.api+json"), "text");
  assert.equal(attachmentPreviewMode("image/svg+xml"), "image");
  // Unknown binary falls back to download.
  assert.equal(attachmentPreviewMode("application/octet-stream"), "download");
  assert.equal(attachmentPreviewMode("application/zip"), "download");
  assert.equal(attachmentPreviewMode(""), "download");
});

test("isAcceptedFile honours wildcards, extensions, and exact MIME", () => {
  const accept = ["image/*", ".pdf"];
  assert.equal(isAcceptedFile({ type: "image/png", name: "a.png" }, accept), true);
  assert.equal(isAcceptedFile({ type: "", name: "doc.PDF" }, accept), true);
  assert.equal(isAcceptedFile({ type: "text/plain", name: "n.txt" }, accept), false);
  // An empty accept list defers entirely to the server.
  assert.equal(isAcceptedFile({ type: "application/x-evil", name: "x" }, []), true);
});

test("isAcceptedFile treats */* and * as accept-anything tokens", () => {
  assert.equal(isAcceptedFile({ type: "application/x-evil", name: "x" }, ["*/*"]), true);
  assert.equal(isAcceptedFile({ type: "", name: "x" }, ["*"]), true);
});

test("stageFiles produces the staged shape for an accepted file", async () => {
  const { staged, errors } = await stageFiles(
    [
      fakeFile({
        name: "shot.png",
        type: "image/png",
        size: 11,
        dataUrl: "data:image/png;base64,cG5n",
      }),
    ],
    { limits: LIMITS, existing: [], t: tKey },
  );

  assert.equal(errors.length, 0);
  assert.equal(staged.length, 1);
  const att = staged[0];
  assert.equal(att.filename, "shot.png");
  assert.equal(att.mimeType, "image/png");
  assert.equal(att.kind, "image");
  assert.equal(att.sizeBytes, 11);
  assert.equal(att.dataBase64, "cG5n");
  // Images carry a preview URL for an instant thumbnail; others do not.
  assert.equal(att.previewUrl, "data:image/png;base64,cG5n");

  assert.deepEqual(toWireAttachment(att), {
    mime_type: "image/png",
    filename: "shot.png",
    data_base64: "cG5n",
  });
  assert.deepEqual(toRenderAttachment(att), {
    id: att.id,
    filename: "shot.png",
    mime_type: "image/png",
    kind: "image",
    size_label: att.sizeLabel,
    preview_url: "data:image/png;base64,cG5n",
  });
});

test("stageFiles: a document gets no preview URL", async () => {
  const { staged } = await stageFiles(
    [
      fakeFile({
        name: "notes.txt",
        type: "text/plain",
        size: 4,
        dataUrl: "data:text/plain;base64,bm90ZQ==",
      }),
    ],
    { limits: LIMITS, existing: [], t: tKey },
  );
  assert.equal(staged.length, 1);
  assert.equal(staged[0].kind, "document");
  assert.equal(staged[0].previewUrl, null);
});

test("stageFiles rejects an unsupported type with a clear error", async () => {
  const { staged, errors } = await stageFiles(
    [fakeFile({ name: "a.exe", type: "application/x-msdownload", size: 5, dataUrl: "data:;base64," })],
    { limits: LIMITS, existing: [], t: tKey },
  );
  assert.equal(staged.length, 0);
  assert.deepEqual(errors, ["chat.attachmentUnsupportedType"]);
});

test("stageFiles rejects an oversized file", async () => {
  const { staged, errors } = await stageFiles(
    [fakeFile({ name: "big.pdf", type: "application/pdf", size: 2000, dataUrl: "data:application/pdf;base64,AA==" })],
    { limits: LIMITS, existing: [], t: tKey },
  );
  assert.equal(staged.length, 0);
  assert.deepEqual(errors, ["chat.attachmentTooLarge"]);
});

test("stageFiles enforces the per-message count budget against existing", async () => {
  const existing = [
    { sizeBytes: 1 },
    { sizeBytes: 1 },
    { sizeBytes: 1 },
  ];
  const { staged, errors } = await stageFiles(
    [fakeFile({ name: "c.txt", type: "text/plain", size: 1, dataUrl: "data:text/plain;base64,YQ==" })],
    { limits: LIMITS, existing, t: tKey },
  );
  assert.equal(staged.length, 0);
  assert.deepEqual(errors, ["chat.attachmentTooMany"]);
});

test("stageFiles enforces the total-bytes budget across the batch", async () => {
  const files = [
    fakeFile({ name: "a.txt", type: "text/plain", size: 900, dataUrl: "data:text/plain;base64,YQ==" }),
    fakeFile({ name: "b.txt", type: "text/plain", size: 900, dataUrl: "data:text/plain;base64,Yg==" }),
  ];
  const { staged, errors } = await stageFiles(files, {
    limits: LIMITS,
    existing: [],
    t: tKey,
  });
  // First fits (900 <= 1000 file, 900 <= 1500 total); second pushes the
  // running total to 1800 > 1500.
  assert.equal(staged.length, 1);
  assert.deepEqual(errors, ["chat.attachmentTotalTooLarge"]);
});

test("stageFiles skips an over-budget file but still stages a later fitting one", async () => {
  const files = [
    fakeFile({ name: "a.txt", type: "text/plain", size: 900, dataUrl: "data:text/plain;base64,YQ==" }),
    // Pushes the total to 1800 > 1500 — skipped, not fatal.
    fakeFile({ name: "big.txt", type: "text/plain", size: 900, dataUrl: "data:text/plain;base64,Yg==" }),
    // Still fits in the remaining budget (900 + 100 = 1000 <= 1500).
    fakeFile({ name: "c.txt", type: "text/plain", size: 100, dataUrl: "data:text/plain;base64,Yw==" }),
  ];
  const { staged, errors } = await stageFiles(files, {
    limits: LIMITS,
    existing: [],
    t: tKey,
  });
  assert.deepEqual(
    staged.map((att) => att.filename),
    ["a.txt", "c.txt"],
  );
  // The over-budget notice is recorded once, not once per skipped file.
  assert.deepEqual(errors, ["chat.attachmentTotalTooLarge"]);
});

test("stageFiles rejects a file whose reader yields a non-string result", async () => {
  // A FileReader that produces a non-string `result` (e.g. null) must not
  // crash `splitDataUrl`'s `.indexOf`; the file is rejected with a read error.
  const file = fakeFile({ name: "weird.txt", type: "text/plain", size: 10, dataUrl: null });
  const { staged, errors } = await stageFiles([file], {
    limits: LIMITS,
    existing: [],
    t: tKey,
  });
  assert.deepEqual(staged, []);
  assert.deepEqual(errors, ["chat.attachmentReadFailed"]);
});

test("normalizeStagedAttachment passes camelCase (web stageFiles) through unchanged", () => {
  const att = {
    id: "staged-1",
    filename: "report.pdf",
    mimeType: "application/pdf",
    kind: "document",
    sizeBytes: 2048,
    sizeLabel: "2.0 KB",
    dataBase64: "QkFTRTY0",
    previewUrl: null,
  };
  const out = normalizeStagedAttachment(att);
  assert.equal(out.mimeType, "application/pdf");
  assert.equal(out.dataBase64, "QkFTRTY0");
  assert.equal(out.sizeBytes, 2048);
  assert.equal(out.filename, "report.pdf");
  assert.equal(out.kind, "document");
});

test("normalizeStagedAttachment maps desktop snake_case (useComposerAttachments) to the canonical shape", () => {
  // Desktop extractable doc: extracted text already swapped into base64 with a
  // text/plain mime by useComposerAttachments.
  const att = {
    id: "att-3",
    filename: "services-template.docx",
    mime_type: "text/plain",
    base64: "TVNBLUNMQVVTRS0xNw==",
    size: 4096,
  };
  const out = normalizeStagedAttachment(att);
  assert.equal(out.mimeType, "text/plain", "snake mime_type maps to mimeType");
  assert.equal(out.dataBase64, "TVNBLUNMQVVTRS0xNw==", "snake base64 maps to dataBase64");
  assert.equal(out.sizeBytes, 4096, "snake size maps to sizeBytes");
  assert.equal(out.filename, "services-template.docx");
  assert.equal(out.kind, attachmentKindFromMime("text/plain"), "kind derived from mime when absent");
  assert.equal(typeof out.sizeLabel, "string");
});

test("normalizeStagedAttachment output survives toWireAttachment + toRenderAttachment for desktop input", () => {
  const out = normalizeStagedAttachment({
    filename: "scope-summary.html",
    mime_type: "text/html",
    base64: "PGgxPlNjb3BlPC9oMT4=",
    size: 64,
  });
  const wire = toWireAttachment(out);
  assert.equal(wire.mime_type, "text/html");
  assert.equal(wire.data_base64, "PGgxPlNjb3BlPC9oMT4=");
  assert.equal(wire.filename, "scope-summary.html");
  const render = toRenderAttachment(out);
  assert.equal(render.mime_type, "text/html");
  assert.equal(render.filename, "scope-summary.html");
  assert.ok(render.kind, "render attachment carries a kind for the chip");
});
