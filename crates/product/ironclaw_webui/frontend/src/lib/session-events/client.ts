// The app-root session event client: one physical WebSocket per
// authenticated page, multiplexing independent typed logical subscriptions.
//
// Ownership rules (2026-08-13 session-transport design §7.3/§7.5):
// - Commands never travel here; the socket is read-only event transport.
// - Each logical subscription keeps its OWN resume cursor; reconnect
//   resubscribes every selector from its own last delivered cursor. There is
//   no session-wide cursor.
// - The socket stays connected while the page is hidden — background
//   delivery is the point of the session transport (the SSE path disconnects
//   hidden tabs; this one must not).
// - A `subscription_error` fails only that subscription: the owner is told
//   to rebase and may resubscribe; other subscriptions never reset.
// - Stale generations are ignored: after a replacement subscribe, frames
//   stamped with an older generation for the same subscription id drop.
// - When ticket minting or the upgrade fails repeatedly, the client degrades
//   for the rest of the page's life and consumers fall back to the
//   compatibility SSE transport (rollout §16).
//
// This module is dynamically imported by its hooks so the ~4 KB it costs
// stays out of the eager /chat closure (bundle budget).

import { mintSessionSocketTicket } from "../api";
import {
  type SessionSelector,
  type SessionServerFrame,
  parseServerFrame,
  pingFrame,
  subscribeFrame,
  unsubscribeFrame,
} from "./protocol";

const RETRY_BASE_MS = 1_000;
const RETRY_MAX_MS = 30_000;
const RETRY_JITTER_RATIO = 0.2;
const PING_INTERVAL_MS = 15_000;
const LIVENESS_DEADLINE_MS = 45_000;
// Consecutive mint/upgrade failures with zero frames received before the
// client degrades to the SSE fallback for the rest of the page's life.
const MAX_CONSECUTIVE_CONNECT_FAILURES = 3;
// Delay before resubscribing a selector the server failed retryably, so a
// persistent condition cannot become a subscribe/error spin loop.
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
  // Terminal for this subscription attempt. The client keeps the
  // subscription registered and resubscribes (from lastCursor when the
  // server supplied one) on the next connect cycle; the owner may rebase
  // local state when `retryable` demands it.
  onError?: (error: SessionSubscriptionError) => void;
  onStatus?: (status: SessionTransportStatus) => void;
};

export type SessionTransportStatus =
  | "connecting"
  | "open"
  | "reconnecting"
  | "degraded";

type Registration = {
  subscriptionId: string;
  selector: SessionSelector;
  cursor: string | null;
  generation: number | null;
  handlers: SessionSubscriptionHandlers;
};

type MintResponse = { ticket?: string; socket_path?: string };

let nextSubscriptionSuffix = 0;

export class SessionEventClient {
  private registrations = new Map<string, Registration>();
  private socket: WebSocket | null = null;
  private status: SessionTransportStatus = "reconnecting";
  private retryAttempt = 0;
  private consecutiveConnectFailures = 0;
  private everReceivedFrame = false;
  private degraded = false;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private lastFrameAt = 0;
  private connecting = false;
  private disposed = false;

  constructor(
    private readonly openSocket: (url: string) => WebSocket = (url) =>
      new WebSocket(url),
    private readonly mintTicket: () => Promise<MintResponse> = () =>
      mintSessionSocketTicket(),
    private readonly subscriptionRetryDelayMs: number = SUBSCRIPTION_RETRY_DELAY_MS,
  ) {
    if (typeof window !== "undefined") {
      window.addEventListener("online", this.handleOnline);
    }
  }

  isDegraded(): boolean {
    return this.degraded;
  }

  currentStatus(): SessionTransportStatus {
    return this.degraded ? "degraded" : this.status;
  }

  subscribe(
    selector: SessionSelector,
    handlers: SessionSubscriptionHandlers,
    options: { fromCursor?: string | null; idPrefix?: string } = {},
  ): { unsubscribe: () => void } {
    nextSubscriptionSuffix += 1;
    const subscriptionId = `${options.idPrefix ?? "sub"}-${nextSubscriptionSuffix}`;
    const registration: Registration = {
      subscriptionId,
      selector,
      cursor: options.fromCursor ?? null,
      generation: null,
      handlers,
    };
    this.registrations.set(subscriptionId, registration);
    handlers.onStatus?.(this.currentStatus());
    if (this.socket && this.socket.readyState === WebSocket.OPEN) {
      this.socket.send(
        subscribeFrame(subscriptionId, selector, registration.cursor),
      );
    } else {
      this.ensureConnected();
    }
    return {
      unsubscribe: () => {
        this.registrations.delete(subscriptionId);
        if (this.socket && this.socket.readyState === WebSocket.OPEN) {
          this.socket.send(unsubscribeFrame(subscriptionId));
        }
        if (this.registrations.size === 0) {
          this.teardownSocket();
        }
      },
    };
  }

  dispose(): void {
    this.disposed = true;
    if (typeof window !== "undefined") {
      window.removeEventListener("online", this.handleOnline);
    }
    this.teardownSocket();
    this.registrations.clear();
  }

  private handleOnline = () => {
    if (!this.degraded && this.registrations.size > 0 && !this.socket) {
      this.retryAttempt = 0;
      this.ensureConnected();
    }
  };

  private setStatus(status: SessionTransportStatus) {
    this.status = status;
    const effective = this.currentStatus();
    for (const registration of this.registrations.values()) {
      registration.handlers.onStatus?.(effective);
    }
  }

