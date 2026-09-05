/**
 * Worker-side RPC client over the framed stdio membrane — the TypeScript
 * counterpart of `StdioRpcClient` in
 * `crates/loop/ironclaw_loop_host/src/remote_host/client.rs`.
 *
 * - `call()` assigns monotonically increasing ids and demuxes `HostResponse`
 *   frames by id, so concurrent calls each resolve with their own response.
 * - `Cancel` frames surface as an `AbortSignal` the adapter can hand to Pi.
 * - `sendOutcome()` writes `Outcome` and waits for `OutcomeAck`; the caller
 *   then exits 0.
 */

import { FrameReader, flushWriter, log, writeFrame } from "./framing.ts";
import type {
  HostCall,
  HostFrame,
  LoopCancellationSignal,
  LoopWorkerOutcome,
  WireError,
} from "./protocol.ts";

/** Error carrying a host-side `WireError` (the `Err` of an RPC response). */
export class HostCallError extends Error {
  constructor(readonly wireError: WireError) {
    super(hostCallErrorMessage(wireError));
  }
}

interface PendingCall {
  resolve: (value: { Ok: unknown } | { Err: WireError } | undefined) => void;
  reject: (error: HostCallError) => void;
}

function hostCallErrorMessage(error: WireError): string {
  if ("Host" in error) {
    return `host ${error.Host.kind}: ${error.Host.safe_summary}`;
  }
  if ("Compaction" in error) {
    return `compaction error: ${JSON.stringify(error.Compaction)}`;
  }
  return `protocol error: ${error.Protocol}`;
}

/** Synthetic request id reserved for the Outcome -> OutcomeAck handshake. */
const OUTCOME_ACK_ID = 0;

export interface HostRpcOptions {
  stdin: ReadableStream<Uint8Array>;
  reader?: FrameReader;
  stdout: Bun.FileSink;
  /** Invoked when the host stream ends without a clean frame boundary. */
  onEarlyEof?: (message: string) => void;
}

export class HostRpc {
  private nextId = 1;
  private readonly pending = new Map<number, PendingCall>();
  private cancelled = false;
  private cancelSignalPayload: LoopCancellationSignal | null = null;
  private cancelWaiters: Array<() => void> = [];
  private readonly abortController = new AbortController();
  private readonly pumpPromise: Promise<void>;
  private closedError: HostCallError | null = null;
  private acknowledged = false;

  constructor(private readonly options: HostRpcOptions) {
    this.pumpPromise = this.pump();
  }

  /** AbortSignal aborted as soon as a `Cancel` frame arrives. */
  get signal(): AbortSignal {
    return this.abortController.signal;
  }

  get isCancelled(): boolean {
    return this.cancelled;
  }

  get cancellationPayload(): LoopCancellationSignal | null {
    return this.cancelSignalPayload;
  }

  /** Resolves once a `Cancel` frame has arrived. */
  waitCancelled(): Promise<void> {
    if (this.cancelled) {
      return Promise.resolve();
    }
    const { promise, resolve } = Promise.withResolvers<void>();
    this.cancelWaiters.push(resolve);
    return promise;
  }

  /**
   * Issue one `HostCall` and await its response. Concurrent calls are safe:
   * the pump demuxes `HostResponse` frames by request id.
   */
  async call<T>(call: HostCall): Promise<T> {
    if (this.closedError) throw this.closedError;
    const id = this.nextId;
    this.nextId += 1;
    const { promise, resolve, reject } = Promise.withResolvers<
      { Ok: unknown } | { Err: WireError }
    >();
    this.pending.set(id, { resolve, reject });
    writeFrame(this.options.stdout, { HostRequest: { id, call } });
    await flushWriter(this.options.stdout);
    const response = await promise;
    if ("Err" in response) {
      throw new HostCallError(response.Err);
    }
    return response.Ok as T;
  }

  /** Send the terminal `Outcome` frame and wait for `OutcomeAck`. */
  async sendOutcome(outcome: LoopWorkerOutcome): Promise<void> {
    if (this.closedError) throw this.closedError;
    const { promise, resolve, reject } = Promise.withResolvers<
      { Ok: unknown } | { Err: WireError } | undefined
    >();
    this.pending.set(OUTCOME_ACK_ID, { resolve, reject });
    writeFrame(this.options.stdout, { Outcome: outcome });
    await flushWriter(this.options.stdout);
    await promise;
  }

  /** Await pump termination (tests). */
  pumpFinished(): Promise<void> {
    return this.pumpPromise;
  }

  private async pump(): Promise<void> {
    const reader = this.options.reader ?? new FrameReader(this.options.stdin);
    try {
      while (true) {
        const frameBody = await reader.read();
        if (frameBody === null) {
          this.failAllPending(
            new HostCallError({ Protocol: "host stream ended" }),
          );
          this.options.onEarlyEof?.(
            "host closed stdin before the worker finished",
          );
          return;
        }
        this.handleFrame(
          JSON.parse(new TextDecoder().decode(frameBody)) as HostFrame,
        );
        if (this.acknowledged) return;
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.failAllPending(new HostCallError({ Protocol: message }));
      this.options.onEarlyEof?.(message);
    }
  }

  private handleFrame(frame: HostFrame): void {
    if (frame === "OutcomeAck") {
      const ack = this.pending.get(OUTCOME_ACK_ID);
      this.pending.delete(OUTCOME_ACK_ID);
      this.acknowledged = true;
      ack?.resolve(undefined);
      return;
    }
    if ("Cancel" in frame) {
      this.cancelled = true;
      this.cancelSignalPayload = frame.Cancel;
      this.abortController.abort();
      for (const waiter of this.cancelWaiters) {
        waiter();
      }
      this.cancelWaiters = [];
      return;
    }
    if ("HostResponse" in frame) {
      const pending = this.pending.get(frame.HostResponse.id);
      this.pending.delete(frame.HostResponse.id);
      if (pending) {
        pending.resolve(frame.HostResponse.result);
      } else {
        log(
          `received response for unknown request id ${frame.HostResponse.id}`,
        );
      }
      return;
    }
    // Bootstrap frames after startup are a protocol violation; ignore.
    log("ignoring unexpected Bootstrap frame after startup");
  }

  private failAllPending(error: HostCallError): void {
    this.closedError = error;
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }
}
