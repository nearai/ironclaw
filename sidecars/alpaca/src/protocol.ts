/**
 * The sidecar wire contract (attested-signing §E2).
 *
 * Versioned JSON envelopes over HTTP/1.1 on a Unix domain socket. The Rust
 * caller parses these with `deny_unknown_fields`, so the shapes here are the
 * contract — a golden-fixture round-trip test on both sides keeps them from
 * drifting silently.
 */

/** Bumped only for a breaking envelope change; Rust rejects anything else. */
export const WIRE_VERSION = 1;

/** Header carrying the per-boot shared token. */
export const TOKEN_HEADER = "x-alpaca-token";

export type AlpacaRequest<T> = {
  /** Must equal [`WIRE_VERSION`]. */
  version: number;
  /** Ledger currency id selecting the chain api. */
  currencyId: string;
  /** Method-specific payload. */
  params: T;
};

export type AlpacaOk<T> = {
  version: number;
  ok: true;
  result: T;
};

export type AlpacaErr = {
  version: number;
  ok: false;
  /** Stable machine-readable category (see [`ErrorCode`]). */
  code: ErrorCode;
  /** Sanitized description. Never carries RPC internals verbatim. */
  message: string;
};

/**
 * Error categories.
 *
 * Deliberately coarse: the Rust caller maps these onto its own fail-closed
 * taxonomy, and a sidecar is untrusted input at that boundary, so a richer
 * error surface would only invite the backend to trust it more than it should.
 */
export type ErrorCode =
  | "unauthorized"
  | "bad_request"
  | "unsupported_chain"
  | "upstream" // the chain RPC failed or timed out
  | "internal";

export function ok<T>(result: T): AlpacaOk<T> {
  return { version: WIRE_VERSION, ok: true, result };
}

export function err(code: ErrorCode, message: string): AlpacaErr {
  return { version: WIRE_VERSION, ok: false, code, message };
}

/** HTTP status for an error category. */
export function statusFor(code: ErrorCode): number {
  switch (code) {
    case "unauthorized":
      return 401;
    case "bad_request":
      return 400;
    case "unsupported_chain":
      return 404;
    case "upstream":
      return 502;
    case "internal":
      return 500;
  }
}

/**
 * Validate a decoded request envelope.
 *
 * Returns the typed request or an error envelope. Strict about the version:
 * an unrecognized version is refused rather than best-effort parsed, so a
 * version skew fails loudly at the first call instead of subtly later.
 */
export function parseEnvelope<T>(body: unknown): AlpacaRequest<T> | AlpacaErr {
  if (typeof body !== "object" || body === null) {
    return err("bad_request", "request body must be a JSON object");
  }
  const envelope = body as Partial<AlpacaRequest<T>>;
  if (envelope.version !== WIRE_VERSION) {
    return err("bad_request", `unsupported wire version: ${String(envelope.version)}`);
  }
  if (typeof envelope.currencyId !== "string" || envelope.currencyId.length === 0) {
    return err("bad_request", "currencyId is required");
  }
  if (envelope.params === undefined || envelope.params === null) {
    return err("bad_request", "params is required");
  }
  return {
    version: envelope.version,
    currencyId: envelope.currencyId,
    params: envelope.params as T,
  };
}

export function isErr(value: unknown): value is AlpacaErr {
  return typeof value === "object" && value !== null && (value as AlpacaErr).ok === false;
}

/**
 * Constant-time-ish token comparison.
 *
 * The token is a per-boot shared secret on a `0700` socket, so this is defense
 * in depth rather than the boundary — but a length-independent compare costs
 * nothing and removes an easy timing signal.
 */
export function tokensMatch(presented: string | undefined, expected: string): boolean {
  if (typeof presented !== "string" || presented.length !== expected.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < expected.length; i += 1) {
    diff |= presented.charCodeAt(i) ^ expected.charCodeAt(i);
  }
  return diff === 0;
}
