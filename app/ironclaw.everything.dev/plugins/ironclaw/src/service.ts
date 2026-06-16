import { Effect } from "every-plugin/effect";
import type { z } from "every-plugin/zod";

import type {
  AcceptedResponseSchema,
  AttachmentCapabilitiesSchema,
  AutomationSchema,
  ChatEventSchema,
  ConnectableChannelSchema,
  ExtensionActionResponseSchema,
  ExtensionRegistryEntrySchema,
  ExtensionSchema,
  ExtensionSetupDetailSchema,
  OutboundPreferencesSchema,
  OutboundTargetSchema,
  SessionSchema,
  SkillActionResponseSchema,
  SkillContentResponseSchema,
  SkillSchema,
  SkillSearchResponseSchema,
  ThreadCreateSchema,
  ThreadListSchema,
  ThreadSchema,
  ThreadStateSchema,
  TimelineEntrySchema,
  TimelineSchema,
} from "./contract";

type Session = z.infer<typeof SessionSchema>;
type ThreadList = z.infer<typeof ThreadListSchema>;
type ThreadCreate = z.infer<typeof ThreadCreateSchema>;
type Timeline = z.infer<typeof TimelineSchema>;
type AcceptedResponse = z.infer<typeof AcceptedResponseSchema>;
type ChatEvent = z.infer<typeof ChatEventSchema>;
type OutboundPreferences = z.infer<typeof OutboundPreferencesSchema>;
type Automation = z.infer<typeof AutomationSchema>;
type Extension = z.infer<typeof ExtensionSchema>;
type ExtensionRegistryEntry = z.infer<typeof ExtensionRegistryEntrySchema>;
type ExtensionActionResponse = z.infer<typeof ExtensionActionResponseSchema>;
type Skill = z.infer<typeof SkillSchema>;
type SkillSearchResponse = z.infer<typeof SkillSearchResponseSchema>;
type SkillContentResponse = z.infer<typeof SkillContentResponseSchema>;
type SkillActionResponse = z.infer<typeof SkillActionResponseSchema>;
type ConnectableChannel = z.infer<typeof ConnectableChannelSchema>;
type ThreadState = z.infer<typeof ThreadStateSchema>;
type AttachmentCapabilities = z.infer<typeof AttachmentCapabilitiesSchema>;

const BODY_METHODS = new Set(["POST", "PUT", "PATCH"]);
const REQUEST_TIMEOUT_MS = 30_000;

export class IronclawUpstreamError extends Error {
  readonly status: number;
  readonly method: string;
  readonly path: string;
  readonly upstreamBody: string;
  readonly upstreamJson: Record<string, unknown> | null;

  constructor(status: number, method: string, path: string, body: string) {
    let parsed: Record<string, unknown> | null = null;
    try {
      parsed = JSON.parse(body);
    } catch {}
    const msg =
      parsed && typeof parsed.message === "string"
        ? parsed.message
        : `Ironclaw API error ${status}`;
    super(msg);
    this.name = "IronclawUpstreamError";
    this.status = status;
    this.method = method;
    this.path = path;
    this.upstreamBody = body;
    this.upstreamJson = parsed;
  }
}

function snakeToCamel(str: string): string {
  return str.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
}

function transformKeys(obj: unknown): unknown {
  if (obj === null || obj === undefined) return obj;
  if (Array.isArray(obj)) return obj.map(transformKeys);
  if (typeof obj !== "object") return obj;
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
    const camelKey = snakeToCamel(key);
    result[camelKey] = transformKeys(value);
  }
  return result;
}

