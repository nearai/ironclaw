/**
 * u32 big-endian length-prefixed JSON frames over stdin/stdout, matching the
 * Rust `write_framed`/`read_framed` helpers in
 * `crates/loop/ironclaw_loop_host/src/remote_host/client.rs`.
 *
 * Invariant: stdout carries framed wire bytes ONLY. Every diagnostic goes to
 * stderr via `log`.
 */

import { LOOP_WORKER_MAX_FRAME_BYTES } from "./protocol.ts";

export function log(message: string): void {
  // Single stderr line; never touches stdout.
  console.error(`ironclaw-pi-worker: ${message}`);
}

/** Encode one value into a length-prefixed frame body. */
export function encodeFrame(value: unknown): Uint8Array {
  const body = new TextEncoder().encode(JSON.stringify(value));
  if (body.length > LOOP_WORKER_MAX_FRAME_BYTES) {
    throw new Error(
      `frame body of ${body.length} bytes exceeds the ${LOOP_WORKER_MAX_FRAME_BYTES}-byte limit`,
    );
  }
  const frame = new Uint8Array(4 + body.length);
  new DataView(frame.buffer).setUint32(0, body.length, false);
  frame.set(body, 4);
  return frame;
}

export function writeFrame(writer: Bun.FileSink, value: unknown): void {
  writer.write(encodeFrame(value));
  // FileSink.flush() returns a number (bytes flushed) synchronously, or a
  // promise when a flush was already in flight; both mean the bytes were
  // handed to the pipe. A failed stdout flush is fatal to the membrane, so
  // surface it on stderr and let the pending RPC hang until the host exits —
  // the process teardown reports it.
  const flushed = writer.flush();
  if (flushed instanceof Promise) {
    flushed.catch((error: unknown) => {
      log(`stdout flush failed: ${error}`);
    });
  }
}

export async function flushWriter(writer: Bun.FileSink): Promise<void> {
  const flushed = writer.flush();
  if (flushed instanceof Promise) {
    await flushed;
  }
}

/** Thrown when the peer closes the stream mid-frame (clean EOF is `null`). */
export class FrameReadError extends Error {
  constructor(
    message: string,
    readonly earlyEof: boolean,
  ) {
    super(message);
  }
}

/**
 * Length-prefixed byte reader over stdin with partial-frame handling: bytes
 * arriving in arbitrary chunks are buffered until a full frame body is
 * available. Returns `null` on clean EOF at a frame boundary.
 */
export class FrameReader {
  private buffer = new Uint8Array(0);

  constructor(private readonly stream: ReadableStream<Uint8Array>) {}

  async read(): Promise<Uint8Array | null> {
    const header = await this.readExact(4, true);
    if (header === null) {
      return null;
    }
    const length = new DataView(
      header.buffer,
      header.byteOffset,
      header.byteLength,
    ).getUint32(0, false);
    if (length > LOOP_WORKER_MAX_FRAME_BYTES) {
      throw new FrameReadError(
        `frame of ${length} bytes exceeds the ${LOOP_WORKER_MAX_FRAME_BYTES}-byte limit`,
        false,
      );
    }
    const body = await this.readExact(length, false);
    return body;
  }

  private async readExact(
    count: number,
    eofOk: boolean,
  ): Promise<Uint8Array | null> {
    const reader = this.stream.getReader();
    try {
      while (this.buffer.length < count) {
        const { done, value } = await reader.read();
        if (done) {
          if (this.buffer.length === 0 && eofOk) {
            return null;
          }
          throw new FrameReadError(
            `host stream ended mid-frame (${this.buffer.length} of ${count} bytes buffered)`,
            true,
          );
        }
        if (value && value.length > 0) {
          const merged = new Uint8Array(this.buffer.length + value.length);
          merged.set(this.buffer, 0);
          merged.set(value, this.buffer.length);
          this.buffer = merged;
        }
      }
      const body = this.buffer.slice(0, count);
      this.buffer = this.buffer.slice(count);
      return body;
    } finally {
      reader.releaseLock();
    }
  }
}

/** Decode one frame body into a typed value; throws on malformed JSON. */
export function decodeFrame<T>(body: Uint8Array): T {
  return JSON.parse(new TextDecoder().decode(body)) as T;
}
