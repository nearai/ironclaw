/**
 * In-process fake host for driving the worker's framed stdio membrane exactly
 * like `serve_loop_worker` does: it plays Bootstrap, answers HostRequests by
 * id, can inject Cancel frames, and reads Outcome frames.
 */

import { encodeFrame } from "../src/framing.ts";
import type {
  LoopWorkerBootstrap,
  LoopWorkerOutcome,
  WorkerFrame,
} from "../src/protocol.ts";

/** One scripted response for a host call matching a variant predicate. */
interface ScriptedReply {
  matches: (call: Record<string, unknown>) => boolean;
  reply: unknown;
}

export interface FakeHostOptions {
  bootstrap: LoopWorkerBootstrap;
  /** The first matching entry answers each HostRequest. */
  replies: ScriptedReply[];
}

export interface FakeHostHandle {
  /** Frames the worker wrote (HostRequests / Outcomes), decoded. */
  workerFrames: WorkerFrame[];
  /** Push a host frame to the worker (Bootstrap / Cancel / HostResponse / OutcomeAck). */
  sendHostFrame: (frame: unknown) => void;
  /** Outcome the worker sent, once observed. */
  outcomeSeen: Promise<LoopWorkerOutcome>;
  /** Close the host->worker stream (simulates early EOF). */
  closeHostToWorker: () => void;
  done: Promise<void>;
}

/**
 * Build the paired stdio plumbing and start the worker under test.
 * `workerMain` receives (stdin stream, stdout sink) like the real entrypoint.
 */
export function startFakeHost(
  options: FakeHostOptions,
  workerMain: (
    stdin: ReadableStream<Uint8Array>,
    stdout: Bun.FileSink,
  ) => Promise<void>,
): FakeHostHandle {
  // host -> worker: the worker's "stdin".
  const hostToWorker = new TransformStream<Uint8Array, Uint8Array>();
  const hostWriter = hostToWorker.writable.getWriter();

  // worker -> host: the worker's "stdout".
  const workerFrames: WorkerFrame[] = [];
  const outcomeResolver = Promise.withResolvers<LoopWorkerOutcome>();
  const doneResolver = Promise.withResolvers<void>();
  let workerBuffer = new Uint8Array(0);

  const yieldMacrotask = () =>
    new Promise<void>((resolve) => setTimeout(resolve, 0));

  const workerToHost = new TransformStream<Uint8Array, Uint8Array>();
  const hostReader = workerToHost.readable.getReader();

  const sink = makeSink(workerToHost.writable);

  async function pumpWorkerFrames(): Promise<void> {
    while (true) {
      const { done, value } = await hostReader.read();
      if (done) {
        doneResolver.resolve();
        return;
      }
      const merged = new Uint8Array(workerBuffer.length + value.length);
      merged.set(workerBuffer, 0);
      merged.set(value, workerBuffer.length);
      workerBuffer = merged;
      let wrote = false;
      while (workerBuffer.length >= 4) {
        const length = new DataView(
          workerBuffer.buffer,
          workerBuffer.byteOffset,
          workerBuffer.byteLength,
        ).getUint32(0, false);
        if (workerBuffer.length < 4 + length) {
          break;
        }
        const body = workerBuffer.slice(4, 4 + length);
        workerBuffer = workerBuffer.slice(4 + length);
        const frame = JSON.parse(new TextDecoder().decode(body)) as WorkerFrame;
        workerFrames.push(frame);
        if ("Outcome" in frame) {
          outcomeResolver.resolve(frame.Outcome);
          try {
            hostWriter.write(encodeFrame("OutcomeAck")).catch(() => {
              // The worker->host pipe is already torn down (early-EOF test).
            });
            wrote = true;
          } catch {
            // Ignore: the worker is already exiting.
          }
        } else if ("HostRequest" in frame) {
          const call = frame.HostRequest.call as Record<string, unknown>;
          const entry = options.replies.find((candidate) =>
            candidate.matches(call),
          );
          if (entry) {
            const reply =
              typeof entry.reply === "function"
                ? entry.reply(call)
                : entry.reply;
            const response: unknown =
              reply instanceof Error
                ? {
                    Err: {
                      Host: { kind: "internal", safe_summary: reply.message },
                    },
                  }
                : { Ok: reply };
            void hostWriter.write(
              encodeFrame({
                HostResponse: { id: frame.HostRequest.id, result: response },
              }),
            );
            wrote = true;
          }
        }
      }
      if (wrote) {
        // Bun 1.3 TransformStream quirk: a write scheduled on the
        // host->worker pipe must settle (macrotask) before the pump re-enters
        // read(), or the worker's pending request never observes the reply.
        await yieldMacrotask();
      }
    }
  }

  void pumpWorkerFrames();

  // Bootstrap first, always.
  void hostWriter.write(encodeFrame({ Bootstrap: options.bootstrap }));

  const run = workerMain(
    hostToWorker.readable,
    sink as unknown as Bun.FileSink,
  );
  void run.finally(() => doneResolver.resolve());

  return {
    workerFrames,
    sendHostFrame: (frame) => {
      void hostWriter.write(encodeFrame(frame));
    },
    outcomeSeen: outcomeResolver.promise,
    closeHostToWorker: () => {
      void hostWriter.close();
    },
    done: doneResolver.promise,
  };
}

/** Minimal Bun.FileSink-compatible sink over a WritableStream. */
function makeSink(writable: WritableStream<Uint8Array>): {
  write: (chunk: Uint8Array) => number;
  flush: () => number;
  end: () => number;
} {
  const writer = writable.getWriter();
  return {
    write: (chunk: Uint8Array) => {
      void writer.write(chunk);
      return chunk.length;
    },
    flush: () => 0,
    end: () => {
      void writer.close();
      return 0;
    },
  };
}
