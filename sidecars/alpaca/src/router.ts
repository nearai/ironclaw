/**
 * Request routing for the Alpaca sidecar.
 *
 * Split from the socket server so the whole contract — auth, envelope
 * validation, dispatch, error mapping — is testable without binding a socket
 * or loading the Ledger dependency tree.
 */

import {
  type AlpacaErr,
  type ErrorCode,
  TOKEN_HEADER,
  WIRE_VERSION,
  err,
  isErr,
  ok,
  parseEnvelope,
  statusFor,
  tokensMatch,
} from "./protocol.ts";

/** One chain api method the sidecar exposes. */
export type ApiMethod =
  | "craftTransaction"
  | "estimateFees"
  | "combine"
  | "broadcast"
  | "getBalance"
  | "lastBlock"
  | "listOperations"
  | "validateIntent";

const METHODS: ReadonlySet<string> = new Set<ApiMethod>([
  "craftTransaction",
  "estimateFees",
  "combine",
  "broadcast",
  "getBalance",
  "lastBlock",
  "listOperations",
  "validateIntent",
]);

export type RouterOptions = {
  /** Per-boot shared token, supplied by the Rust parent on stdin. */
  token: string;
  /** Chain apis by Ledger currency id. */
  apis: Map<string, Record<string, unknown>>;
  /** Reported by `/version` for operator diagnosis. */
  buildVersion: string;
};

export type HandledResponse = {
  status: number;
  body: string;
};

function fail(code: ErrorCode, message: string): HandledResponse {
  return respond(err(code, message));
}

function respond(payload: AlpacaErr | { version: number; ok: true; result: unknown }): HandledResponse {
  const status = isErr(payload) ? statusFor(payload.code) : 200;
  return { status, body: JSON.stringify(payload) };
}

/**
 * Handle one request.
 *
 * `/healthz` and `/version` are the only unauthenticated paths, and they
 * deliberately reveal nothing beyond liveness and a build string — the socket
 * already lives in a `0700` directory, but a health probe that leaked config
 * would still be a needless disclosure.
 */
export async function handle(
  options: RouterOptions,
  method: string,
  path: string,
  headers: Record<string, string | string[] | undefined>,
  rawBody: string,
): Promise<HandledResponse> {
  if (path === "/healthz") {
    return { status: 200, body: JSON.stringify({ version: WIRE_VERSION, ok: true, result: "ok" }) };
  }
  if (path === "/version") {
    return {
      status: 200,
      body: JSON.stringify({
        version: WIRE_VERSION,
        ok: true,
        result: { wire: WIRE_VERSION, build: options.buildVersion },
      }),
    };
  }

  // Everything else requires the token — checked BEFORE the path is validated,
  // so an unauthenticated caller cannot enumerate which methods exist.
  const presented = headers[TOKEN_HEADER];
  const token = Array.isArray(presented) ? presented[0] : presented;
  if (!tokensMatch(token, options.token)) {
    return fail("unauthorized", "missing or invalid sidecar token");
  }

  if (method !== "POST") {
    return fail("bad_request", "method calls must be POST");
  }

  const name = path.startsWith("/v1/") ? path.slice("/v1/".length) : "";
  if (!METHODS.has(name)) {
    return fail("bad_request", "unknown method");
  }

  let decoded: unknown;
  try {
    decoded = JSON.parse(rawBody);
  } catch {
    return fail("bad_request", "request body is not valid JSON");
  }

  const envelope = parseEnvelope<unknown>(decoded);
  if (isErr(envelope)) {
    return respond(envelope);
  }

  const api = options.apis.get(envelope.currencyId);
  if (!api) {
    return fail("unsupported_chain", "no chain api for that currency id");
  }
  const fn = api[name];
  if (typeof fn !== "function") {
    return fail("bad_request", "method not available for that chain");
  }

  try {
    const result = await (fn as (params: unknown) => Promise<unknown>).call(api, envelope.params);
    return respond(ok(result));
  } catch (cause) {
    // The chain RPC and the Ledger library are both upstream of us. Their
    // messages can carry endpoint detail, so the category is what crosses the
    // wire and the detail stays in the sidecar's own log.
    const message = cause instanceof Error ? cause.message : String(cause);
    console.error(`[alpaca] ${name} failed: ${message}`);
    return fail("upstream", `${name} failed`);
  }
}