  private markDegraded() {
    if (this.degraded) return;
    this.degraded = true;
    this.teardownSocket();
    this.setStatus("degraded");
    // Page-lifetime decision, so leave a trace for support consoles: a
    // browser that never gets a socket is otherwise indistinguishable from
    // one that was never offered it. Never the ticket value.
    console.warn(
      "IronClaw session socket degraded to compatibility SSE after repeated connect failures",
    );
  }

  private teardownSocket() {
    if (this.connectTimer) {
      clearTimeout(this.connectTimer);
      this.connectTimer = null;
    }
    if (this.pingTimer) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      try {
        socket.close();
      } catch (_) {
        // Best-effort close on teardown.
      }
    }
  }

  private ensureConnected() {
    if (
      this.disposed ||
      this.degraded ||
      this.connecting ||
      this.connectTimer ||
      (this.socket && this.socket.readyState <= WebSocket.OPEN) ||
      this.registrations.size === 0
    ) {
      return;
    }
    void this.connect();
  }

  private async connect() {
    this.connecting = true;
    this.setStatus(this.retryAttempt === 0 ? "connecting" : "reconnecting");
    let ticket: string;
    let socketPath: string;
    try {
      const response = await this.mintTicket();
      if (!response?.ticket) throw new Error("mint returned no ticket");
      ticket = response.ticket;
      socketPath = response.socket_path ?? "/api/webchat/v2/session/websocket";
    } catch (_) {
      this.connecting = false;
      this.recordConnectFailure();
      return;
    }
    if (this.disposed || this.degraded || this.registrations.size === 0) {
      // Torn down (e.g. sign-out) while the mint was in flight: never open a
      // socket with a ticket minted for a session the page has abandoned.
      // The unconsumed ticket expires server-side.
      this.connecting = false;
      return;
    }
    let socket: WebSocket;
    try {
      const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
      const url = new URL(socketPath, window.location.origin);
      url.protocol = scheme;
      url.searchParams.set("ticket", ticket);
      socket = this.openSocket(url.toString());
    } catch (_) {
      this.connecting = false;
      this.recordConnectFailure();
      return;
    }
    this.socket = socket;
    socket.onopen = () => {
      this.connecting = false;
      this.lastFrameAt = Date.now();
      for (const registration of this.registrations.values()) {
        socket.send(
          subscribeFrame(
            registration.subscriptionId,
            registration.selector,
            registration.cursor,
          ),
        );
      }
      this.setStatus("open");
      this.startPinging(socket);
    };
    socket.onmessage = (message) => {
      this.lastFrameAt = Date.now();
      const frame = parseServerFrame(message.data);
      if (!frame) return;
      // Any application frame proves the transport works; reset the
      // degradation counters.
      this.everReceivedFrame = true;
      this.consecutiveConnectFailures = 0;
      this.retryAttempt = 0;
      this.handleFrame(frame);
    };
    socket.onclose = () => {
      if (this.socket === socket) {
        this.socket = null;
        if (this.pingTimer) {
          clearInterval(this.pingTimer);
          this.pingTimer = null;
        }
        this.connecting = false;
        if (!this.everReceivedFrame) {
          this.recordConnectFailure();
        } else if (this.registrations.size > 0) {
          this.scheduleReconnect();
        }
      }
    };
    socket.onerror = () => {
      // onclose follows; the close handler owns retry accounting.
    };
  }

  private startPinging(socket: WebSocket) {
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = setInterval(() => {
      if (socket.readyState !== WebSocket.OPEN) return;
      if (Date.now() - this.lastFrameAt > LIVENESS_DEADLINE_MS) {
        // The server answered nothing for three ping intervals: treat the
        // transport as dead and reconnect with each cursor intact.
        try {
          socket.close();
        } catch (_) {
          // close failures fall through to onclose.
        }
        return;
      }
      try {
        socket.send(pingFrame());
      } catch (_) {
        // Send failure surfaces through onclose.
      }
    }, PING_INTERVAL_MS);
  }

  private recordConnectFailure() {
    this.consecutiveConnectFailures += 1;
    if (
      !this.everReceivedFrame &&
      this.consecutiveConnectFailures >= MAX_CONSECUTIVE_CONNECT_FAILURES
    ) {
      this.markDegraded();
      return;
    }
    this.scheduleReconnect();
  }

  private scheduleReconnect() {
    if (this.disposed || this.degraded || this.connectTimer) return;
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
      this.ensureConnected();
    }, delay);
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
          lastCursor:
            typeof frame.last_cursor === "string" ? frame.last_cursor : null,
        });
        if (!retryable) {
          // The server says this selector cannot be admitted (revoked or
          // foreign): stop resubscribing. The owner decides whether to
          // rebase and register a fresh subscription.
          this.registrations.delete(registration.subscriptionId);
          if (this.registrations.size === 0) this.teardownSocket();
          return;
        }
        // Retryable: re-admit after a short delay so a persistently failing
        // selector cannot spin subscribe/subscription_error at frame rate.
        const subscriptionId = registration.subscriptionId;
        setTimeout(() => {
          const current = this.registrations.get(subscriptionId);
          const socket = this.socket;
          if (!current || !socket || socket.readyState !== WebSocket.OPEN) {
            return;
          }
          socket.send(
            subscribeFrame(subscriptionId, current.selector, current.cursor),
          );
        }, this.subscriptionRetryDelayMs);
        return;
      }
      case "reconnect_hint": {
        // Normal lifetime expiry: mint a fresh ticket (re-evaluating the
        // bearer) and resume every selector from its own cursor.
        this.teardownSocket();
        this.retryAttempt = 0;
        this.ensureConnected();
        return;
      }
      case "pong":
      case "unsubscribed":
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
