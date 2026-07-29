// Shared contracts for the DEMO-mode network interceptor.
//
// DEMO mode (build-time flag `VITE_DEMO_MODE=1`) replaces every network
// surface the SPA touches — `fetch`, `EventSource`, `WebSocket` — with an
// in-memory fixture router so the whole workspace renders and navigates
// without a backend. See `src/demo/install.ts` for the entry point.

export type DemoRequest = {
  method: string;
  /** URL pathname, e.g. "/api/webchat/v2/threads". */
  path: string;
  /** Full parsed URL (same-origin) for query-param access. */
  url: URL;
  /** Parsed JSON request body, or null when absent/non-JSON. */
  body: Record<string, unknown> | null;
};

export type DemoResponse = {
  status?: number;
  /** JSON payload (serialized with application/json). */
  json?: unknown;
  /** Raw text payload; `contentType` defaults to text/plain. */
  text?: string;
  contentType?: string;
};

export type DemoRouteHandler = (
  req: DemoRequest,
  match: RegExpExecArray
) => DemoResponse | undefined;

export type DemoRoute = {
  method: "GET" | "POST" | "DELETE" | "PUT" | "PATCH";
  /** Matched against `DemoRequest.path` (pathname only, no query). */
  pattern: RegExp;
  handle: DemoRouteHandler;
};
