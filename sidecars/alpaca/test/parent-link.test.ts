import { Readable } from "node:stream";

import { describe, expect, it, vi } from "vitest";

import { readTokenAndWatchParent } from "../src/parent-link.ts";

/** A stdin stand-in we can feed and close on demand. */
function pipe(): Readable & { feed: (chunk: string) => void; close: () => void } {
  const stream = new Readable({ read() {} }) as Readable & {
    feed: (chunk: string) => void;
    close: () => void;
  };
  stream.feed = (chunk: string) => stream.push(chunk);
  stream.close = () => stream.push(null);
  return stream;
}

describe("readTokenAndWatchParent", () => {
  it("resolves on the first newline without waiting for the stream to close", async () => {
    const stdin = pipe();
    const onParentGone = vi.fn();
    const token = readTokenAndWatchParent(stdin, onParentGone);

    stdin.feed("abc123\n");

    // The parent deliberately keeps the pipe open after the token: the open
    // pipe IS the liveness link. Resolving here proves we do not wait for EOF.
    await expect(token).resolves.toBe("abc123");
    expect(onParentGone).not.toHaveBeenCalled();
  });

  /**
   * The whole point of holding the pipe open. Any parent death — clean exit,
   * panic, SIGKILL — closes its end, and the child must not survive as an
   * orphan still bound to the signing socket.
   */
  it("fires the parent-gone callback when the pipe later closes", async () => {
    const stdin = pipe();
    const onParentGone = vi.fn();
    const token = readTokenAndWatchParent(stdin, onParentGone);
    stdin.feed("tok\n");
    await expect(token).resolves.toBe("tok");

    stdin.close();
    await new Promise((resolve) => setImmediate(resolve));
    expect(onParentGone).toHaveBeenCalledTimes(1);
  });

  it("rejects when the parent closes without ever sending a token", async () => {
    const stdin = pipe();
    const token = readTokenAndWatchParent(stdin, vi.fn());
    stdin.close();
    await expect(token).rejects.toThrow(/no sidecar token/i);
  });

  it("rejects an empty token line rather than serving with no credential", async () => {
    const stdin = pipe();
    const token = readTokenAndWatchParent(stdin, vi.fn());
    stdin.feed("   \n");
    await expect(token).rejects.toThrow(/no sidecar token/i);
  });

  /** A misbehaving or wrong-protocol parent must not be able to grow our heap. */
  it("rejects an unbounded stream that never sends a newline", async () => {
    const stdin = pipe();
    const token = readTokenAndWatchParent(stdin, vi.fn());
    stdin.feed("x".repeat(5000));
    await expect(token).rejects.toThrow(/exceeded/i);
  });
});
