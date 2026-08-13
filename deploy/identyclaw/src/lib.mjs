/**
 * IdentyClaw host helper — JWT sessions + HOLA create/verify.
 * Private keys and JWTs never appear in tool return values intended for the model.
 */
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import nacl from "tweetnacl";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const require = createRequire(pathToFileURL(path.join(ROOT, "package.json")));
const { nearPrivateKeyToSigningSecretKey } = require(
  path.join(ROOT, "vendor", "hola-client", "lib", "near-key.js")
);

// Quiet @rodit/rodit-auth-be import-time logging before first require.
process.env.LOG_LEVEL = process.env.LOG_LEVEL || "error";
process.env.SUPPRESS_NO_CONFIG_WARNING = process.env.SUPPRESS_NO_CONFIG_WARNING || "true";
process.env.SUPPRESS_STRICTNESS_CHECK = process.env.SUPPRESS_STRICTNESS_CHECK || "true";

const ONE_MINUTE_MS = 60_000;
const DEFAULT_BASE = "https://api.identyclaw.com";

/** OpenClaw parity: tell the model federated login ≠ home IdentyClaw surface. */
export const FEDERATED_SESSION_NOTE =
  "Federated session ready. Peers share Rodit login only — they do not need the same " +
  "endpoints as api.identyclaw.com. Do not call home tools (me / /api/me/identity / HOLA / " +
  "agents) against this host. Login is complete when ok=true; stop unless the user named a " +
  "specific product path. Then use request with the same base. Keep Passport/HOLA/DID on home.";

/** @type {Map<string, { token: string, expiresAtMs: number, federated: boolean, tokenId?: string }>} */
const memorySessions = new Map();

export function normalizeApiUrl(raw) {
  const trimmed = String(raw || "").trim().replace(/\/+$/, "");
  if (!trimmed) return DEFAULT_BASE;
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  return `https://${trimmed}`;
}

export function hostKeyFromApiUrl(apiUrl) {
  try {
    const u = new URL(normalizeApiUrl(apiUrl));
    return u.host.replace(/[^a-zA-Z0-9._-]+/g, "_");
  } catch {
    return "default";
  }
}

export function parseNearCreds(credPath) {
  const creds = JSON.parse(fs.readFileSync(credPath, "utf8"));
  const accountId = creds.implicit_account_id || creds.account_id || "";
  const privateKey = creds.private_key || "";
  if (!accountId || !privateKey) {
    throw new Error("credentials missing account_id/implicit_account_id or private_key");
  }
  return { accountId, privateKey, credPath };
}

export function resolveCredentialsPath(explicit) {
  if (explicit) {
    if (!fs.existsSync(explicit)) throw new Error(`credentials not found: ${explicit}`);
    return explicit;
  }
  const fromEnv = process.env.NEAR_CREDENTIALS_FILE_PATH || process.env.IDENTYCLAW_CREDENTIALS;
  if (fromEnv && fs.existsSync(fromEnv)) return fromEnv;

  const dir =
    process.env.IDENTYCLAW_NEAR_CREDENTIALS_DIR ||
    process.env.IRONCLAW_NEAR_CREDENTIALS_DIR ||
    "";
  if (!dir) {
    throw new Error(
      "set NEAR_CREDENTIALS_FILE_PATH or IDENTYCLAW_NEAR_CREDENTIALS_DIR (gennearaccount JSON)"
    );
  }
  const active = path.join(dir, ".active");
  if (fs.existsSync(active)) {
    const name = fs.readFileSync(active, "utf8").trim();
    const candidate = path.isAbsolute(name) ? name : path.join(dir, name);
    if (fs.existsSync(candidate)) return candidate;
  }
  const files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .map((f) => path.join(dir, f))
    .filter((f) => fs.statSync(f).isFile());
  if (!files.length) throw new Error(`no *.json under ${dir}`);
  return files[0];
}

