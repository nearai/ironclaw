// @ts-nocheck
// Demo-mode mock backend.
//
// Installed by main.tsx ONLY when the bundle is built with
// `VITE_IRONCLAW_DEMO=1` (the hosted workspace preview). It patches
// `window.fetch` for same-origin WebChat v2 / auth routes and replaces
// `EventSource` with an in-memory emitter, so the full authenticated
// workspace renders on a static host with no Rust backend. Any `?token=`
// value is accepted — the demo deploy is shared via `/?token=demo`.
//
// Production builds never set the flag; the dynamic import in main.tsx
// is dead-code-eliminated, so none of this ships in the embedded UI.

import {
  DEMO_AUTOMATIONS,
  DEMO_CANNED_REPLIES,
  DEMO_CONNECTABLE_CHANNELS,
  DEMO_EXTENSIONS,
  DEMO_FS_CONTENT,
  DEMO_FS_MOUNTS,
  DEMO_FS_TREE,
  DEMO_LLM_PROVIDERS,
  DEMO_LOGS,
  DEMO_OUTBOUND_PREFERENCES,
  DEMO_OUTBOUND_TARGETS,
  DEMO_REGISTRY,
  DEMO_SESSION,
  DEMO_SKILLS,
  DEMO_THREADS,
  DEMO_TIMELINES,
} from "./fixtures";

const V2 = "/api/webchat/v2";

// --- Mutable in-memory state (reset on reload) ---

const state = {
  threads: DEMO_THREADS.map((thread) => ({ ...thread })),
  timelines: Object.fromEntries(
    Object.entries(DEMO_TIMELINES).map(([id, messages]) => [
      id,
      messages.map((message) => ({ ...message })),
    ]),
  ),
  automations: JSON.parse(JSON.stringify(DEMO_AUTOMATIONS)),
  cannedReplyIndex: 0,
  sequence: 1000,
};

function threadRecord(threadId) {
  return state.threads.find((thread) => thread.thread_id === threadId) || null;
}

function timelineFor(threadId) {
  if (!state.timelines[threadId]) state.timelines[threadId] = [];
  return state.timelines[threadId];
}

// --- EventSource stub ---
//
// useSSE only needs: constructor(url), onopen/onerror/onmessage,
// addEventListener, close(), readyState, and the readyState constants.

const streamsByThread = new Map();

function threadIdFromEventsUrl(url) {
  const match = /\/threads\/([^/]+)\/events/.exec(url);
  return match ? decodeURIComponent(match[1]) : null;
}

class DemoEventSource extends EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  constructor(url) {
    super();
    this.url = String(url);
    this.readyState = DemoEventSource.CONNECTING;
    this.onopen = null;
    this.onerror = null;
    this.onmessage = null;
    this.threadId = threadIdFromEventsUrl(this.url);
    if (this.threadId) {
      if (!streamsByThread.has(this.threadId)) {
        streamsByThread.set(this.threadId, new Set());
      }
      streamsByThread.get(this.threadId).add(this);
    }
    setTimeout(() => {
      if (this.readyState === DemoEventSource.CLOSED) return;
      this.readyState = DemoEventSource.OPEN;
      this.onopen?.(new Event("open"));
    }, 30);
  }

  emitFrame(name, frame) {
    if (this.readyState === DemoEventSource.CLOSED) return;
    const event = new MessageEvent(name, { data: JSON.stringify(frame) });
    this.dispatchEvent(event);
    if (name === "message") this.onmessage?.(event);
  }

  close() {
    this.readyState = DemoEventSource.CLOSED;
    if (this.threadId) streamsByThread.get(this.threadId)?.delete(this);
  }
}

function emitToThread(threadId, name, frame) {
  for (const stream of streamsByThread.get(threadId) || []) {
    stream.emitFrame(name, frame);
  }
}

// --- Live chat: accept a send, then stream a canned reply ---

