/**
 * The Alpaca sidecar process (attested-signing §E2).
 *
 * Node stdlib `http` bound to a Unix domain socket — no server framework, and
 * no inbound port on any external interface. The per-boot token arrives on
 * stdin from the Rust parent so it never appears in `ps`, an env dump, or a
 * config file.
 */

import { createServer } from "node:http";
import { chmodSync, mkdirSync, rmSync } from "node:fs";
import { dirname } from "node:path";

import { createChainApis, type SupportedChain } from "./bootstrap.ts";
import { readTokenAndWatchParent } from "./parent-link.ts";
import { handle, type RouterOptions } from "./router.ts";

const BUILD_VERSION = "0.0.0";

/** Request bodies are small JSON envelopes; anything larger is refused. */
const MAX_BODY_BYTES = 256 * 1024;

/**
 * Unix domain socket paths are bounded by `sun_path`: 104 bytes on macOS/BSD,
 * 108 on Linux. Exceeding it surfaces as a bare `EINVAL` from `listen`, which
 * is genuinely baffling in a supervised child. Check it up front and say what
 * is actually wrong.
 */
const SUN_PATH_MAX = 104;

function chainsFromEnv(): SupportedChain[] {
  const raw = process.env.ALPACA_CHAINS;
  if (!raw) {
    throw new Error("ALPACA_CHAINS is required (JSON array of {currencyId, chainId, rpcUri})");
  }
  const parsed: unknown = JSON.parse(raw);
  if (!Array.isArray(parsed) || parsed.length === 0) {
    throw new Error("ALPACA_CHAINS must be a non-empty JSON array");
  }
  return parsed as SupportedChain[];
}

async function main(): Promise<void> {
  const socketPath = process.env.ALPACA_SOCKET_PATH;
  if (!socketPath) {
    throw new Error("ALPACA_SOCKET_PATH is required");
  }
  if (Buffer.byteLength(socketPath) > SUN_PATH_MAX) {
    throw new Error(
      `ALPACA_SOCKET_PATH is ${Buffer.byteLength(socketPath)} bytes; the OS limit for a unix ` +
        `socket path is ${SUN_PATH_MAX}. Use a shorter directory (e.g. under /tmp).`,
    );
  }
  // Declared before the token read: the same stdin pipe that carries the token
  // is the parent-liveness link, and its EOF must be able to shut us down from
  // the moment it can fire.
  let server: ReturnType<typeof createServer> | undefined;
  let shuttingDown = false;
  const shutdown = (why: string) => {
    if (shuttingDown) {
      return;
    }
    shuttingDown = true;
    console.error(`[alpaca] shutting down: ${why}`);
    const done = () => {
      rmSync(socketPath, { force: true });
      process.exit(0);
    };
    if (server) {
      server.close(done);
    } else {
      done();
    }
  };

  const token = await readTokenAndWatchParent(process.stdin, () =>
    shutdown("the parent process closed the stdin link"),
  );
  const chains = chainsFromEnv();

  const options: RouterOptions = {
    token,
    apis: createChainApis(chains) as Map<string, Record<string, unknown>>,
    buildVersion: BUILD_VERSION,
  };

  server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    let total = 0;
    let aborted = false;

    req.on("data", (chunk: Buffer) => {
      total += chunk.length;
      if (total > MAX_BODY_BYTES) {
        aborted = true;
        res.writeHead(413).end();
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });

    req.on("end", () => {
      if (aborted) {
        return;
      }
      void handle(
        options,
        req.method ?? "GET",
        req.url ?? "/",
        req.headers,
        Buffer.concat(chunks).toString("utf8"),
      )
        .then(({ status, body }) => {
          res.writeHead(status, { "content-type": "application/json" }).end(body);
        })
        .catch((cause: unknown) => {
          console.error(`[alpaca] handler panic: ${String(cause)}`);
          res.writeHead(500, { "content-type": "application/json" }).end(
            JSON.stringify({ version: 1, ok: false, code: "internal", message: "handler failed" }),
          );
        });
    });
  });

  // A stale socket from an unclean exit would otherwise block the bind.
  rmSync(socketPath, { force: true });
  mkdirSync(dirname(socketPath), { recursive: true, mode: 0o700 });

  server.listen(socketPath, () => {
    // Owner-only: the socket is the entire access boundary alongside the token.
    chmodSync(socketPath, 0o600);
    console.error(`[alpaca] listening on ${socketPath}`);
  });

  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));
}

main().catch((cause: unknown) => {
  console.error(`[alpaca] fatal: ${String(cause)}`);
  process.exit(1);
});