function mapThreadEntry(m: any): z.infer<typeof TimelineEntrySchema> {
  const kind = m.kind ?? "";
  const role = (() => {
    const lower = kind.toLowerCase();
    if (lower === "user" || lower === "user_message") return "user";
    if (lower === "system") return "system";
    if (lower === "assistant" || lower === "assistant_message" || lower === "tool_result")
      return "assistant";
    if (m.actor_id ?? m.actorId) return "user";
    return "assistant";
  })();
  return {
    messageId: m.message_id,
    threadId: m.thread_id,
    sequence: m.sequence,
    kind: m.kind,
    status: m.status,
    actorId: m.actor_id ?? undefined,
    sourceBindingId: m.source_binding_id ?? undefined,
    replyTargetBindingId: m.reply_target_binding_id ?? undefined,
    turnId: m.turn_id ?? undefined,
    turnRunId: m.turn_run_id ?? undefined,
    toolResultRef: m.tool_result_ref ?? undefined,
    content: m.content ?? undefined,
    redactionRef: m.redaction_ref ?? undefined,
    role,
    createdAt: m.created_at ?? undefined,
    attachments: (m.attachments ?? []).map((att: any) => ({
      id: att.id ?? "",
      kind: att.kind ?? "document",
      mimeType: att.mime_type ?? "application/octet-stream",
      filename: att.filename ?? undefined,
      sizeBytes: att.size_bytes ?? undefined,
      storageKey: att.storage_key ?? undefined,
      extractedText: att.extracted_text ?? undefined,
      previewUrl: att.preview_url ?? undefined,
    })),
  };
}

function mapThreadRecord(t: any): z.infer<typeof ThreadSchema> {
  return {
    threadId: t.thread_id,
    scope: {
      tenantId: t.scope?.tenant_id,
      agentId: t.scope?.agent_id,
      projectId: t.scope?.project_id ?? undefined,
      ownerUserId: t.scope?.owner_user_id ?? undefined,
      missionId: t.scope?.mission_id ?? undefined,
    },
    createdByActorId: t.created_by_actor_id,
    title: t.title ?? undefined,
    metadataJson: t.metadata_json ?? undefined,
    createdAt: t.created_at ?? undefined,
    updatedAt: t.updated_at ?? undefined,
    goal: t.goal
      ? {
          statement: t.goal.statement,
          refinedAtSequence: t.goal.refined_at_sequence,
          refinementCount: t.goal.refinement_count,
        }
      : undefined,
  };
}

function mapSkill(s: any): Skill {
  return {
    name: s.name,
    description: s.description,
    version: s.version,
    trust: s.trust,
    source: s.source,
    keywords: s.keywords ?? [],
    usageHint: s.usage_hint ?? undefined,
    setupHint: s.setup_hint ?? undefined,
    bundlePath: s.bundle_path ?? undefined,
    installSourceUrl: s.install_source_url ?? undefined,
    hasRequirements: s.has_requirements ?? false,
    hasScripts: s.has_scripts ?? false,
    canEdit: s.can_edit ?? false,
    canDelete: s.can_delete ?? false,
  };
}

export class IronclawService {
  private baseUrl: string;
  private token: string;

  constructor(baseUrl: string, token: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    params?: Record<string, string | undefined>,
  ): Promise<T> {
    const url = new URL(`${this.baseUrl}${path}`);
    if (params) {
      for (const [key, value] of Object.entries(params)) {
        if (value !== undefined) url.searchParams.set(key, value);
      }
    }

    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
    };

    if (body !== undefined || BODY_METHODS.has(method.toUpperCase())) {
      headers["Content-Type"] = "application/json";
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

    try {
      const response = await fetch(url.toString(), {
        method,
        headers,
        body: body !== undefined ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      });

      if (!response.ok) {
        const errorBody = await response.text().catch(() => "");
        throw new IronclawUpstreamError(response.status, method, path, errorBody);
      }

      return response.json();
    } finally {
      clearTimeout(timeout);
    }
  }

