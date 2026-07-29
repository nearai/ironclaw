/**
 * The stdin link to the Rust parent (attested-signing §E2).
 *
 * The link carries two things over one pipe:
 *
 * 1. **The per-boot token**, as the first newline-terminated line. Stdin rather
 *    than argv (visible in `ps`) or the environment (visible in a crash dump).
 *
 * 2. **Liveness.** The parent holds its write end open for the process's whole
 *    life, so EOF here means the parent is gone — by clean exit, panic, or
 *    SIGKILL alike. That is the one signal that survives a parent the OS killed
 *    without warning, and it is why the token is a *line* rather than the whole
 *    stream: reading to EOF would spend the signal on startup.
 *
 * A sidecar that outlived its parent would be an orphan still bound to the
 * signing socket, which the next boot's child would then fail to bind or, worse,
 * displace — so EOF is a shutdown, not a warning.
 */

import type { Readable } from "node:stream";

/** A token line longer than this is a protocol error, not a token. */
const MAX_TOKEN_LINE_BYTES = 4096;

/**
 * Read the token line, then watch the pipe for the parent's death.
 *
 * Resolves as soon as the first newline arrives — it deliberately does not wait
 * for the stream to end, because the stream ending is the shutdown signal.
 * `onParentGone` fires at most once, and only after a token was accepted.
 */
export function readTokenAndWatchParent(
  stdin: Readable,
  onParentGone: () => void,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let buffered = "";
    let settled = false;

    const fail = (message: string) => {
      if (settled) {
        return;
      }
      settled = true;
      reject(new Error(message));
    };

    stdin.setEncoding("utf8");

    stdin.on("data", (chunk: string) => {
      if (settled) {
        // Post-token traffic is not part of the protocol; ignore it rather than
        // buffer it. Only the pipe closing means anything from here on.
        return;
      }
      buffered += chunk;
      const newline = buffered.indexOf("\n");
      if (newline === -1) {
        if (buffered.length > MAX_TOKEN_LINE_BYTES) {
          fail(`token line exceeded ${MAX_TOKEN_LINE_BYTES} bytes with no newline`);
        }
        return;
      }
      const token = buffered.slice(0, newline).trim();
      if (token.length === 0) {
        fail("no sidecar token supplied on stdin");
        return;
      }
      settled = true;
      resolve(token);
    });

    stdin.on("end", () => {
      if (!settled) {
        fail("no sidecar token supplied on stdin");
        return;
      }
      onParentGone();
    });

    stdin.on("error", (cause: Error) => {
      if (!settled) {
        fail(`stdin failed before a token arrived: ${cause.message}`);
        return;
      }
      // A broken pipe after startup is the parent dying by another name.
      onParentGone();
    });
  });
}