function acceptMessage(threadId, body) {
  const timeline = timelineFor(threadId);
  const runId = `run-demo-${state.sequence}`;
  const messageId = `msg-demo-${state.sequence}`;
  state.sequence += 1;
  const nowIso = new Date().toISOString();

  timeline.push({
    message_id: messageId,
    thread_id: threadId,
    sequence: state.sequence,
    kind: "user",
    status: "accepted",
    content: String(body?.content ?? ""),
    created_at: nowIso,
    updated_at: nowIso,
    turn_run_id: runId,
  });

  const thread = threadRecord(threadId);
  if (thread) {
    thread.updated_at = nowIso;
    if (!thread.title) {
      thread.title = String(body?.content ?? "").slice(0, 64) || "New conversation";
    }
  }

  const replyText =
    DEMO_CANNED_REPLIES[state.cannedReplyIndex % DEMO_CANNED_REPLIES.length];
  state.cannedReplyIndex += 1;

  setTimeout(() => {
    emitToThread(threadId, "accepted", {
      type: "accepted",
      ack: { run_id: runId, thread_id: threadId, status: "running" },
    });
  }, 120);

  setTimeout(() => {
    const generatedAt = new Date().toISOString();
    timeline.push({
      message_id: `msg-demo-${state.sequence}`,
      thread_id: threadId,
      sequence: (state.sequence += 1),
      kind: "assistant",
      status: "finalized",
      content: replyText,
      created_at: generatedAt,
      updated_at: generatedAt,
      turn_run_id: runId,
    });
    emitToThread(threadId, "final_reply", {
      type: "final_reply",
      reply: { turn_run_id: runId, text: replyText, generated_at: generatedAt },
    });
  }, 1100);

  return {
    outcome: "accepted",
    thread_id: threadId,
    run_id: runId,
    accepted_message_ref: { message_id: messageId },
  };
}

// --- fetch router ---

