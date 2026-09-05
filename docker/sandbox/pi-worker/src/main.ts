/**
 * IronClaw Pi loop worker entrypoint.
 *
 * Runs over stdio: reads `HostFrame`s (u32 big-endian length-prefixed JSON,
 * 1 MiB ceiling) on stdin, writes `WorkerFrame`s on stdout. Diagnostics go to
 * stderr only. Protocol mirrors
 * `crates/loop/ironclaw_loop_host/src/remote_host/protocol.rs` (wire v2).
 *
 * Lifecycle (mirrors `run_loop_worker_stdio` in
 * `crates/loop/ironclaw_turn_runner/src/sandboxed_planned_driver.rs`):
 * 1. read the `Bootstrap` frame; reject unknown wire versions
 * 2. drive the Pi agent loop through the host RPC membrane
 * 3. send `Outcome`, wait for `OutcomeAck`, exit 0
 *
 * Exit codes: 0 after OutcomeAck; 1 on early host EOF or a fatal wire error
 * (with one stderr line); 2 when the first frame is not a valid bootstrap.
 */

import { FrameReader, log } from "./framing.ts";
import { HostRpc } from "./host.ts";
import {
  LOOP_WORKER_WIRE_VERSION,
  type HostFrame,
  type LoopWorkerBootstrap,
} from "./protocol.ts";
import { runPiWorker } from "./pi-adapter.ts";

async function main(): Promise<void> {
  const stdin = Bun.stdin.stream();
  const stdout = Bun.stdout.writer();

  // Read the bootstrap before starting the RPC pump so the HostRpc never sees
  // a `Bootstrap` frame.
  const reader = new FrameReader(stdin);
  let bootstrapBody: Uint8Array;
  try {
    const body = await reader.read();
    if (body === null) {
      log("host closed stdin before sending a bootstrap");
      process.exit(1);
    }
    bootstrapBody = body;
  } catch (error) {
    log(
      `bootstrap frame read failed: ${error instanceof Error ? error.message : String(error)}`,
    );
    process.exit(1);
  }

  let bootstrap: LoopWorkerBootstrap;
  try {
    const frame = JSON.parse(
      new TextDecoder().decode(bootstrapBody),
    ) as HostFrame;
    if (typeof frame !== "object" || !("Bootstrap" in frame)) {
      log("first host frame was not a Bootstrap frame");
      process.exit(2);
    }
    bootstrap = frame.Bootstrap;
  } catch (error) {
    log(
      `bootstrap frame was malformed: ${error instanceof Error ? error.message : String(error)}`,
    );
    process.exit(2);
  }

  if (bootstrap.wire_version !== LOOP_WORKER_WIRE_VERSION) {
    // Report the version mismatch as a Failed outcome so the host surfaces it
    // in run diagnostics, then exit non-zero.
    log(
      `unsupported bootstrap wire_version ${bootstrap.wire_version} (expected ${LOOP_WORKER_WIRE_VERSION})`,
    );
    await failFast(
      stdout,
      "invalid_wire_version",
      `bootstrap wire_version ${bootstrap.wire_version} is not supported (expected ${LOOP_WORKER_WIRE_VERSION})`,
    );
    process.exit(2);
  }

  let earlyEof = false;
  const host = new HostRpc({
    stdin,
    reader,
    stdout,
    onEarlyEof: (message) => {
      earlyEof = true;
      log(message);
    },
  });

  let outcome = await runPiWorker(bootstrap, host);
  if (earlyEof) {
    // The host died mid-run: exit non-zero with the stderr line already emitted.
    process.exit(1);
  }

  try {
    await host.sendOutcome(outcome);
  } catch (error) {
    log(
      `outcome send failed: ${error instanceof Error ? error.message : String(error)}`,
    );
    process.exit(1);
  }

  process.exit(0);
}

/** Send a `Failed` outcome without a full pump (wire-version failures). */
async function failFast(
  stdout: Bun.FileSink,
  kind: string,
  detail: string,
): Promise<void> {
  const frame = { Outcome: { Failed: { kind, detail } } };
  stdout.write(
    (() => {
      const body = new TextEncoder().encode(JSON.stringify(frame));
      const framed = new Uint8Array(4 + body.length);
      new DataView(framed.buffer).setUint32(0, body.length, false);
      framed.set(body, 4);
      return framed;
    })(),
  );
  const flushed = stdout.flush();
  if (flushed instanceof Promise) {
    await flushed;
  }
}

await main();
