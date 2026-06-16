import assert from "node:assert/strict";
import test from "node:test";

// Stub fetch unconditionally so the microtask-deferred start() never makes
// real network calls. Two cases:
//   - signal already aborted (closed() helper) → immediate AbortError, caught silently.
//   - signal live (read() tests, stream not yet closed) → hang forever; start() waits
//     on the fetch Promise that never resolves, leaving read() free to run.
globalThis.fetch = async (_url, opts) => {
  if (opts?.signal?.aborted)
    throw Object.assign(new Error("Aborted"), { name: "AbortError" });
  return new Promise(() => {}); // hang — start() blocks on this, never interfering
};

import { FetchEventStream } from "./api.js";

// ─── helpers ───────────────────────────────────────────────────────────────

// Close immediately so the microtask-scheduled start() gets a pre-aborted
// signal and becomes a no-op. Tests then drive dispatchSseChunk() directly.
function closed() {
  const s = new FetchEventStream("http://test.local/", null);
  s.close();
  return s;
}

// Create an open stream for read() integration tests. start() is safely
// neutralised by the fetch stub above (hangs on an unresolved Promise).
function open() {
  const s = new FetchEventStream("http://test.local/", null);
  s.onerror = () => {}; // swallow any background errors
  return s;
}

// Collect all events of the given type from a stream into an array.
function collect(stream, type = "message") {
  const events = [];
  stream.addEventListener(type, (e) => events.push(e));
  return events;
}

// Build a ReadableStream from a string (UTF-8 encoded).
function bodyFrom(text) {
  const bytes = new TextEncoder().encode(text);
  return new ReadableStream({
    start(ctrl) {
      ctrl.enqueue(bytes);
      ctrl.close();
    },
  });
}

// ─── dispatchSseChunk: field parsing ───────────────────────────────────────

test("dispatchSseChunk: basic data field dispatches message event", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("data: hello");
  assert.equal(events.length, 1);
  assert.equal(events[0].type, "message");
  assert.equal(events[0].data, "hello");
});

test("dispatchSseChunk: event field changes dispatch type", () => {
  const s = closed();
  const deltas = collect(s, "delta");
  const messages = collect(s);
  s.dispatchSseChunk("event: delta\ndata: token");
  assert.equal(deltas.length, 1);
  assert.equal(deltas[0].data, "token");
  assert.equal(messages.length, 0);
});

test("dispatchSseChunk: multiple data lines join with newline", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("data: line1\ndata: line2\ndata: line3");
  assert.equal(events.length, 1);
  assert.equal(events[0].data, "line1\nline2\nline3");
});

test("dispatchSseChunk: id field captured in lastEventId", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("id: 42\ndata: hello");
  assert.equal(events[0].lastEventId, "42");
});

test("dispatchSseChunk: leading space stripped from field value", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("data: hello"); // one leading space → stripped
  assert.equal(events[0].data, "hello");
});

test("dispatchSseChunk: no leading space — raw value used", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("data:no-space");
  assert.equal(events[0].data, "no-space");
});

// ─── dispatchSseChunk: suppression cases ───────────────────────────────────

test("dispatchSseChunk: comment line (:) produces no event", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk(":heartbeat");
  assert.equal(events.length, 0);
});

test("dispatchSseChunk: chunk with no data field produces no event", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("event: ping"); // event but no data
  assert.equal(events.length, 0);
});

test("dispatchSseChunk: empty chunk produces no event", () => {
  const s = closed();
  const events = collect(s);
  s.dispatchSseChunk("");
  assert.equal(events.length, 0);
});

// ─── onmessage shorthand ───────────────────────────────────────────────────

test("onmessage callback receives message events", () => {
  const s = closed();
  const received = [];
  s.onmessage = (e) => received.push(e);
  s.dispatchSseChunk("data: direct");
  assert.equal(received.length, 1);
  assert.equal(received[0].data, "direct");
});

test("onmessage shorthand does not fire for named events", () => {
  const s = closed();
  const received = [];
  s.onmessage = (e) => received.push(e);
  s.dispatchSseChunk("event: delta\ndata: token");
  assert.equal(received.length, 0);
});

// ─── addEventListener / removeEventListener ─────────────────────────────────

test("removeEventListener stops delivery", () => {
  const s = closed();
  const events = [];
  const handler = (e) => events.push(e);
  s.addEventListener("message", handler);
  s.dispatchSseChunk("data: first");
  s.removeEventListener("message", handler);
  s.dispatchSseChunk("data: second");
  assert.equal(events.length, 1);
  assert.equal(events[0].data, "first");
});

test("multiple listeners all receive the event", () => {
  const s = closed();
  const a = [];
  const b = [];
  s.addEventListener("message", (e) => a.push(e.data));
  s.addEventListener("message", (e) => b.push(e.data));
  s.dispatchSseChunk("data: shared");
  assert.deepEqual(a, ["shared"]);
  assert.deepEqual(b, ["shared"]);
});

// ─── dispatchError + close ──────────────────────────────────────────────────

test("dispatchError silenced after close()", () => {
  const s = closed(); // already closed
  let errorFired = false;
  s.onerror = () => { errorFired = true; };
  s.dispatchError(new Error("should be suppressed"));
  assert.equal(errorFired, false);
});

test("dispatchError fires onerror when not closed", () => {
  const s = new FetchEventStream("http://test.local/", null);
  // Do NOT close — so onerror should fire
  const errors = [];
  s.onerror = (e) => errors.push(e);
  s.dispatchError(new Error("connection failed"));
  s.close(); // now close so the start() microtask is silenced
  assert.equal(errors.length, 1);
  assert.equal(errors[0].message, "connection failed");
});

// ─── read(): framing integration ───────────────────────────────────────────

test("read: dispatches two events separated by double newline", async () => {
  const s = open();
  const events = collect(s);
  await s.read(bodyFrom("data: first\n\ndata: second\n\n"));
  s.close();
  assert.equal(events.length, 2);
  assert.equal(events[0].data, "first");
  assert.equal(events[1].data, "second");
});

test("read: normalises CRLF to LF before framing", async () => {
  const s = open();
  const events = collect(s);
  await s.read(bodyFrom("data: crlf\r\n\r\n"));
  s.close();
  assert.equal(events.length, 1);
  assert.equal(events[0].data, "crlf");
});

test("read: heartbeat-only chunk between events does not produce an event", async () => {
  const s = open();
  const events = collect(s);
  await s.read(bodyFrom("data: one\n\n:heartbeat\n\ndata: two\n\n"));
  s.close();
  assert.equal(events.length, 2);
  assert.equal(events[0].data, "one");
  assert.equal(events[1].data, "two");
});

test("read: partial buffer flushed at stream end if non-empty", async () => {
  const s = open();
  const events = collect(s);
  // No trailing \n\n — the flush-at-end path catches it.
  await s.read(bodyFrom("data: trailing"));
  s.close();
  assert.equal(events.length, 1);
  assert.equal(events[0].data, "trailing");
});