export function applyNearRoditEnv({ accountId, privateKey, credPath }) {
  process.env.RODIT_NEAR_CREDENTIALS_SOURCE = "file";
  process.env.NEAR_CREDENTIALS_FILE_PATH = credPath;
  process.env.IDENTYCLAW_ACCOUNT_ID = accountId;
  process.env.IDENTYCLAW_NEAR_PRIVATE_KEY = privateKey;
  process.env.NEAR_CONTRACT_ID =
    process.env.NEAR_CONTRACT_ID ||
    process.env.IDENTYCLAW_NEAR_CONTRACT_ID ||
    "genaaaa-identyclaw-com.near";
  process.env.LOG_LEVEL = process.env.LOG_LEVEL || "error";
  process.env.SUPPRESS_NO_CONFIG_WARNING = process.env.SUPPRESS_NO_CONFIG_WARNING || "true";
  process.env.SUPPRESS_STRICTNESS_CHECK = process.env.SUPPRESS_STRICTNESS_CHECK || "true";
}

function base64Url(bytes) {
  return Buffer.from(bytes).toString("base64url");
}

/** NEAR `ed25519:...` → 64-byte tweetnacl signing secret (same as openclaw-identyclaw-plugin). */
export function secretKeyFromNearPrivateKey(nearPrivateKey) {
  return nearPrivateKeyToSigningSecretKey(nearPrivateKey);
}

function sessionDir() {
  const dir =
    process.env.IDENTYCLAW_SESSION_DIR ||
    path.join(process.env.IRONCLAW_APP_DIR || process.cwd(), "data", "identyclaw", "sessions");
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  return dir;
}

function sessionPath(apiUrl) {
  return path.join(sessionDir(), `${hostKeyFromApiUrl(apiUrl)}.jwt`);
}

function metaPath(apiUrl) {
  return path.join(sessionDir(), `${hostKeyFromApiUrl(apiUrl)}.meta.json`);
}

function decodeJwtPayload(jwt) {
  const parts = String(jwt).split(".");
  if (parts.length < 2) return {};
  const padded = parts[1].replace(/-/g, "+").replace(/_/g, "/");
  const json = Buffer.from(padded, "base64").toString("utf8");
  try {
    return JSON.parse(json);
  } catch {
    return {};
  }
}

function jwtExpiryMs(jwt) {
  const payload = decodeJwtPayload(jwt);
  if (Number.isFinite(payload.exp)) return payload.exp * 1000;
  return Date.now() + 55 * ONE_MINUTE_MS;
}

function loadCachedSession(apiUrl) {
  const key = normalizeApiUrl(apiUrl);
  const mem = memorySessions.get(key);
  if (mem && mem.expiresAtMs - ONE_MINUTE_MS > Date.now()) return mem;

  const jwtFile = sessionPath(key);
  if (!fs.existsSync(jwtFile)) return null;
  const token = fs.readFileSync(jwtFile, "utf8").trim();
  if (!token) return null;
  const expiresAtMs = jwtExpiryMs(token);
  if (expiresAtMs - ONE_MINUTE_MS <= Date.now()) return null;
  let meta = {};
  try {
    meta = JSON.parse(fs.readFileSync(metaPath(key), "utf8"));
  } catch {
    /* ignore */
  }
  const entry = {
    token,
    expiresAtMs,
    federated: Boolean(meta.federated),
    tokenId: meta.tokenId,
  };
  memorySessions.set(key, entry);
  return entry;
}

function persistSession(apiUrl, token, { federated = false, tokenId } = {}) {
  const key = normalizeApiUrl(apiUrl);
  const expiresAtMs = jwtExpiryMs(token);
  const payload = decodeJwtPayload(token);
  const resolvedTokenId =
    tokenId ||
    payload.token_id ||
    payload.roditid ||
    payload.sub ||
    undefined;
  fs.writeFileSync(sessionPath(key), token, { mode: 0o600 });
  fs.writeFileSync(
    metaPath(key),
    JSON.stringify(
      {
        apiEndpoint: key,
        federated,
        tokenId: resolvedTokenId || null,
        jwt_length: token.length,
        expiresAtMs,
        updatedAt: new Date().toISOString(),
      },
      null,
      2
    ),
    { mode: 0o600 }
  );
  const entry = { token, expiresAtMs, federated, tokenId: resolvedTokenId };
  memorySessions.set(key, entry);
  return entry;
}