function json(body, init = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

function text(body, contentType = "text/plain; charset=utf-8") {
  return new Response(body, {
    status: 200,
    headers: { "Content-Type": contentType },
  });
}

function notFound() {
  return new Response(JSON.stringify({ error: "not_found", kind: "not_found" }), {
    status: 404,
    headers: { "Content-Type": "application/json" },
  });
}

async function requestBody(init, input) {
  try {
    if (init?.body) return JSON.parse(String(init.body));
    if (input instanceof Request) return await input.clone().json();
  } catch {
    return null;
  }
  return null;
}

function routeFs(url) {
  const params = url.searchParams;
  if (url.pathname === `${V2}/fs/mounts`) return json(DEMO_FS_MOUNTS);
  if (url.pathname === `${V2}/fs/list`) {
    const mount = params.get("mount") || "";
    const path = params.get("path") || "";
    const entries = DEMO_FS_TREE[mount]?.[path];
    if (!entries) return notFound();
    return json({ mount, path, entries });
  }
  if (url.pathname === `${V2}/fs/stat`) {
    const mount = params.get("mount") || "";
    const path = params.get("path") || "";
    const parent = path.split("/").slice(0, -1).join("/");
    const entry = DEMO_FS_TREE[mount]?.[parent]?.find((item) => item.path === path);
    if (!entry) return notFound();
    return json({
      stat: {
        path,
        kind: entry.kind,
        size_bytes: entry.size_bytes ?? 0,
        mime_type: path.endsWith(".md")
          ? "text/markdown"
          : path.endsWith(".csv")
            ? "text/csv"
            : "text/plain",
      },
    });
  }
  if (url.pathname === `${V2}/fs/content`) {
    const mount = params.get("mount") || "";
    const path = params.get("path") || "";
    const content = DEMO_FS_CONTENT[`${mount}/${path}`];
    if (content == null) return notFound();
    return text(content);
  }
  return null;
}

async function route(input, init) {
  const rawUrl =
    typeof input === "string" || input instanceof URL ? String(input) : input.url;
  const url = new URL(rawUrl, window.location.origin);
  if (url.origin !== window.location.origin) return null;
  const path = url.pathname;
  const isApi = path.startsWith(`${V2}/`) || path === V2;
  const isAuth = path === "/auth/providers" || path === "/auth/logout";
  const isReborn = path.startsWith("/api/reborn/");
  if (!isApi && !isAuth && !isReborn) return null;

  const method = (
    init?.method ||
    (input instanceof Request ? input.method : "GET")
  ).toUpperCase();

  // Auth surface.
  if (path === "/auth/providers") return json({ providers: [] });
  if (path === "/auth/logout") return json({ ok: true });

  // Session.
  if (path === `${V2}/session`) return json(DEMO_SESSION);

  // Threads.
  if (path === `${V2}/threads` && method === "GET") {
    if (url.searchParams.get("needs_approval") === "true") {
      return json({ threads: [], next_cursor: null });
    }
    const threads = [...state.threads].sort((a, b) =>
      String(b.updated_at || "").localeCompare(String(a.updated_at || "")),
    );
    return json({ threads, next_cursor: null });
  }
  if (path === `${V2}/threads` && method === "POST") {
    const body = await requestBody(init, input);
    const nowIso = new Date().toISOString();
    const thread = {
      thread_id: body?.requested_thread_id || `thread-demo-${state.sequence++}`,
      created_by_actor_id: DEMO_SESSION.user_id,
      created_at: nowIso,
      updated_at: nowIso,
      scope: {
        tenant_id: DEMO_SESSION.tenant_id,
        agent_id: "agent-demo",
        owner_user_id: DEMO_SESSION.user_id,
      },
    };
    state.threads.unshift(thread);
    return json({ thread });
  }

  const threadMatch = /^\/api\/webchat\/v2\/threads\/([^/]+)(\/.*)?$/.exec(path);
  if (threadMatch) {
    const threadId = decodeURIComponent(threadMatch[1]);
    const rest = threadMatch[2] || "";
    if (!rest && method === "DELETE") {
      state.threads = state.threads.filter((thread) => thread.thread_id !== threadId);
      delete state.timelines[threadId];
      return json({ ok: true });
    }
    if (rest === "/timeline") {
      const thread = threadRecord(threadId);
      if (!thread) return notFound();
      return json({
        thread,
        messages: timelineFor(threadId),
        summary_artifacts: [],
        next_cursor: null,
      });
    }
    if (rest === "/messages" && method === "POST") {
      const body = await requestBody(init, input);
      if (!threadRecord(threadId)) return notFound();
      return json(acceptMessage(threadId, body));
    }
    if (/^\/runs\/[^/]+\/cancel$/.test(rest)) return json({ ok: true });
    if (/^\/runs\/[^/]+\/artifact$/.test(rest)) return notFound();
    if (rest === "/files" || rest.startsWith("/files/")) {
      return json({ entries: [] });
    }
  }

  // Automations.
  if (path === `${V2}/automations` && method === "GET") {
    return json(state.automations);
  }
  const automationMatch = /^\/api\/webchat\/v2\/automations\/([^/]+)(\/(pause|resume))?$/.exec(
    path,
  );
  if (automationMatch && method === "POST") {
    const automationId = decodeURIComponent(automationMatch[1]);
    const action = automationMatch[3];
    const record = state.automations.automations.find(
      (automation) => automation.automation_id === automationId,
    );
    if (!record) return notFound();
    if (action === "pause") {
      record.state = "paused";
      record.is_active = false;
    } else if (action === "resume") {
      record.state = "active";
      record.is_active = true;
    } else {
      const body = await requestBody(init, input);
      if (body?.name) record.name = String(body.name);
    }
    return json({ ok: true });
  }
  if (automationMatch && method === "DELETE") {
    const automationId = decodeURIComponent(automationMatch[1]);
    state.automations.automations = state.automations.automations.filter(
      (automation) => automation.automation_id !== automationId,
    );
    return json({ ok: true });
  }

  // Extensions.
  if (path === `${V2}/extensions`) return json(DEMO_EXTENSIONS);
  if (path === `${V2}/extensions/registry`) return json(DEMO_REGISTRY);
  if (path.startsWith(`${V2}/extensions/`)) {
    if (path.endsWith("/setup") && method === "GET") {
      return json({ secrets: [], fields: [], onboarding: null });
    }
    return json({ success: true });
  }

  // Filesystem viewer.
  if (path.startsWith(`${V2}/fs/`)) {
    const response = routeFs(url);
    if (response) return response;
  }

  // Logs (caller-scoped and operator command plane).
  if (path === `${V2}/logs`) return json(DEMO_LOGS);
  if (path === `${V2}/operator/logs`) {
    return json({ area: "logs", status: "available", message: "ok", logs: DEMO_LOGS });
  }

  // LLM provider config (admin onboarding gate reads `active`).
  if (path === `${V2}/llm/providers` && method === "GET") {
    return json(DEMO_LLM_PROVIDERS);
  }
  if (path.startsWith(`${V2}/llm/`)) return json({ success: true });

  // Outbound delivery.
  if (path === `${V2}/outbound/preferences` && method === "GET") {
    return json(DEMO_OUTBOUND_PREFERENCES);
  }
  if (path === `${V2}/outbound/targets`) return json(DEMO_OUTBOUND_TARGETS);
  if (path === `${V2}/outbound/preferences`) return json({ ok: true });

  // Channels / skills / misc settings surfaces.
  if (path === `${V2}/channels/connectable`) return json(DEMO_CONNECTABLE_CHANNELS);
  if (path === `${V2}/skills`) return json(DEMO_SKILLS);
  if (path.startsWith(`${V2}/skills/`)) return json({ success: true });
  if (path === `${V2}/settings/tools` && method === "GET") {
    return json({ entries: [], diagnostics: [], precedence: [] });
  }
  if (path.startsWith(`${V2}/operator/config`)) {
    if (method === "GET") return json({ entry: null });
    return json({ entry: null, success: true });
  }

  // Reborn product-auth and anything else under the mocked prefixes:
  // succeed with an empty object so optional surfaces degrade quietly.
  return json(method === "GET" ? {} : { success: true });
}

export function installDemoBackend() {
  const realFetch = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const response = await route(input, init).catch(() => null);
    return response ?? realFetch(input, init);
  };
  window.EventSource = DemoEventSource;
  // eslint-disable-next-line no-console
  console.info("[ironclaw] demo mode: in-browser mock backend installed");
}
