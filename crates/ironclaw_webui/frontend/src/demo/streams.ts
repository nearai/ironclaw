// Inert EventSource / WebSocket stand-ins for DEMO mode.
//
// The chat surface opens an SSE stream (with a WebSocket upgrade path) per
// thread. In DEMO mode there is no backend, so both constructors are
// replaced with silently-open fakes: they never error (which would surface
// reconnect/offline banners) and never emit events (the timeline fixtures
// are the source of truth). `emitDemoThreadEvent` lets fixture mutations
// push a synthetic event to any listener attached to a thread's stream.

type Listener = (event: MessageEvent) => void;

const openDemoStreams = new Set<DemoEventSource>();

export class DemoEventSource extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  readonly CONNECTING = DemoEventSource.CONNECTING;
  readonly OPEN = DemoEventSource.OPEN;
  readonly CLOSED = DemoEventSource.CLOSED;

  url: string;
  readyState: number = DemoEventSource.CONNECTING;
  withCredentials = false;
  onopen: ((event: Event) => void) | null = null;
  onmessage: Listener | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(url: string | URL) {
    super();
    this.url = String(url);
    openDemoStreams.add(this);
    // Open asynchronously like a real EventSource would.
    setTimeout(() => {
      if (this.readyState === DemoEventSource.CLOSED) return;
      this.readyState = DemoEventSource.OPEN;
      const event = new Event("open");
      this.onopen?.(event);
      this.dispatchEvent(event);
    }, 0);
  }

  close() {
    this.readyState = DemoEventSource.CLOSED;
    openDemoStreams.delete(this);
  }
}

/** Push a synthetic SSE frame to every open stream for `threadId`. */
export function emitDemoThreadEvent(threadId: string, type: string, data: unknown) {
  const payload = JSON.stringify(data);
  for (const stream of openDemoStreams) {
    if (!stream.url.includes(encodeURIComponent(threadId))) continue;
    const event = new MessageEvent(type, { data: payload });
    stream.dispatchEvent(event);
    if (type === "message") stream.onmessage?.(event);
  }
}

export class DemoWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readonly CONNECTING = DemoWebSocket.CONNECTING;
  readonly OPEN = DemoWebSocket.OPEN;
  readonly CLOSING = DemoWebSocket.CLOSING;
  readonly CLOSED = DemoWebSocket.CLOSED;

  url: string;
  readyState: number = DemoWebSocket.CONNECTING;
  binaryType = "blob";
  bufferedAmount = 0;
  protocol = "";
  extensions = "";
  onopen: ((event: Event) => void) | null = null;
  onmessage: Listener | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;

  constructor(url: string | URL) {
    super();
    this.url = String(url);
    setTimeout(() => {
      if (this.readyState !== DemoWebSocket.CONNECTING) return;
      this.readyState = DemoWebSocket.OPEN;
      const event = new Event("open");
      this.onopen?.(event);
      this.dispatchEvent(event);
    }, 0);
  }

  send(_data: unknown) {
    // Mutations ride the fetch fixtures; socket sends are no-ops.
  }

  close() {
    if (this.readyState === DemoWebSocket.CLOSED) return;
    this.readyState = DemoWebSocket.CLOSED;
    const event = new Event("close");
    this.onclose?.(event);
    this.dispatchEvent(event);
  }
}
