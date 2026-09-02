// The app-root session event client: one header-authenticated `fetch`
// stream per page (`POST /api/webchat/v2/session/events`, answered as
// `text/event-stream`), multiplexing independent typed logical subscriptions.
//
// Ownership rules (2026-08-13 session-transport design §7.3/§7.5):
// - Commands never travel here; the stream is read-only event transport.
// - Each logical subscription keeps its OWN resume cursor; every (re)connect
//   names every selector with its last delivered cursor. There is no
//   session-wide cursor.
// - The subscription set is fixed per connection: subscribing or
//   unsubscribing reconnects (debounced) with the new set, which is the same
//   resume path the client runs on lifetime expiry and on every drop.
// - The stream stays connected while the page is hidden — background
//   delivery is the point of the session transport.
// - A `subscription_error` fails only that subscription: retryable ones are
//   resubscribed on the next connect cycle; non-retryable ones are dropped.
// - Stale generations are ignored: frames stamped with a generation older
//   than the one the current connection admitted never deliver.
// - There is no fallback transport. A stream that cannot connect keeps
//   retrying with capped backoff and reports `reconnecting`; durable cursors
//   guarantee nothing is lost while disconnected.
//
// This module is dynamically imported by its hooks so its bytes stay out of
// the eager /chat closure (bundle budget).

import { sessionEventsStreamRequest } from "../api";
import {
  type SessionSelector,
  type SessionServerFrame,
  parseServerFrame,
  sessionEventsRequestBody,
} from "./protocol";

const RETRY_BASE_MS = 1_000;
const RETRY_MAX_MS = 30_000;
const RETRY_JITTER_RATIO = 0.2;
// The server sends a keep-alive comment every 15 s; three missed intervals
// means the connection is dead even though the socket has not closed.
const LIVENESS_DEADLINE_MS = 45_000;
const LIVENESS_CHECK_INTERVAL_MS = 15_000;
// Subscription-set changes within this window coalesce into one reconnect
// (route transitions subscribe the new thread and unsubscribe the old one in
// the same tick).
const RESYNC_DEBOUNCE_MS = 25;
// Delay before reconnecting to resubscribe a selector the server failed
// retryably, so a persistent condition cannot become a connect/error spin.
const SUBSCRIPTION_RETRY_DELAY_MS = 2_000;

export type SessionSubscriptionEvent = {
  cursor: string | null;
  body: Record<string, unknown>;
};

export type SessionSubscriptionError = {
  error: string;
  kind: string;
  retryable: boolean;
  lastCursor: string | null;
};

export type SessionSubscriptionHandlers = {
  onEvent: (event: SessionSubscriptionEvent) => void;
  // Terminal for this subscription attempt. When `retryable`, the client
  // keeps the subscription registered and resubscribes (from lastCursor when
  // the server supplied one) on the next connect cycle; the owner may rebase
  // local state. When not retryable the subscription is dropped.
  onError?: (error: SessionSubscriptionError) => void;
  onStatus?: (status: SessionTransportStatus) => void;
};

export type SessionTransportStatus = "connecting" | "open" | "reconnecting";

type Registration = {
  subscriptionId: string;
  selector: SessionSelector;
  cursor: string | null;
  generation: number | null;
  handlers: SessionSubscriptionHandlers;
};

export type StreamResponse = {
  status: number;
  body: ReadableStream<Uint8Array> | null;
};

export type StreamOpener = (input: {
  url: string;
  headers: Record<string, string>;
  body: string;
  signal: AbortSignal;
}) => Promise<StreamResponse>;

const defaultStreamOpener: StreamOpener = async ({ url, headers, body, signal }) => {
  const response = await fetch(url, {
    method: "POST",
    credentials: "same-origin",
    headers: {
      ...headers,
      Accept: "text/event-stream",
      "Content-Type": "application/json",
    },
    body,
    signal,
  });
  return { status: response.status, body: response.body };
};

let nextSubscriptionSuffix = 0;

export class SessionEventClient {
  private registrations = new Map<string, Registration>();
  private controller: AbortController | null = null;
  private streamGeneration = 0;
  private status: SessionTransportStatus = "reconnecting";
  private retryAttempt = 0;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private resyncTimer: ReturnType<typeof setTimeout> | null = null;
  private livenessTimer: ReturnType<typeof setInterval> | null = null;
  private lastBytesAt = 0;
  private connecting = false;
  private disposed = false;

  constructor(
    private readonly openStream: StreamOpener = defaultStreamOpener,
    private readonly request: () => {
      url: string;
      headers: () => Record<string, string>;
    } = sessionEventsStreamRequest,
    private readonly resyncDebounceMs: number = RESYNC_DEBOUNCE_MS,
    private readonly subscriptionRetryDelayMs: number = SUBSCRIPTION_RETRY_DELAY_MS,
  ) {
    if (typeof window !== "undefined") {
      window.addEventListener("online", this.handleOnline);
    }
  }

  currentStatus(): SessionTransportStatus {
    return this.status;
  }

