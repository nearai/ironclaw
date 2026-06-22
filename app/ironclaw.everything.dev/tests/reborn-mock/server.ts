import http from "node:http";
import { applyScenario } from "./scenarios";
import { createDefaultState } from "./state";
import type { RebornMockHandle, RebornMockOptions, RebornMockState, ScenarioName } from "./types";

export async function startRebornMock(options: RebornMockOptions = {}): Promise<RebornMockHandle> {
  const token = options.token ?? "test-token-123";
  const scenario = options.scenario ?? "healthy-empty";

  const baseState = createDefaultState(token);
  let state: RebornMockState = applyScenario(baseState, scenario);

  function reset() {
    const fresh = createDefaultState(token);
    state = applyScenario(fresh, scenario);
  }

  function setScenario(name: ScenarioName) {
    const fresh = createDefaultState(token);
    state = applyScenario(fresh, name);
  }

  function tokenFromQuery(url: string | undefined): string | undefined {
    if (!url) return undefined;
    const idx = url.indexOf("?");
    if (idx === -1) return undefined;
    for (const part of url.slice(idx + 1).split("&")) {
      const [k, v] = part.split("=");
      if (decodeURIComponent(k) === "token") return decodeURIComponent(v ?? "");
    }
    return undefined;
  }

  function validateAuth(req: http.IncomingMessage): boolean {
    const auth = req.headers["authorization"];
    if (auth) {
      const parts = auth.split(" ");
      if (parts.length === 2 && parts[0] === "Bearer" && parts[1] === state.token) return true;
    }
    const queryToken = tokenFromQuery(req.url);
    if (queryToken && queryToken === state.token) return true;
    return false;
  }

  function jsonResponse(res: http.ServerResponse, status: number, data: unknown) {
    res.writeHead(status, { "Content-Type": "application/json" });
    res.end(JSON.stringify(data));
  }

  function notFound(res: http.ServerResponse) {
    jsonResponse(res, 404, { error: "not_found", message: "Not found" });
  }

  function requireAuth(req: http.IncomingMessage, res: http.ServerResponse): boolean {
    if (!validateAuth(req)) {
      jsonResponse(res, 401, { error: "unauthorized", message: "Invalid or missing bearer token" });
      return false;
    }
    return true;
  }

  async function readBody(req: http.IncomingMessage): Promise<unknown> {
    const chunks: Buffer[] = [];
    for await (const chunk of req) {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    }
    if (chunks.length === 0) return undefined;
    const body = Buffer.concat(chunks).toString("utf8");
    try {
      return JSON.parse(body);
    } catch {
      return body;
    }
  }

  function extractPath(url: string): string {
    const idx = url.indexOf("?");
    return idx === -1 ? url : url.slice(0, idx);
  }

  function sseHandler(req: http.IncomingMessage, res: http.ServerResponse) {
    if (!requireAuth(req, res)) return;

    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
      "X-Accel-Buffering": "no",
    });

    if (state.sseHandler) {
      const write = (event: string, data: unknown) => {
        res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
      };

      state
        .sseHandler(write)
        .catch(() => {})
        .finally(() => res.end());
    } else {
      res.end();
    }

    req.on("close", () => {});
  }

  const server = http.createServer(async (req, res) => {
    const method = req.method ?? "GET";
    const path = extractPath(req.url ?? "");

    if (
      method === "GET" &&
      path.startsWith("/api/webchat/v2/threads/") &&
      path.endsWith("/events")
    ) {
      const match = path.match(/\/api\/webchat\/v2\/threads\/([^/]+)\/events/);
      if (match) return sseHandler(req, res);
    }

    if (
      method === "GET" &&
      path.startsWith("/api/webchat/v2/threads/") &&
      path.endsWith("/timeline")
    ) {
      if (!requireAuth(req, res)) return;
      const match = path.match(/\/api\/webchat\/v2\/threads\/([^/]+)\/timeline/);
      if (!match) return notFound(res);
      const thread = state.threads.find((t) => t.threadId === match[1]);
      if (!thread) return jsonResponse(res, 200, { messages: [], next_cursor: null });
      return jsonResponse(res, 200, {
        messages: thread.messages.map((m) => ({
          message_id: m.messageId,
          thread_id: m.threadId,
          sequence: m.sequence,
          kind: m.kind,
          content: m.content,
          actor_id: m.actorId,
          status: m.status,
          turn_id: m.turnId,
          turn_run_id: m.turnRunId,
          created_at: m.createdAt,
        })),
        next_cursor: null,
      });
    }

    if (method === "DELETE" && path.startsWith("/api/webchat/v2/threads/")) {
      if (!requireAuth(req, res)) return;
      const threadId = path.slice("/api/webchat/v2/threads/".length);
      state.threads = state.threads.filter((t) => t.threadId !== threadId);
      return jsonResponse(res, 200, { success: true });
    }

    if (
      method === "POST" &&
      path.startsWith("/api/webchat/v2/threads/") &&
      path.endsWith("/messages")
    ) {
      if (!requireAuth(req, res)) return;
      const match = path.match(/\/api\/webchat\/v2\/threads\/([^/]+)\/messages/);
      if (!match) return notFound(res);
      const body = (await readBody(req)) as any;
      const thread = state.threads.find((t) => t.threadId === match[1]);
      if (!thread) return notFound(res);

      const msgId = `msg-${state.nextSeq++}`;
      const bodyContent = body?.content ?? "";
      const currentSeq = thread.messages.length + 1;
      const msgKind = "User" as const;
      const actorId = "user";
      const turnSeq = state.nextSeq;
      const createdAt = new Date().toISOString();

      state.eventCursor++;

      thread.messages.push({
        messageId: msgId,
        threadId: thread.threadId,
        sequence: currentSeq,
        kind: msgKind,
        content: bodyContent,
        actorId,
        status: "accepted",
        turnId: `turn-${turnSeq}`,
        turnRunId: `run-${turnSeq}`,
        createdAt,
      });

      return jsonResponse(res, 200, {
        outcome: "submitted",
        thread_id: thread.threadId,
        accepted_message_ref: msgId,
        status: "running",
        run_id: `run-${state.nextSeq}`,
        turn_id: `turn-${state.nextSeq}`,
        resolved_run_profile_id: "default",
        resolved_run_profile_version: "1",
        event_cursor: state.eventCursor,
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/session") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        tenant_id: state.session.tenantId,
        user_id: state.session.userId,
        capabilities: { operator_webui_config: state.session.capabilities.operatorWebuiConfig },
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/threads") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        threads: state.threads.map((t) => ({
          thread_id: t.threadId,
          title: t.title,
          scope: {
            tenant_id: state.session.tenantId,
            agent_id: state.session.userId,
            project_id: undefined,
            owner_user_id: undefined,
            mission_id: undefined,
          },
          created_by_actor_id: state.session.userId,
          created_at: t.createdAt,
        })),
        next_cursor: null,
      });
    }

    if (method === "POST" && path === "/api/webchat/v2/threads") {
      if (!requireAuth(req, res)) return;
      const threadId = `thread-${Date.now()}`;
      const thread = {
        threadId,
        title: "New Thread",
        createdAt: new Date().toISOString(),
        messages: [],
      };
      state.threads.push(thread);
      return jsonResponse(res, 200, {
        thread_id: threadId,
        title: "New Thread",
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/automations") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        automations: state.automations.map((a) => ({
          automation_id: a.id,
          name: a.name,
          source: { type: "schedule", cron: "0 9 * * *", timezone: "UTC" },
          state: a.status,
          is_active: a.isActive,
          recent_runs: [],
        })),
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/outbound/preferences") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        final_reply_target: state.outboundPrefs.finalReplyTarget
          ? {
              target_id: state.outboundPrefs.finalReplyTarget.targetId,
              channel: state.outboundPrefs.finalReplyTarget.channel,
              display_name: state.outboundPrefs.finalReplyTarget.displayName,
            }
          : null,
      });
    }

    if (method === "POST" && path === "/api/webchat/v2/outbound/preferences") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, { success: true });
    }

    if (method === "GET" && path === "/api/webchat/v2/outbound/targets") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        targets: state.outboundTargets.map((o) => ({
          target: {
            target_id: o.target.targetId,
            channel: o.target.channel,
            display_name: o.target.displayName,
          },
          capabilities: {
            final_replies: o.capabilities.finalReplies,
            gate_prompts: o.capabilities.gatePrompts,
            auth_prompts: o.capabilities.authPrompts,
          },
        })),
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/extensions") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        extensions: state.extensions.map((e) => ({
          package_ref: e.packageRef,
          display_name: e.displayName,
          kind: e.kind,
          description: e.description,
          active: e.active,
          authenticated: false,
          tools: [],
          needs_setup: false,
          has_auth: false,
        })),
      });
    }

    if (method === "GET" && path === "/api/webchat/v2/extensions/registry") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        entries: state.extensionRegistry.map((e) => ({
          package_ref: e.packageRef,
          display_name: e.displayName,
          kind: e.kind,
          description: e.description,
          installed: e.installed,
          keywords: [],
        })),
      });
    }

    if (method === "POST" && path === "/api/webchat/v2/extensions/install") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, { success: true, message: "Extension installed" });
    }

    if (method === "GET" && path === "/api/webchat/v2/skills") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        skills: state.skills.map((s) => ({
          name: s.name,
          description: s.description,
          version: s.version,
          trust: s.trust,
          source: s.source,
          keywords: [],
          usage_hint: undefined,
          setup_hint: undefined,
        })),
        count: state.skills.length,
      });
    }

    if (method === "POST" && path === "/api/webchat/v2/skills/search") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        catalog: [],
        installed: state.skills.map((s) => ({
          name: s.name,
          description: s.description,
          version: s.version,
          trust: s.trust,
          source: s.source,
          keywords: [],
        })),
        registry_url: "https://hub.ironclaw.com/registry",
      });
    }

    if (method === "POST" && path === "/api/webchat/v2/skills/install") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, { success: true, message: "Skill installed" });
    }

    if (method === "GET" && path === "/api/webchat/v2/channels/connectable") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, {
        channels: state.connectableChannels.map((c) => ({
          channel: c.channel,
          display_name: c.displayName,
          strategy: c.strategy,
          action: null,
          command_aliases: [],
        })),
      });
    }

    if (method === "GET" && path === "/auth/providers") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, state.authProviders);
    }

    if (method === "POST" && path === "/auth/session/exchange") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, { token: "exchanged-session-token" });
    }

    if (method === "POST" && path === "/auth/logout") {
      if (!requireAuth(req, res)) return;
      return jsonResponse(res, 200, { success: true });
    }

    jsonResponse(res, 404, {
      error: "not_found",
      message: `No mock handler for ${method} ${path}`,
    });
  });

  const port = options.port ?? 0;
  await new Promise<void>((resolve, reject) => {
    server.listen(port, "127.0.0.1", () => resolve());
    server.on("error", reject);
  });

  const addr = server.address();
  const baseUrl = `http://127.0.0.1:${typeof addr === "object" && addr ? addr.port : port}`;

  return {
    baseUrl,
    token,
    state,
    stop: () => new Promise<void>((resolve) => server.close(() => resolve())),
    reset,
    setScenario,
  };
}