  ping() {
    return Effect.tryPromise({
      try: async () => {
        await this.request("GET", "/api/webchat/v2/session");
        return { status: "ok" as const, timestamp: new Date().toISOString() };
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Ironclaw health check failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  getSession(): Effect.Effect<Session, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/session");
        const caps = raw.capabilities ?? {};
        const attCaps: AttachmentCapabilities | undefined = caps.attachments
          ? {
              accept: caps.attachments.accept ?? [],
              maxCount: caps.attachments.max_count ?? 0,
              maxFileBytes: caps.attachments.max_file_bytes ?? 0,
              maxTotalBytes: caps.attachments.max_total_bytes ?? 0,
            }
          : undefined;
        return {
          tenantId: raw.tenant_id,
          userId: raw.user_id,
          capabilities: {
            operatorWebuiConfig: caps.operator_webui_config ?? false,
            attachments: attCaps,
          },
        } as Session;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to get session: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  listThreads(limit?: number, cursor?: string): Effect.Effect<ThreadList, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/threads", undefined, {
          limit: limit?.toString(),
          cursor,
        });
        const threads = (raw.threads ?? []).map(mapThreadRecord);
        return {
          data: threads,
          meta: {
            total: threads.length,
            hasMore: raw.next_cursor != null,
            nextCursor: raw.next_cursor ?? null,
          },
        } as ThreadList;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to list threads: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  createThread(clientActionId?: string): Effect.Effect<ThreadCreate, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("POST", "/api/webchat/v2/threads", {
          client_action_id: clientActionId ?? crypto.randomUUID(),
        });
        const t = raw.thread ?? raw;
        return { threadId: t.thread_id, title: t.title ?? undefined } as ThreadCreate;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to create thread: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  deleteThread(id: string): Effect.Effect<void, Error> {
    return Effect.tryPromise({
      try: () => this.request<void>("DELETE", `/api/webchat/v2/threads/${encodeURIComponent(id)}`),
      catch: (error: unknown) =>
        new Error(
          `Failed to delete thread: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  sendMessage(
    id: string,
    content: string,
    clientActionId?: string,
    attachments?: Array<{ mimeType: string; filename?: string; dataBase64: string }>,
  ): Effect.Effect<AcceptedResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const body: Record<string, unknown> = {
          client_action_id: clientActionId ?? crypto.randomUUID(),
          content,
        };
        if (attachments && attachments.length > 0) {
          body.attachments = attachments.map((a) => ({
            mime_type: a.mimeType,
            filename: a.filename,
            data_base64: a.dataBase64,
          }));
        }
        const raw: any = await this.request(
          "POST",
          `/api/webchat/v2/threads/${encodeURIComponent(id)}/messages`,
          body,
        );
        const base = {
          outcome: raw.outcome,
          threadId: raw.thread_id,
          acceptedMessageRef: raw.accepted_message_ref,
          status: raw.status,
        };
        if (raw.outcome === "submitted") {
          return {
            ...base,
            runId: raw.run_id,
            turnId: raw.turn_id,
            resolvedRunProfileId: raw.resolved_run_profile_id,
            resolvedRunProfileVersion: raw.resolved_run_profile_version,
            eventCursor: raw.event_cursor,
          } as AcceptedResponse;
        }
        if (raw.outcome === "deferred_busy") {
          return {
            ...base,
            activeRunId: raw.active_run_id,
            eventCursor: raw.event_cursor,
          } as AcceptedResponse;
        }
        return {
          ...base,
          runId: raw.run_id,
          eventCursor: raw.event_cursor,
        } as AcceptedResponse;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to send message: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  getTimeline(id: string, limit?: number, cursor?: string): Effect.Effect<Timeline, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "GET",
          `/api/webchat/v2/threads/${encodeURIComponent(id)}/timeline`,
          undefined,
          {
            limit: limit?.toString(),
            cursor,
          },
        );
        const messages = (raw.messages ?? []).map(mapThreadEntry);
        return {
          data: messages,
          meta: {
            total: messages.length,
            hasMore: raw.next_cursor != null,
            nextCursor: raw.next_cursor ?? null,
          },
        } as Timeline;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to get timeline: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  cancelRun(
    id: string,
    runId: string,
  ): Effect.Effect<
    {
      success: boolean;
      runId: string;
      status: string;
      eventCursor: number;
      alreadyTerminal: boolean;
    },
    Error
  > {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "POST",
          `/api/webchat/v2/threads/${encodeURIComponent(id)}/runs/${encodeURIComponent(runId)}/cancel`,
        );
        return {
          success: true,
          runId: raw.run_id,
          status: raw.status,
          eventCursor: raw.event_cursor,
          alreadyTerminal: raw.already_terminal,
        };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to cancel run: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  resolveGate(
    id: string,
    runId: string,
    gateRef: string,
    resolution: string,
    always?: boolean,
  ): Effect.Effect<void, Error> {
    return Effect.tryPromise({
      try: () => {
        const body: Record<string, unknown> = { resolution };
        if (always !== undefined) body.always = always;
        return this.request<void>(
          "POST",
          `/api/webchat/v2/threads/${encodeURIComponent(id)}/runs/${encodeURIComponent(runId)}/gates/${encodeURIComponent(gateRef)}/resolve`,
          body,
        );
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to resolve gate: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  streamEvents(id: string, afterCursor?: string): AsyncGenerator<ChatEvent> {
    const baseUrl = this.baseUrl;
    const token = this.token;
    const generator: AsyncGenerator<ChatEvent> = (async function* () {
      let cursor = afterCursor;

      const terminalTypes = new Set(["final_reply", "cancelled", "failed"]);
      const sseEventTypes = new Set([
        "accepted",
        "running",
        "capability_progress",
        "capability_activity",
        "capability_display_preview",
        "gate",
        "auth_required",
        "final_reply",
        "cancelled",
        "failed",
        "projection_snapshot",
        "projection_update",
        "keep_alive",
      ]);

      const transformEvent = (raw: any): ChatEvent => {
        const base: any = {
          cursor: raw.cursor ?? undefined,
          type: raw.type,
        };
        if (raw.ack) {
          base.ack = {
            outcome: raw.ack.outcome,
            threadId: raw.ack.thread_id,
            runId: raw.ack.run_id ?? undefined,
            activeRunId: raw.ack.active_run_id ?? undefined,
            acceptedMessageRef: raw.ack.accepted_message_ref,
            status: raw.ack.status,
            turnId: raw.ack.turn_id ?? undefined,
            eventCursor: raw.ack.event_cursor ?? undefined,
          };
        }
        if (raw.progress) {
          base.progress = {
            kind: raw.progress.kind,
            turnRunId: raw.progress.turn_run_id ?? undefined,
            generatedAt: raw.progress.generated_at ?? undefined,
          };
        }
        if (raw.activity) {
          base.activity = transformKeys(raw.activity);
        }
        if (raw.preview) {
          base.preview = transformKeys(raw.preview);
        }
        if (raw.reply) {
          base.reply = {
            text: raw.reply.text,
            turnRunId: raw.reply.turn_run_id,
            generatedAt: raw.reply.generated_at,
          };
        }
        if (raw.prompt) {
          base.prompt = {
            turnRunId: raw.prompt.turn_run_id,
            gateRef: raw.prompt.gate_ref,
            headline: raw.prompt.headline,
            body: raw.prompt.body,
            allowAlways: raw.prompt.allow_always ?? undefined,
            approvalContext: raw.prompt.approval_context
              ? {
                  toolName: raw.prompt.approval_context.tool_name,
                  action: raw.prompt.approval_context.action,
                  scope: raw.prompt.approval_context.scope,
                  reason: raw.prompt.approval_context.reason ?? undefined,
                  destination: raw.prompt.approval_context.destination ?? undefined,
                  details: raw.prompt.approval_context.details ?? undefined,
                }
              : undefined,
          };
        }
        if (raw.auth_prompt) {
          base.authPrompt = transformKeys(raw.auth_prompt) as any;
        }
        if (raw.type === "cancelled" && raw.response) {
          base.response = {
            runId: raw.response.run_id,
            status: raw.response.status,
            eventCursor: raw.response.event_cursor,
            alreadyTerminal: raw.response.already_terminal,
          };
        }
        if (raw.type === "failed" && raw.run_state) {
          base.runState = {
            turnId: raw.run_state.turn_id,
            runId: raw.run_state.run_id,
            status: raw.run_state.status,
            eventCursor: raw.run_state.event_cursor,
            acceptedMessageRef: raw.run_state.accepted_message_ref,
            resolvedRunProfileId: raw.run_state.resolved_run_profile_id,
            resolvedRunProfileVersion: raw.run_state.resolved_run_profile_version,
            receivedAt: raw.run_state.received_at,
            checkpointId: raw.run_state.checkpoint_id ?? undefined,
            gateRef: raw.run_state.gate_ref ?? undefined,
            failure: raw.run_state.failure ?? undefined,
          };
        }
        if (raw.state) {
          base.state = transformKeys(raw.state);
        }
        return base as ChatEvent;
      };

      let retryDelayMs = 1_000;

      while (true) {
        const url = `${baseUrl}/api/webchat/v2/threads/${encodeURIComponent(id)}/events?token=${encodeURIComponent(token)}${cursor ? `&after_cursor=${encodeURIComponent(cursor)}` : ""}`;
        let sessionEnded = false;

        try {
          const response = await fetch(url, {
            headers: { Accept: "text/event-stream" },
          });

          if (!response.ok || !response.body) throw new Error(`SSE connection failed: ${response.status}`);

          const reader = response.body.getReader();
          const decoder = new TextDecoder();
          let buffer = "";

          try {
            while (!sessionEnded) {
              const { done, value } = await reader.read();
              if (done) break;

              buffer += decoder.decode(value, { stream: true });

              const parts = buffer.split("\n\n");
              buffer = parts.pop() || "";

              for (const part of parts) {
                const lines = part.split("\n");
                let eventName = "";
                let dataStr = "";

                for (const line of lines) {
                  if (line.startsWith("event: ")) eventName = line.slice(7).trim();
                  else if (line.startsWith("data: ")) dataStr = line.slice(6);
                }

                if (!eventName || !dataStr || !sseEventTypes.has(eventName)) continue;

                try {
                  const raw = JSON.parse(dataStr);
                  const event = transformEvent(raw);
                  if (event.cursor) cursor = event.cursor;
                  yield event;
                  if (terminalTypes.has(event.type)) {
                    sessionEnded = true;
                  }
                } catch {
                }
              }
            }
          } finally {
            try {
              reader.releaseLock();
            } catch {}
          }
        } catch {
        }

        if (sessionEnded) return;

        await new Promise((res) => setTimeout(res, retryDelayMs));
        retryDelayMs = Math.min(retryDelayMs * 2, 30_000);
      }
    })();
    return generator;
  }

  listAutomations(limit?: number, runLimit?: number): Effect.Effect<{ data: Automation[] }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/automations", undefined, {
          limit: limit?.toString(),
          run_limit: runLimit?.toString(),
        });
        const automations: Automation[] = (raw.automations ?? []).map((a: any) => ({
          id: a.automation_id,
          name: a.name,
          source: {
            type: "schedule" as const,
            cron: a.source?.cron ?? "",
            timezone: a.source?.timezone ?? "UTC",
          },
          state: a.state,
          nextRunAt: a.next_run_at ?? undefined,
          lastRunAt: a.last_run_at ?? undefined,
          lastStatus: a.last_status ?? undefined,
          recentRuns: (a.recent_runs ?? []).map((r: any) => ({
            runId: r.run_id ?? undefined,
            threadId: r.thread_id ?? undefined,
            fireSlot: r.fire_slot ?? undefined,
            status: r.status,
            submittedAt: r.submitted_at,
            completedAt: r.completed_at ?? undefined,
          })),
          isActive: a.is_active ?? false,
          createdAt: a.created_at ?? undefined,
        }));
        return { data: automations };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list automations: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  getOutboundPreferences(): Effect.Effect<OutboundPreferences, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/outbound/preferences");
        return {
          finalReplyTarget: raw.final_reply_target
            ? {
                targetId: raw.final_reply_target.target_id,
                channel: raw.final_reply_target.channel,
                displayName: raw.final_reply_target.display_name,
                description: raw.final_reply_target.description ?? undefined,
              }
            : undefined,
          status: raw.final_reply_target_status ?? undefined,
          modality: raw.default_modality ?? undefined,
        } as OutboundPreferences;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to get preferences: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  setOutboundPreferences(prefs: OutboundPreferences): Effect.Effect<void, Error> {
    return Effect.tryPromise({
      try: async () => {
        const body: Record<string, unknown> = {};
        if (prefs.finalReplyTarget?.targetId) {
          body.final_reply_target_id = prefs.finalReplyTarget.targetId;
        }
        await this.request<void>("POST", "/api/webchat/v2/outbound/preferences", body);
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to set preferences: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listOutboundTargets(): Effect.Effect<
    { data: z.infer<typeof OutboundTargetSchema>[]; nextCursor?: string | null },
    Error
  > {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/outbound/targets");
        const targets = (raw.targets ?? []).map((o: any) => ({
          target: {
            targetId: o.target?.target_id,
            channel: o.target?.channel,
            displayName: o.target?.display_name,
            description: o.target?.description ?? undefined,
          },
          capabilities: {
            finalReplies: o.capabilities?.final_replies ?? false,
            gatePrompts: o.capabilities?.gate_prompts ?? false,
            authPrompts: o.capabilities?.auth_prompts ?? false,
          },
        }));
        return { data: targets, nextCursor: raw.next_cursor ?? null };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list targets: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listExtensions(): Effect.Effect<{ data: Extension[] }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/extensions");
        const extensions: Extension[] = (raw.extensions ?? []).map((e: any) => ({
          packageRef: { kind: e.package_ref?.kind, id: e.package_ref?.id },
          displayName: e.display_name,
          kind: e.kind,
          description: e.description,
          authenticated: e.authenticated ?? false,
          active: e.active ?? false,
          tools: e.tools ?? [],
          needsSetup: e.needs_setup ?? false,
          hasAuth: e.has_auth ?? false,
          activationStatus: e.activation_status ?? undefined,
          activationError: e.activation_error ?? undefined,
          version: e.version ?? undefined,
          onboardingState: e.onboarding_state ?? undefined,
          onboarding: e.onboarding
            ? {
                credentialInstructions: e.onboarding.credential_instructions ?? undefined,
                setupUrl: e.onboarding.setup_url ?? undefined,
                credentialNextStep: e.onboarding.credential_next_step ?? undefined,
              }
            : undefined,
        }));
        return { data: extensions };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list extensions: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listExtensionRegistry(): Effect.Effect<{ data: ExtensionRegistryEntry[] }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/extensions/registry");
        const entries: ExtensionRegistryEntry[] = (raw.entries ?? []).map((e: any) => ({
          packageRef: { kind: e.package_ref?.kind, id: e.package_ref?.id },
          displayName: e.display_name,
          kind: e.kind,
          description: e.description,
          installed: e.installed ?? false,
          keywords: e.keywords ?? [],
          version: e.version ?? undefined,
        }));
        return { data: entries };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list extension registry: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  installExtension(packageRef: {
    kind: string;
    id: string;
  }): Effect.Effect<ExtensionActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("POST", "/api/webchat/v2/extensions/install", {
          package_ref: packageRef,
        });
        return transformKeys(raw) as ExtensionActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to install extension: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  activateExtension(name: string): Effect.Effect<ExtensionActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "POST",
          `/api/webchat/v2/extensions/${encodeURIComponent(name)}/activate`,
        );
        return transformKeys(raw) as ExtensionActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to activate extension: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  removeExtension(name: string): Effect.Effect<ExtensionActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "POST",
          `/api/webchat/v2/extensions/${encodeURIComponent(name)}/remove`,
        );
        return transformKeys(raw) as ExtensionActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to remove extension: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  getExtensionSetup(
    name: string,
  ): Effect.Effect<z.infer<typeof ExtensionSetupDetailSchema>, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "GET",
          `/api/webchat/v2/extensions/${encodeURIComponent(name)}/setup`,
        );
        return transformKeys(raw);
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to get extension setup: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  setupExtension(
    name: string,
    action: string,
    payload?: Record<string, unknown>,
  ): Effect.Effect<{ success: boolean; message?: string }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "POST",
          `/api/webchat/v2/extensions/${encodeURIComponent(name)}/setup`,
          { action, payload },
        );
        return { success: raw.success ?? false, message: raw.message ?? undefined };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to setup extension: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listSkills(): Effect.Effect<{ data: Skill[]; count: number }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/skills");
        const skills: Skill[] = (raw.skills ?? []).map(mapSkill);
        return { data: skills, count: raw.count ?? skills.length };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list skills: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  searchSkills(query: string): Effect.Effect<SkillSearchResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("POST", "/api/webchat/v2/skills/search", { query });
        const installed: Skill[] = (raw.installed ?? []).map(mapSkill);
        return {
          catalog: raw.catalog ?? [],
          installed,
          registryUrl: raw.registry_url ?? "",
          catalogError: raw.catalog_error ?? undefined,
        } as SkillSearchResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to search skills: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  installSkill(name: string, content?: string): Effect.Effect<SkillActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("POST", "/api/webchat/v2/skills/install", {
          name,
          content,
        });
        return { success: raw.success ?? false, message: raw.message ?? "" } as SkillActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to install skill: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  getSkill(name: string): Effect.Effect<SkillContentResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "GET",
          `/api/webchat/v2/skills/${encodeURIComponent(name)}`,
        );
        return { name: raw.name, content: raw.content } as SkillContentResponse;
      },
      catch: (error: unknown) =>
        new Error(`Failed to get skill: ${error instanceof Error ? error.message : String(error)}`),
    });
  }

  updateSkill(name: string, content: string): Effect.Effect<SkillActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "PUT",
          `/api/webchat/v2/skills/${encodeURIComponent(name)}`,
          { content },
        );
        return { success: raw.success ?? false, message: raw.message ?? "" } as SkillActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to update skill: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  removeSkill(name: string): Effect.Effect<SkillActionResponse, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "DELETE",
          `/api/webchat/v2/skills/${encodeURIComponent(name)}`,
        );
        return { success: raw.success ?? false, message: raw.message ?? "" } as SkillActionResponse;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to remove skill: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listConnectableChannels(): Effect.Effect<{ data: ConnectableChannel[] }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("GET", "/api/webchat/v2/channels/connectable");
        const channels: ConnectableChannel[] = (raw.channels ?? []).map((c: any) => ({
          channel: c.channel,
          displayName: c.display_name,
          strategy: c.strategy,
          action: {
            title: c.action?.title,
            instructions: c.action?.instructions,
            inputPlaceholder: c.action?.input_placeholder,
            submitLabel: c.action?.submit_label,
            successMessage: c.action?.success_message,
            errorMessage: c.action?.error_message,
          },
          commandAliases: c.command_aliases ?? [],
        }));
        return { data: channels };
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to list connectable channels: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  listAuthProviders(): Effect.Effect<
    { data: Array<{ id: string; name: string; type: string }> },
    Error
  > {
    return Effect.tryPromise({
      try: () => this.request("GET", "/auth/providers"),
      catch: (error: unknown) =>
        new Error(
          `Failed to list providers: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  exchangeLoginTicket(loginTicket: string): Effect.Effect<{ token: string }, Error> {
    return Effect.tryPromise({
      try: () =>
        this.request<{ token: string }>("POST", "/auth/session/exchange", {
          login_ticket: loginTicket,
        }),
      catch: (error: unknown) =>
        new Error(
          `Failed to exchange ticket: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  logout(): Effect.Effect<void, Error> {
    return Effect.tryPromise({
      try: () => this.request<void>("POST", "/auth/logout"),
      catch: (error: unknown) =>
        new Error(`Failed to logout: ${error instanceof Error ? error.message : String(error)}`),
    });
  }

  getThreadState(id: string): Effect.Effect<ThreadState, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request(
          "GET",
          `/api/webchat/v2/threads/${encodeURIComponent(id)}/state`,
        );
        return {
          thread: mapThreadRecord(raw.thread),
          messages: (raw.messages ?? []).map(mapThreadEntry),
          summaryArtifacts: raw.summary_artifacts ?? [],
        } as ThreadState;
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to get thread state: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }

  createAccessSession(
    tenantId: string,
    agentId?: string,
    projectId?: string,
  ): Effect.Effect<{ token: string; expiresAt: string }, Error> {
    return Effect.tryPromise({
      try: async () => {
        const raw: any = await this.request("POST", "/api/webchat/v2/operator/access-sessions", {
          tenant_id: tenantId,
          agent_id: agentId,
          project_id: projectId,
        });
        return { token: raw.token, expiresAt: raw.expires_at };
      },
      catch: (error: unknown) => {
        if (error instanceof IronclawUpstreamError) return error;
        return new Error(
          `Failed to create access session: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    });
  }
}