  subscribe(
    selector: SessionSelector,
    handlers: SessionSubscriptionHandlers,
    options: { fromCursor?: string | null; idPrefix?: string } = {},
  ): { unsubscribe: () => void } {
    nextSubscriptionSuffix += 1;
    const subscriptionId = `${options.idPrefix ?? "sub"}-${nextSubscriptionSuffix}`;
    this.registrations.set(subscriptionId, {
      subscriptionId,
      selector,
      cursor: options.fromCursor ?? null,
      generation: null,
      handlers,
    });
    handlers.onStatus?.(this.currentStatus());
    this.scheduleResync(this.resyncDebounceMs);
    return {
      unsubscribe: () => {
        if (this.registrations.delete(subscriptionId)) {
          this.scheduleResync(this.resyncDebounceMs);
        }
      },
    };
  }

  dispose(): void {
    this.disposed = true;
    if (typeof window !== "undefined") {
      window.removeEventListener("online", this.handleOnline);
    }
    this.clearTimers();
    this.teardownStream();
    this.registrations.clear();
  }

  private handleOnline = () => {
    if (this.registrations.size > 0 && !this.controller) {
      this.retryAttempt = 0;
      this.scheduleResync(0);
    }
  };

  private setStatus(status: SessionTransportStatus) {
    this.status = status;
    for (const registration of this.registrations.values()) {
      registration.handlers.onStatus?.(status);
    }
  }

  private clearTimers() {
    if (this.connectTimer) {
      clearTimeout(this.connectTimer);
      this.connectTimer = null;
    }
    if (this.resyncTimer) {
      clearTimeout(this.resyncTimer);
      this.resyncTimer = null;
    }
    this.stopLivenessWatch();
  }

  private stopLivenessWatch() {
    if (this.livenessTimer) {
      clearInterval(this.livenessTimer);
      this.livenessTimer = null;
    }
  }

  /** Abort the current stream without scheduling anything. */
  private teardownStream() {
    this.streamGeneration += 1;
    this.stopLivenessWatch();
    const controller = this.controller;
    this.controller = null;
    this.connecting = false;
    if (controller) {
      try {
        controller.abort();
      } catch (_) {
        // Best-effort abort on teardown.
      }
    }
  }

  /**
   * Coalesce subscription-set changes, then reconnect with the full set. A
   * deliberate resync resets the backoff: it is not a failure.
   */
  private scheduleResync(delayMs: number) {
    if (this.disposed) return;
    if (this.resyncTimer) clearTimeout(this.resyncTimer);
    this.resyncTimer = setTimeout(() => {
      this.resyncTimer = null;
      if (this.connectTimer) {
        clearTimeout(this.connectTimer);
        this.connectTimer = null;
      }
      this.retryAttempt = 0;
      this.teardownStream();
      if (this.registrations.size === 0) return;
      void this.connect();
    }, delayMs);
  }

  private scheduleReconnect() {
    if (this.disposed || this.connectTimer || this.resyncTimer) return;
    if (this.registrations.size === 0) return;
    this.setStatus("reconnecting");
    const exponential = Math.min(
      RETRY_MAX_MS,
      RETRY_BASE_MS * 2 ** Math.min(this.retryAttempt, 10),
    );
    this.retryAttempt += 1;
    const jitter = exponential * RETRY_JITTER_RATIO * (Math.random() * 2 - 1);
    const delay = Math.max(0, Math.round(exponential + jitter));
    this.connectTimer = setTimeout(() => {
      this.connectTimer = null;
      this.teardownStream();
      if (this.registrations.size === 0) return;
      void this.connect();
    }, delay);
  }

  private requestBody(): string {
    return sessionEventsRequestBody(
      Array.from(this.registrations.values()).map((registration) => ({
        subscription_id: registration.subscriptionId,
        selector: registration.selector,
        after_cursor: registration.cursor,
      })),
    );
  }

  private async connect() {
    if (this.disposed || this.connecting || this.registrations.size === 0) return;
    this.connecting = true;
    this.setStatus(this.retryAttempt === 0 ? "connecting" : "reconnecting");
    const generation = this.streamGeneration;
    const controller = new AbortController();
    this.controller = controller;
    // Every subscription on this connection is admitted afresh: forget the
    // generations the previous connection stamped.
    for (const registration of this.registrations.values()) {
      registration.generation = null;
    }
    let response: StreamResponse;
    try {
      const { url, headers } = this.request();
      response = await this.openStream({
        url,
        headers: headers(),
        body: this.requestBody(),
        signal: controller.signal,
      });
    } catch (_) {
      if (this.streamGeneration !== generation) return;
      this.connecting = false;
      this.controller = null;
      this.scheduleReconnect();
      return;
    }
    if (this.streamGeneration !== generation) return;
    if (response.status !== 200 || !response.body) {
      this.connecting = false;
      this.controller = null;
      this.scheduleReconnect();
      return;
    }
    this.connecting = false;
    this.retryAttempt = 0;
    this.lastBytesAt = Date.now();
    this.setStatus("open");
    this.startLivenessWatch(controller);
    let endedCleanly = false;
    try {
      endedCleanly = await this.readStream(response.body, generation);
    } catch (_) {
      // Network error mid-stream; handled like a close below.
    }
    if (this.streamGeneration !== generation) return;
    // The server ended the stream (lifetime expiry, every subscription
    // finished, or a transport drop): resume every selector from its cursor.
    this.controller = null;
    this.stopLivenessWatch();
    if (endedCleanly) {
      this.scheduleResync(0);
    } else {
      this.scheduleReconnect();
    }
  }