function tryLoadRoditAuthBe() {
  try {
    return require("@rodit/rodit-auth-be");
  } catch {
    return null;
  }
}

function loadHolaClient() {
  const vendor = path.join(ROOT, "vendor", "hola-client", "index.js");
  return require(vendor);
}

async function loginWire({ baseUrl, accountId, privateKey }) {
  const tsRes = await fetch(`${baseUrl}/api/login/timestamp`);
  if (!tsRes.ok) {
    throw new Error(`GET /api/login/timestamp failed (${tsRes.status})`);
  }
  const ts = await tsRes.json();
  if (!Number.isFinite(ts.timestamp) || !ts.timestamp_iso) {
    throw new Error("timestamp endpoint returned invalid payload");
  }
  const message = `${accountId}${ts.timestamp_iso}`;
  const secretKey = secretKeyFromNearPrivateKey(privateKey);
  const sig = nacl.sign.detached(new TextEncoder().encode(message), secretKey);
  const loginRes = await fetch(`${baseUrl}/api/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      accountid: accountId,
      timestamp: ts.timestamp,
      base64url_signature: base64Url(sig),
    }),
  });
  const login = await loginRes.json().catch(() => ({}));
  if (!loginRes.ok || !login.jwt_token) {
    throw new Error(`POST /api/login failed (${loginRes.status}): ${JSON.stringify(login)}`);
  }
  return {
    jwt_token: login.jwt_token,
    tokenId: login.token_id || login.roditid || null,
    via: "wire",
  };
}

async function loginRoditSdk({ baseUrl, homeBase, creds }) {
  const pkg = tryLoadRoditAuthBe();
  if (!pkg?.RoditClient) return null;
  applyNearRoditEnv(creds);
  const client = await pkg.RoditClient.create({ role: "client" });
  const opts = {};
  if (normalizeApiUrl(baseUrl) !== normalizeApiUrl(homeBase)) {
    opts.apiEndpoint = baseUrl;
  }
  const result = await client.login_server(opts);
  if (!result?.jwt_token) {
    throw new Error(result?.error || "RoditClient.login_server failed");
  }
  return {
    jwt_token: result.jwt_token,
    tokenId: result.token_id || result.roditid || null,
    via: "rodit-auth-be",
  };
}

export async function ensureSession({
  apiEndpoint,
  credentialsPath,
  preferSdk = true,
} = {}) {
  const homeBase = normalizeApiUrl(
    process.env.IDENTYCLAW_BASE_URL || process.env.IDENTYCLAW_API_BASE_URL || DEFAULT_BASE
  );
  const target = normalizeApiUrl(apiEndpoint || homeBase);
  const cached = loadCachedSession(target);
  if (cached) {
    const federated = cached.federated || target !== homeBase;
    return {
      ok: true,
      apiEndpoint: target,
      federated,
      tokenId: cached.tokenId || null,
      jwt_length: cached.token.length,
      expiresAtMs: cached.expiresAtMs,
      cached: true,
      via: "cache",
      ...(federated ? { note: FEDERATED_SESSION_NOTE } : {}),
    };
  }

  const credPath = resolveCredentialsPath(credentialsPath);
  const creds = parseNearCreds(credPath);
  applyNearRoditEnv(creds);

  let login = null;
  if (preferSdk) {
    try {
      login = await loginRoditSdk({ baseUrl: target, homeBase, creds });
    } catch (err) {
      // Fall through to wire login; SDK may be missing or RPC-unavailable.
      login = { error: err };
    }
  }
  if (!login?.jwt_token) {
    login = await loginWire({
      baseUrl: target,
      accountId: creds.accountId,
      privateKey: creds.privateKey,
    });
  }

  const federated = target !== homeBase;
  const entry = persistSession(target, login.jwt_token, {
    federated,
    tokenId: login.tokenId || undefined,
  });

  // Optionally resolve Passport id from /api/me/identity when login did not include it.
  let tokenId = entry.tokenId || null;
  if (!tokenId) {
    try {
      const me = await apiRequest({
        method: "GET",
        path: "/api/me/identity",
        apiEndpoint: target,
        auth: true,
        credentialsPath: credPath,
      });
      tokenId = me.body?.tokenId || me.body?.token_id || me.body?.roditid || null;
      if (tokenId) persistSession(target, entry.token, { federated, tokenId });
    } catch {
      /* identity lookup is best-effort */
    }
  }

  return {
    ok: true,
    apiEndpoint: target,
    federated,
    tokenId,
    jwt_length: entry.token.length,
    expiresAtMs: entry.expiresAtMs,
    cached: false,
    via: login.via || "wire",
    ...(federated ? { note: FEDERATED_SESSION_NOTE } : {}),
  };
}

export function listSessions() {
  const dir = sessionDir();
  const homeBase = normalizeApiUrl(
    process.env.IDENTYCLAW_BASE_URL || process.env.IDENTYCLAW_API_BASE_URL || DEFAULT_BASE
  );
  const sessions = [];
  for (const name of fs.readdirSync(dir)) {
    if (!name.endsWith(".meta.json")) continue;
    try {
      const meta = JSON.parse(fs.readFileSync(path.join(dir, name), "utf8"));
      const alive = Number(meta.expiresAtMs) - ONE_MINUTE_MS > Date.now();
      sessions.push({
        apiEndpoint: meta.apiEndpoint,
        federated: Boolean(meta.federated),
        tokenId: meta.tokenId || null,
        jwt_length: meta.jwt_length || null,
        expiresAtMs: meta.expiresAtMs || null,
        alive,
        isHome: normalizeApiUrl(meta.apiEndpoint) === homeBase,
      });
    } catch {
      /* skip corrupt */
    }
  }
  return {
    ok: true,
    homeBase,
    sessions,
    note: "JWTs are host-only and never returned to the model.",
  };
}

async function getJwt(apiEndpoint, credentialsPath) {
  const target = normalizeApiUrl(
    apiEndpoint ||
      process.env.IDENTYCLAW_BASE_URL ||
      process.env.IDENTYCLAW_API_BASE_URL ||
      DEFAULT_BASE
  );
  let cached = loadCachedSession(target);
  if (!cached) {
    await ensureSession({ apiEndpoint: target, credentialsPath });
    cached = loadCachedSession(target);
  }
  if (!cached?.token) throw new Error(`no JWT session for ${target}`);
  return { target, token: cached.token };
}

export async function apiRequest({
  method = "GET",
  path: reqPath,
  body,
  apiEndpoint,
  auth = true,
  credentialsPath,
  responseType = "json",
} = {}) {
  if (!reqPath || !String(reqPath).startsWith("/")) {
    throw new Error("path must start with /");
  }
  const { target, token } = auth
    ? await getJwt(apiEndpoint, credentialsPath)
    : {
        target: normalizeApiUrl(
          apiEndpoint ||
            process.env.IDENTYCLAW_BASE_URL ||
            process.env.IDENTYCLAW_API_BASE_URL ||
            DEFAULT_BASE
        ),
        token: null,
      };

  const headers = {};
  if (auth) headers.authorization = `Bearer ${token}`;
  let payload;
  if (body !== undefined && method !== "GET" && method !== "HEAD") {
    headers["content-type"] = "application/json";
    payload = typeof body === "string" ? body : JSON.stringify(body);
  }
  const res = await fetch(`${target}${reqPath}`, { method, headers, body: payload });
  const newToken = res.headers.get("new-token") || res.headers.get("New-Token");
  if (newToken && auth) {
    const prev = loadCachedSession(target);
    persistSession(target, newToken, {
      federated: prev?.federated,
      tokenId: prev?.tokenId,
    });
  }

  if (responseType === "text") {
    const text = await res.text();
    if (!res.ok) throw new Error(`${method} ${reqPath} failed (${res.status}): ${text.slice(0, 400)}`);
    return { ok: true, status: res.status, apiEndpoint: target, body: text };
  }

  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw new Error(
      `${method} ${reqPath} failed (${res.status}): ${JSON.stringify(json).slice(0, 400)}`
    );
  }
  return { ok: true, status: res.status, apiEndpoint: target, body: json };
}

export async function createHola({
  recipient = "MUNDO",
  apiEndpoint,
  credentialsPath,
  tokenId,
} = {}) {
  const session = await ensureSession({ apiEndpoint, credentialsPath });
  const { target, token } = await getJwt(apiEndpoint, credentialsPath);
  const credPath = resolveCredentialsPath(credentialsPath);
  const creds = parseNearCreds(credPath);
  let resolvedTokenId = tokenId || session.tokenId;
  if (!resolvedTokenId) {
    const me = await apiRequest({
      method: "GET",
      path: "/api/me/identity",
      apiEndpoint: target,
      auth: true,
      credentialsPath: credPath,
    });
    resolvedTokenId = me.body?.tokenId || me.body?.token_id || me.body?.roditid;
  }
  if (!resolvedTokenId) {
    throw new Error("could not resolve own Passport tokenId — run ensure_session / me first");
  }

  const { createHola: createHolaLine } = loadHolaClient();
  const signed = await createHolaLine({
    nearPrivateKey: creds.privateKey,
    jwt: token,
    tokenId: resolvedTokenId,
    baseUrl: target,
    recipient,
  });

  return {
    ok: true,
    apiEndpoint: target,
    hola: signed.hola,
    tokenId: signed.tokenId,
    recipient: signed.recipient,
    noncetsHex: signed.noncetsHex,
    timestamp: signed.timestamp,
    checksum: signed.checksum,
    note: "Send this HOLA on the same channel; never paste nearPrivateKey or JWTs.",
  };
}

export async function verifyHola({
  hola,
  expectedRecipient,
  apiEndpoint,
  auth = false,
  credentialsPath,
  maxAgeMs,
} = {}) {
  if (!hola || typeof hola !== "string") {
    throw new Error("hola must be a single string");
  }
  const target = normalizeApiUrl(
    apiEndpoint ||
      process.env.IDENTYCLAW_BASE_URL ||
      process.env.IDENTYCLAW_API_BASE_URL ||
      DEFAULT_BASE
  );
  const headers = { "content-type": "application/json" };
  if (auth) {
    const { token } = await getJwt(target, credentialsPath);
    headers.authorization = `Bearer ${token}`;
  }
  const body = { hola };
  if (expectedRecipient) body.expectedRecipient = expectedRecipient;
  if (Number.isFinite(maxAgeMs)) body.constraints = { maxAgeMs };

  const res = await fetch(`${target}/api/identity/verify`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  const json = await res.json().catch(() => ({}));
  if (!res.ok && res.status !== 200) {
    throw new Error(`verify failed (${res.status}): ${JSON.stringify(json).slice(0, 400)}`);
  }
  return { ok: true, status: res.status, apiEndpoint: target, ...json };
}

export function helperInfo() {
  const hasSdk = Boolean(tryLoadRoditAuthBe());
  return {
    ok: true,
    homeBase: normalizeApiUrl(
      process.env.IDENTYCLAW_BASE_URL || process.env.IDENTYCLAW_API_BASE_URL || DEFAULT_BASE
    ),
    sessionDir: sessionDir(),
    roditAuthBe: hasSdk,
    contract: "IronClaw host helper — equivalent to OpenClaw identyclaw-tools via host login",
  };
}
