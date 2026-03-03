import { randomUUID } from "node:crypto";
import { mkdir } from "node:fs/promises";
import path from "node:path";

import Fastify from "fastify";
import { Camoufox } from "camoufox-js";
import { z } from "zod";

const app = Fastify({ logger: true });

const port = Number(process.env.CAMOUFOX_TOOL_PORT || "8788");
const artifactDir = process.env.CAMOUFOX_ARTIFACT_DIR || "/data/artifacts/camoufox";
const defaultHeadless = (process.env.CAMOUFOX_HEADLESS || "true") === "true";
const defaultTimeoutMs = Number(process.env.CAMOUFOX_DEFAULT_TIMEOUT_MS || "25000");
const challengeKeywords = (process.env.BROWSER_CHALLENGE_KEYWORDS || "captcha,verify you are human,security check,mfa,one-time code,unusual traffic,access denied")
  .split(",")
  .map((value) => value.trim().toLowerCase())
  .filter(Boolean);

/** @type {Map<string, {browser: import('playwright').Browser, context: import('playwright').BrowserContext, page: import('playwright').Page}>} */
const sessions = new Map();

const actionSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("browser.goto"), url: z.string().url() }),
  z.object({ type: z.literal("browser.click"), selector: z.string().min(1) }),
  z.object({ type: z.literal("browser.fill"), selector: z.string().min(1), value: z.string() }),
  z.object({ type: z.literal("browser.press"), selector: z.string().min(1), key: z.string().min(1) }),
  z.object({ type: z.literal("browser.click_xy"), x: z.number(), y: z.number() }),
  z.object({ type: z.literal("browser.wait_for_selector"), selector: z.string().min(1), timeoutMs: z.number().int().positive().optional() }),
  z.object({ type: z.literal("browser.wait"), timeoutMs: z.number().int().positive() }),
  z.object({ type: z.literal("browser.screenshot"), label: z.string().min(1), fullPage: z.boolean().optional() })
]);

const newSessionSchema = z.object({
  headless: z.boolean().optional(),
  timeoutMs: z.number().int().positive().optional(),
  viewport: z.object({ width: z.number().int().positive(), height: z.number().int().positive() }).optional()
});

const sanitize = (input) => input.replace(/[^a-zA-Z0-9_-]/g, "-").toLowerCase();

const detectChallenge = async (page) => {
  const content = (await page.content()).toLowerCase();
  const title = (await page.title()).toLowerCase();

  for (const keyword of challengeKeywords) {
    if (content.includes(keyword) || title.includes(keyword)) {
      return { detected: true, reason: `keyword:${keyword}` };
    }
  }

  return { detected: false, reason: null };
};

const closeSession = async (sessionId) => {
  const session = sessions.get(sessionId);
  if (!session) {
    return false;
  }

  await session.page.close().catch(() => undefined);
  await session.context.close().catch(() => undefined);
  await session.browser.close().catch(() => undefined);
  sessions.delete(sessionId);
  return true;
};

app.get("/healthz", async () => ({
  status: "ok",
  service: "camoufox-tool",
  activeSessions: sessions.size,
  timestamp: new Date().toISOString()
}));

app.post("/session/new", async (request, reply) => {
  const parsed = newSessionSchema.safeParse(request.body ?? {});
  if (!parsed.success) {
    return reply.code(400).send({ error: parsed.error.flatten() });
  }

  const sessionId = randomUUID();
  const browser = /** @type {import('playwright').Browser} */ (await Camoufox({
    headless: parsed.data.headless ?? defaultHeadless
  }));

  const context = await browser.newContext({
    viewport: parsed.data.viewport ?? { width: 1366, height: 900 }
  });

  const page = await context.newPage();
  page.setDefaultTimeout(parsed.data.timeoutMs ?? defaultTimeoutMs);

  sessions.set(sessionId, { browser, context, page });

  return reply.send({
    sessionId,
    status: "ready"
  });
});

app.delete("/session/:sessionId", async (request, reply) => {
  const { sessionId } = /** @type {{sessionId: string}} */ (request.params);
  const closed = await closeSession(sessionId);
  return reply.send({
    sessionId,
    status: closed ? "closed" : "not_found"
  });
});

app.post("/session/:sessionId/action", async (request, reply) => {
  const { sessionId } = /** @type {{sessionId: string}} */ (request.params);
  const session = sessions.get(sessionId);
  if (!session) {
    return reply.code(404).send({ error: `Unknown sessionId: ${sessionId}` });
  }

  const parsed = actionSchema.safeParse(request.body ?? {});
  if (!parsed.success) {
    return reply.code(400).send({ error: parsed.error.flatten() });
  }

  const action = parsed.data;
  let artifactPath = null;

  switch (action.type) {
    case "browser.goto":
      await session.page.goto(action.url, { waitUntil: "domcontentloaded" });
      break;
    case "browser.click":
      await session.page.click(action.selector);
      break;
    case "browser.fill":
      await session.page.fill(action.selector, action.value);
      break;
    case "browser.press":
      await session.page.press(action.selector, action.key);
      break;
    case "browser.click_xy":
      await session.page.mouse.click(action.x, action.y);
      break;
    case "browser.wait_for_selector":
      await session.page.waitForSelector(action.selector, { timeout: action.timeoutMs ?? defaultTimeoutMs });
      break;
    case "browser.wait":
      await session.page.waitForTimeout(action.timeoutMs);
      break;
    case "browser.screenshot": {
      const dir = path.join(artifactDir, sessionId);
      await mkdir(dir, { recursive: true });
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      artifactPath = path.join(dir, `${sanitize(action.label)}-${stamp}.png`);
      await session.page.screenshot({
        path: artifactPath,
        fullPage: action.fullPage ?? true
      });
      break;
    }
    default:
      return reply.code(400).send({ error: "Unsupported action type" });
  }

  if (action.type !== "browser.wait") {
    const challenge = await detectChallenge(session.page);
    if (challenge.detected) {
      return reply.send({
        status: "human_required",
        reason: challenge.reason,
        sessionId,
        artifactPath
      });
    }
  }

  return reply.send({
    status: "ok",
    sessionId,
    artifactPath
  });
});

const shutdown = async () => {
  const ids = [...sessions.keys()];
  for (const sessionId of ids) {
    await closeSession(sessionId);
  }
};

process.on("SIGTERM", () => {
  void shutdown().finally(() => process.exit(0));
});

process.on("SIGINT", () => {
  void shutdown().finally(() => process.exit(0));
});

app.listen({ host: "0.0.0.0", port })
  .then(() => app.log.info({ port }, "camoufox-tool started"))
  .catch((error) => {
    app.log.error({ err: error }, "failed to start camoufox-tool");
    process.exit(1);
  });
