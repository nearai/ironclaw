#!/usr/bin/env node
/**
 * Loopback HTTP sidecar for IdentyClaw host login / HOLA.
 * Bind 127.0.0.1 only — never publish this port through nginx.
 */
import http from "node:http";
import {
  apiRequest,
  createHola,
  ensureSession,
  FEDERATED_SESSION_NOTE,
  helperInfo,
  listSessions,
  normalizeApiUrl,
  verifyHola,
} from "./lib.mjs";

const HOST = process.env.IDENTYCLAW_HELPER_HOST || "127.0.0.1";
const PORT = Number(process.env.IDENTYCLAW_HELPER_PORT || 3921);

function readJson(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (c) => {
      size += c.length;
      if (size > 256 * 1024) {
        reject(new Error("body too large"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => {
      if (!chunks.length) {
        resolve({});
        return;
      }
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch (err) {
        reject(err);
      }
    });
    req.on("error", reject);
  });
}

function send(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json",
    "cache-control": "no-store",
  });
  res.end(payload);
}

async function handle(req, res) {
  const url = new URL(req.url || "/", `http://${HOST}:${PORT}`);
  const route = url.pathname.replace(/\/+$/, "") || "/";

  try {
    if (req.method === "GET" && (route === "/health" || route === "/v1/health")) {
      send(res, 200, { ok: true, ...helperInfo() });
      return;
    }
    if (req.method === "GET" && route === "/v1/sessions") {
      send(res, 200, listSessions());
      return;
    }
    if (req.method === "GET" && route === "/v1/info") {
      send(res, 200, helperInfo());
      return;
    }

    const body = ["POST", "PUT", "PATCH"].includes(req.method) ? await readJson(req) : {};

    if (req.method === "POST" && route === "/v1/ensure_session") {
      send(res, 200, await ensureSession(body));
      return;
    }
    if (req.method === "POST" && route === "/v1/request") {
      send(res, 200, await apiRequest(body));
      return;
    }
    if (req.method === "POST" && route === "/v1/create_hola") {
      send(res, 200, await createHola(body));
      return;
    }
    if (req.method === "POST" && route === "/v1/verify_hola") {
      send(res, 200, await verifyHola(body));
      return;
    }
    if (req.method === "GET" && route === "/v1/me") {
      const apiEndpoint = url.searchParams.get("apiEndpoint") || undefined;
      const homeBase = normalizeApiUrl(
        process.env.IDENTYCLAW_BASE_URL ||
          process.env.IDENTYCLAW_API_BASE_URL ||
          "https://api.identyclaw.com"
      );
      const target = normalizeApiUrl(apiEndpoint || homeBase);
      // Home-only surface (OpenClaw parity). Federated peers share login, not /api/me/identity.
      if (target !== homeBase) {
        send(res, 200, {
          ok: false,
          home_only: true,
          apiEndpoint: target,
          error:
            "me /api/me/identity is a home IdentyClaw route. Federated login does not imply this path exists on the peer.",
          note: FEDERATED_SESSION_NOTE,
          hint: "If ensure_session already returned ok=true for this base, login succeeded — stop. Use request only for product paths the user named.",
        });
        return;
      }
      send(
        res,
        200,
        await apiRequest({
          method: "GET",
          path: "/api/me/identity",
          apiEndpoint: homeBase,
          auth: true,
        })
      );
      return;
    }
    if (req.method === "GET" && route === "/v1/agents") {
      const limit = url.searchParams.get("limit") || "20";
      const cursor = url.searchParams.get("cursor");
      const q = new URLSearchParams({ limit });
      if (cursor) q.set("cursor", cursor);
      send(
        res,
        200,
        await apiRequest({
          method: "GET",
          path: `/api/agents?${q}`,
          auth: false,
        })
      );
      return;
    }

    send(res, 404, { ok: false, error: "not_found", route });
  } catch (err) {
    send(res, 500, {
      ok: false,
      error: err?.message || String(err),
    });
  }
}

const server = http.createServer((req, res) => {
  handle(req, res);
});

server.listen(PORT, HOST, () => {
  process.stderr.write(
    `ironclaw-identyclaw helper listening on http://${HOST}:${PORT} (loopback only)\n`
  );
});