  private startLivenessWatch(controller: AbortController) {
    this.stopLivenessWatch();
    this.livenessTimer = setInterval(() => {
      if (this.controller !== controller) return;
      if (Date.now() - this.lastBytesAt > LIVENESS_DEADLINE_MS) {
        // Nothing arrived for three keep-alive intervals: treat the
        // transport as dead and reconnect with each cursor intact.
        this.stopLivenessWatch();
        try {
          controller.abort();
        } catch (_) {
          // The read loop observes the abort.
        }
      }
    }, LIVENESS_CHECK_INTERVAL_MS);
  }

  /**
   * Read SSE events off the body until it ends. Returns `true` when the
   * server ended the stream after a `reconnect_hint` (a clean rotation),
   * `false` on any other end. Any bytes — comments included — prove liveness.
   */
  private async readStream(
    body: ReadableStream<Uint8Array>,
    generation: number,
  ): Promise<boolean> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffered = "";
    let dataLines: string[] = [];
    let rotated = false;
    const dispatch = () => {
      if (dataLines.length === 0) return;
      const frame = parseServerFrame(dataLines.join("\n"));
      dataLines = [];
      if (!frame) return;
      if (frame.type === "reconnect_hint") rotated = true;
      this.handleFrame(frame);
    };
    for (;;) {
      const { value, done } = await reader.read();
      if (this.streamGeneration !== generation) {
        try {
          reader.cancel();
        } catch (_) {
          // Superseded stream; nothing to release.
        }
        return false;
      }
      if (done) break;
      this.lastBytesAt = Date.now();
      buffered += decoder.decode(value, { stream: true });
      let newline = buffered.indexOf("\n");
      while (newline !== -1) {
        let line = buffered.slice(0, newline);
        buffered = buffered.slice(newline + 1);
        if (line.endsWith("\r")) line = line.slice(0, -1);
        if (line === "") {
          dispatch();
        } else if (line.startsWith("data:")) {
          dataLines.push(line.slice(5).replace(/^ /, ""));
        }
        // `event:`/`id:`/`retry:` fields and `:` comments carry nothing the
        // frame body does not already say; comments are liveness only.
        newline = buffered.indexOf("\n");
      }
    }
    dispatch();
    return rotated;
  }

  private handleFrame(frame: SessionServerFrame) {
    switch (frame.type) {
      case "subscribed": {
        const registration = this.registrations.get(frame.subscription_id ?? "");
        if (!registration) return;
        registration.generation = frame.generation ?? null;
        if (typeof frame.cursor === "string") {
          registration.cursor = frame.cursor;
        }
        return;
      }
      case "event": {
        const registration = this.registrations.get(frame.subscription_id ?? "");
        if (!registration) return;
        // Frames from a superseded generation must never deliver.
        if (
          registration.generation !== null &&
          typeof frame.generation === "number" &&
          frame.generation < registration.generation
        ) {
          return;
        }
        if (typeof frame.cursor === "string") {
          registration.cursor = frame.cursor;
        }
        if (frame.event && typeof frame.event === "object") {
          registration.handlers.onEvent({
            cursor: typeof frame.cursor === "string" ? frame.cursor : null,
            body: frame.event,
          });
        }
        return;
      }
      case "subscription_error": {
        const registration = this.registrations.get(frame.subscription_id ?? "");
        if (!registration) return;
        if (typeof frame.last_cursor === "string") {
          registration.cursor = frame.last_cursor;
        }
        const retryable = frame.retryable !== false;
        registration.handlers.onError?.({
          error: String(frame.error ?? "unavailable"),
          kind: String(frame.kind ?? "service_unavailable"),
          retryable,
          lastCursor: typeof frame.last_cursor === "string" ? frame.last_cursor : null,
        });
        if (!retryable) {
          // The server says this selector cannot be admitted (revoked or
          // foreign): stop resubscribing it. The owner decides whether to
          // rebase and register a fresh subscription.
          this.registrations.delete(registration.subscriptionId);
          return;
        }
        // Retryable: the subscription set is fixed per connection, so
        // resubscribing means reconnecting — after a short delay so a
        // persistently failing selector cannot spin.
        this.scheduleResync(this.subscriptionRetryDelayMs);
        return;
      }
      case "reconnect_hint":
        // Normal lifetime expiry: the read loop ends and reconnects with
        // every selector's own cursor.
        return;
      default:
        // Unknown vocabulary from a newer server: ignore.
        return;
    }
  }
}

let sharedClient: SessionEventClient | null = null;

export function sessionEventClient(): SessionEventClient {
  if (!sharedClient) {
    sharedClient = new SessionEventClient();
  }
  return sharedClient;
}

export function resetSessionEventClientForTests(): void {
  sharedClient?.dispose();
  sharedClient = null;
}
