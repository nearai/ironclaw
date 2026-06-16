import type { ContractRouterClient } from "@orpc/contract";
import { and, count, desc, eq } from "drizzle-orm";
import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";
import type { ContractType as IronclawContract } from "../../plugins/ironclaw/src/contract";
import { contract } from "./contract";
import { loadMigrations } from "./db/load-migrations";
import { migrate } from "./db/migrator";
import {
  ironclawConnections,
  ironclawScopeBindings,
  registrations,
  submissions,
  userPreferences,
} from "./db/schema";
import { createAuthMiddleware } from "./lib/auth";
import { normalizeThread, normalizeTimelinePage } from "./lib/conversation";
import { createThreadChatBridge } from "./lib/conversation-live";
import type { PluginsClient } from "./lib/plugins-types.gen";

function generateId(): string {
  return `hc_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

async function lookupCredentialsByScope(db: any, tenantId: string, scopeType: string) {
  const bindings = await db
    .select()
    .from(ironclawScopeBindings)
    .where(
      and(
        eq(ironclawScopeBindings.tenantId, tenantId),
        eq(ironclawScopeBindings.scopeType, scopeType),
      ),
    );
  if (bindings.length > 0) {
    const conns = await db
      .select()
      .from(ironclawConnections)
      .where(eq(ironclawConnections.id, bindings[0].connectionId));
    if (conns.length > 0) {
      return {
        tunnelUrl: conns[0].tunnelUrl,
        apiToken: conns[0].apiToken,
        connectionId: conns[0].id,
      };
    }
  }
  return null;
}

async function mintAccessSession(baseUrl: string, operatorToken: string, tenantId: string) {
  try {
    const resp = await fetch(
      `${baseUrl.replace(/\/+$/, "")}/api/webchat/v2/operator/access-sessions`,
      {
        method: "POST",
        headers: {
          Authorization: `Bearer ${operatorToken}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ tenant_id: tenantId }),
        signal: AbortSignal.timeout(10_000),
      },
    );
    if (resp.ok) {
      const data = (await resp.json()) as { token?: string };
      if (data.token) return data.token;
    }
  } catch {}
  return operatorToken;
}

function resolveTenantForScope(
  scope: string,
  userId: string | undefined,
  organizationId: string | undefined,
): string | null {
  if (scope === "personal") return userId ?? null;
  if (scope === "organization") return organizationId ?? null;
  if (scope === "platform") return "__platform_default__";
  return null;
}

type Ic = ContractRouterClient<IronclawContract>;

const h0 =
  (services: { ironclaw: (ctx: any) => Ic }, select: (ic: Ic) => () => any) =>
  async ({ context }: any) => {
    const ic = services.ironclaw(context);
    return await select(ic)();
  };

const h1 =
  (services: { ironclaw: (ctx: any) => Ic }, select: (ic: Ic) => (input: any) => any) =>
  async ({ input, context }: any) => {
    const ic = services.ironclaw(context);
    return await select(ic)(input);
  };

const hStream = (
  services: { ironclaw: (ctx: any) => Ic },
  select: (ic: Ic) => (input: any) => any,
) =>
  async function* ({ input, signal, context }: any) {
    const ic = services.ironclaw(context);
    try {
      const events = await select(ic)(input);
      for await (const event of events) {
        if (signal?.aborted) break;
        yield event;
      }
    } catch (error) {
      throw error;
    }
  };

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

export default createPlugin.withPlugins<PluginsClient>()({
  variables: z.object({}),

  secrets: z.object({
    API_DATABASE_URL: z.string().default("pglite:.bos/api/:memory:"),
    IRONCLAW_BASE_URL: z.string().optional(),
    IRONCLAW_API_TOKEN: z.string().optional(),
  }),

  context: z.object({
    userId: z.string().optional(),
    user: z
      .object({
        id: z.string(),
        role: z.string().optional(),
        email: z.string().optional(),
        name: z.string().optional(),
      })
      .optional(),
    organizationId: z.string().optional(),
    apiKey: z
      .object({
        id: z.string(),
        userId: z.string().optional(),
        permissions: z.record(z.string(), z.array(z.string())).optional(),
      })
      .optional(),
    reqHeaders: z.custom<Headers>().optional(),
    getRawBody: z.custom<() => Promise<string>>().optional(),
  }),

  contract,

  initialize: (config, plugins) =>
    Effect.promise(async () => {
      const { createDatabaseDriver } = await import("./db/index");
      const driver = await createDatabaseDriver(config.secrets.API_DATABASE_URL);

      const migrations = await loadMigrations();
      await migrate(driver.db, migrations);
      console.log("[API] Migrations applied");

      const { auth, ironclaw, ...restPlugins } = plugins;
      console.log("[API] Services Initialized");

      return {
        ironclaw,
        auth,
        plugins: restPlugins,
        db: driver.db,
        driver,
        secrets: config.secrets,
      };
    }),

  shutdown: (services) =>
    Effect.promise(async () => {
      console.log("[API] Shutdown");
      await (services as any).driver?.close?.();
    }),

  createRouter: (services, builder) => {
    const s = services as any;
    const { requireAuth } = createAuthMiddleware(builder);

    const resolveCredentials = builder.middleware(async ({ context, next }) => {
      const { userId, organizationId, apiKey } = context;

      if (s.db) {
        try {
          // Check if user explicitly prefers hosted mode — skip DB entirely
          if (userId) {
            const prefs = await s.db
              .select()
              .from(userPreferences)
              .where(eq(userPreferences.userId, userId));
            if (prefs[0]?.ironclawMode === "hosted") {
              if (s.secrets?.IRONCLAW_BASE_URL && s.secrets?.IRONCLAW_API_TOKEN) {
                const tenantId = organizationId ?? userId;
                const sessionToken = tenantId
                  ? await mintAccessSession(
                      s.secrets.IRONCLAW_BASE_URL,
                      s.secrets.IRONCLAW_API_TOKEN,
                      tenantId,
                    )
                  : s.secrets.IRONCLAW_API_TOKEN;
                return next({
                  context: {
                    ...context,
                    baseUrl: s.secrets.IRONCLAW_BASE_URL,
                    apiToken: sessionToken,
                  },
                });
              }
              if (s.secrets?.IRONCLAW_BASE_URL) {
                return next({ context });
              }
              return next({ context });
            }
          }

          // Priority 1: org-level credentials (when org context is active)
          if (organizationId) {
            const orgCreds = await lookupCredentialsByScope(s.db, organizationId, "organization");
            if (orgCreds) {
              return next({
                context: { ...context, baseUrl: orgCreds.tunnelUrl, apiToken: orgCreds.apiToken },
              });
            }
          }

          // Priority 2: personal credentials (checked regardless of org context)
          const effectiveUserId = userId ?? apiKey?.userId;
          if (effectiveUserId) {
            const personalCreds = await lookupCredentialsByScope(s.db, effectiveUserId, "personal");
            if (personalCreds) {
              return next({
                context: {
                  ...context,
                  baseUrl: personalCreds.tunnelUrl,
                  apiToken: personalCreds.apiToken,
                },
              });
            }
          }

          // Priority 3: platform default
          const platformCreds = await lookupCredentialsByScope(
            s.db,
            "__platform_default__",
            "platform",
          );
          if (platformCreds) {
            return next({
              context: {
                ...context,
                baseUrl: platformCreds.tunnelUrl,
                apiToken: platformCreds.apiToken,
              },
            });
          }
        } catch (e) {
          console.error("[auth] DB credential lookup error:", e);
        }
      }

      // Priority 4: host secrets with full config
      if (s.secrets?.IRONCLAW_BASE_URL && s.secrets?.IRONCLAW_API_TOKEN) {
        const tenantId = organizationId ?? userId ?? apiKey?.userId;
        const sessionToken = tenantId
          ? await mintAccessSession(
              s.secrets.IRONCLAW_BASE_URL,
              s.secrets.IRONCLAW_API_TOKEN,
              tenantId,
            )
          : s.secrets.IRONCLAW_API_TOKEN;
        return next({
          context: { ...context, baseUrl: s.secrets.IRONCLAW_BASE_URL, apiToken: sessionToken },
        });
      }

      // Priority 5: host secrets with base URL only (plugin resolves token itself)
      if (s.secrets?.IRONCLAW_BASE_URL) {
        return next({ context });
      }

      return next({ context });
    });

    const ic = {
      credentials: resolveCredentials,
    };

    return {
      ping: builder.ping.handler(async () => ({
        status: "ok" as const,
        timestamp: new Date().toISOString(),
      })),

      hackathon: {
        register: builder.hackathon.register
          .use(requireAuth)
          .handler(async ({ input, context }) => {
            const db = (services as any).db;

            const existing = await db
              .select({ count: count() })
              .from(registrations)
              .where(eq(registrations.agentId, input.agentId));

            if (existing[0]?.count > 0) {
              throw new ORPCError("BAD_REQUEST", {
                message: `Agent "${input.agentId}" is already registered`,
              });
            }

            await db.insert(registrations).values({
              id: generateId(),
              agentId: input.agentId,
              participantName: input.participantName,
              novaAccountId: input.novaAccountId,
              userId: context.userId!,
            });

            return {
              success: true,
              message: `Registered "${input.agentId}" — send your NOVA account ID (${input.novaAccountId}) to hackathon staff to be added to the submission group.`,
            };
          }),

        submit: builder.hackathon.submit.use(requireAuth).handler(async ({ input, context }) => {
          const db = (services as any).db;

          const reg = await db
            .select()
            .from(registrations)
            .where(eq(registrations.agentId, input.agentId));

          if (reg.length === 0) {
            throw new ORPCError("BAD_REQUEST", {
              message: `Agent "${input.agentId}" is not registered. Register first.`,
            });
          }

          const existing = await db
            .select({ count: count() })
            .from(submissions)
            .where(eq(submissions.agentId, input.agentId));

          if (existing[0]?.count > 0) {
            throw new ORPCError("BAD_REQUEST", {
              message: `Already submitted for "${input.agentId}". You can only submit once.`,
            });
          }

          await db.insert(submissions).values({
            id: generateId(),
            agentId: input.agentId,
            userId: context.userId!,
            projectTitle: input.projectTitle,
            description: input.description,
            demoUrl: input.demoUrl,
            githubUrl: input.githubUrl,
            skillsList: input.skillsList,
            demoNotes: input.demoNotes,
            cid: "",
          });

          return {
            success: true,
            cid: "pending-upload",
            message: `Submission for "${input.agentId}" recorded. Use the nova-submit extension to upload your encrypted submission file.`,
          };
        }),

        leaderboard: builder.hackathon.leaderboard.handler(async () => {
          const db = (services as any).db;

          const results = await db
            .select({
              agentId: submissions.agentId,
              projectTitle: submissions.projectTitle,
              userId: submissions.userId,
              createdAt: submissions.createdAt,
            })
            .from(submissions)
            .orderBy(desc(submissions.createdAt));

          const participantNames = await db
            .select({
              agentId: registrations.agentId,
              participantName: registrations.participantName,
            })
            .from(registrations);

          const nameMap = new Map(participantNames.map((r: any) => [r.agentId, r.participantName]));

          return {
            entries: results.map((r: any) => ({
              agentId: r.agentId,
              participantName: nameMap.get(r.agentId) ?? r.agentId,
              projectTitle: r.projectTitle,
              submittedAt:
                r.createdAt instanceof Date ? r.createdAt.toISOString() : String(r.createdAt),
            })),
          };
        }),
      },

      ironclaw: {
        ping: builder.ironclaw.ping.use(ic.credentials).handler(h0(services, (ic) => ic.ping)),

        session: builder.ironclaw.session
          .use(requireAuth)
          .use(ic.credentials)
          .handler(h0(services, (ic) => ic.session)),

        settings: {
          get: builder.ironclaw.settings.get
            .use(requireAuth)
            .handler(async ({ input, context }) => {
              const scope = input?.scope ?? "personal";
              const { userId, organizationId } = context;
              const tenantId = resolveTenantForScope(scope, userId, organizationId);
              if (!tenantId) {
                throw new ORPCError("UNAUTHORIZED", { message: "No tenant or user context" });
              }
              if (scope === "organization" && !organizationId) {
                throw new ORPCError("BAD_REQUEST", { message: "No active organization" });
              }
              if (scope === "platform" && context.user?.role !== "admin") {
                throw new ORPCError("FORBIDDEN", { message: "Admin access required" });
              }
              const db = s.db;
              const creds = await lookupCredentialsByScope(db, tenantId, scope);
              if (!creds) {
                throw new ORPCError("NOT_FOUND", { message: "No ironclaw settings configured" });
              }
              return {
                tunnelUrl: creds.tunnelUrl,
                apiToken: "",
                hasToken: true,
                scope,
              };
            }),

          update: builder.ironclaw.settings.update
            .use(requireAuth)
            .handler(async ({ input, context }) => {
              const scope = input.scope;
              const { userId, organizationId } = context;
              const tenantId = resolveTenantForScope(scope, userId, organizationId);
              if (!tenantId) {
                throw new ORPCError("UNAUTHORIZED", { message: "No tenant or user context" });
              }
              if (scope === "organization" && !organizationId) {
                throw new ORPCError("BAD_REQUEST", { message: "No active organization" });
              }
              if (scope === "platform" && context.user?.role !== "admin") {
                throw new ORPCError("FORBIDDEN", { message: "Admin access required" });
              }
              const db = s.db;

              let apiToken = input.apiToken;
              if (!apiToken) {
                const existing = await lookupCredentialsByScope(db, tenantId, scope);
                if (existing) apiToken = existing.apiToken;
              }
              if (!apiToken) {
                throw new ORPCError("BAD_REQUEST", {
                  message: "API token is required when no token is already configured",
                });
              }

              const connectionId = `conn_${tenantId}_${scope}`;
              await db
                .insert(ironclawConnections)
                .values({
                  id: connectionId,
                  name: `${scope} connection for ${tenantId}`,
                  tunnelUrl: input.tunnelUrl,
                  apiToken,
                  createdBy: userId,
                })
                .onConflictDoUpdate({
                  target: ironclawConnections.id,
                  set: {
                    tunnelUrl: input.tunnelUrl,
                    apiToken,
                    updatedBy: userId,
                  },
                });
              await db
                .insert(ironclawScopeBindings)
                .values({
                  id: `sb_${tenantId}_${scope}`,
                  tenantId,
                  scopeType: scope,
                  connectionId,
                  createdBy: userId,
                })
                .onConflictDoUpdate({
                  target: ironclawScopeBindings.id,
                  set: { connectionId },
                });
              return { success: true };
            }),

          delete: builder.ironclaw.settings.delete
            .use(requireAuth)
            .handler(async ({ input, context }) => {
              const scope = input?.scope ?? "personal";
              const { userId, organizationId } = context;
              const tenantId = resolveTenantForScope(scope, userId, organizationId);
              if (!tenantId) {
                throw new ORPCError("UNAUTHORIZED", { message: "No tenant or user context" });
              }
              if (scope === "organization" && !organizationId) {
                throw new ORPCError("BAD_REQUEST", { message: "No active organization" });
              }
              if (scope === "platform" && context.user?.role !== "admin") {
                throw new ORPCError("FORBIDDEN", { message: "Admin access required" });
              }
              const db = s.db;
              const creds = await lookupCredentialsByScope(db, tenantId, scope);
              if (creds) {
                await db
                  .delete(ironclawConnections)
                  .where(eq(ironclawConnections.id, creds.connectionId));
              }
              await db
                .delete(ironclawScopeBindings)
                .where(
                  and(
                    eq(ironclawScopeBindings.tenantId, tenantId),
                    eq(ironclawScopeBindings.scopeType, scope),
                  ),
                );
              return { success: true };
            }),

          getMode: builder.ironclaw.settings.getMode
            .use(requireAuth)
            .handler(async ({ context }) => {
              const uid = context.userId!;
              const db = s.db;
              const rows = await db
                .select()
                .from(userPreferences)
                .where(eq(userPreferences.userId, uid));
              return { mode: (rows[0]?.ironclawMode as "auto" | "hosted" | "local") ?? "auto" };
            }),

          setMode: builder.ironclaw.settings.setMode
            .use(requireAuth)
            .handler(async ({ input, context }) => {
              const uid = context.userId!;
              const db = s.db;
              await db
                .insert(userPreferences)
                .values({
                  userId: uid,
                  ironclawMode: input.mode,
                })
                .onConflictDoUpdate({
                  target: userPreferences.userId,
                  set: { ironclawMode: input.mode },
                });
              return { success: true };
            }),
        },

        threads: {
          list: builder.ironclaw.threads.list
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.list)),
          create: builder.ironclaw.threads.create
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.create)),
          delete: builder.ironclaw.threads.delete
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.delete)),
          sendMessage: builder.ironclaw.threads.sendMessage
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.sendMessage)),
          getTimeline: builder.ironclaw.threads.getTimeline
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.getTimeline)),
          cancelRun: builder.ironclaw.threads.cancelRun
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.cancelRun)),
          resolveGate: builder.ironclaw.threads.resolveGate
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.resolveGate)),
          streamEvents: builder.ironclaw.threads.streamEvents
            .use(requireAuth)
            .use(ic.credentials)
            .handler(hStream(services, (ic) => ic.threads.streamEvents)),
          getState: builder.ironclaw.threads.getState
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.threads.getState)),
        },

        automations: {
          list: builder.ironclaw.automations.list
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.automations.list)),
        },

        outbound: {
          getPreferences: builder.ironclaw.outbound.getPreferences
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.outbound.getPreferences)),
          setPreferences: builder.ironclaw.outbound.setPreferences
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.outbound.setPreferences)),
          listTargets: builder.ironclaw.outbound.listTargets
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.outbound.listTargets)),
        },

        extensions: {
          list: builder.ironclaw.extensions.list
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.extensions.list)),
          listRegistry: builder.ironclaw.extensions.listRegistry
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.extensions.listRegistry)),
          install: builder.ironclaw.extensions.install
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.extensions.install)),
          activate: builder.ironclaw.extensions.activate
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.extensions.activate)),
          remove: builder.ironclaw.extensions.remove
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.extensions.remove)),
          getSetup: builder.ironclaw.extensions.getSetup
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.extensions.getSetup)),
          setup: builder.ironclaw.extensions.setup
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.extensions.setup)),
        },

        skills: {
          list: builder.ironclaw.skills.list
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.skills.list)),
          search: builder.ironclaw.skills.search
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.skills.search)),
          install: builder.ironclaw.skills.install
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.skills.install)),
          get: builder.ironclaw.skills.get
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.skills.get)),
          update: builder.ironclaw.skills.update
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.skills.update)),
          remove: builder.ironclaw.skills.remove
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.skills.remove)),
        },

        channels: {
          listConnectable: builder.ironclaw.channels.listConnectable
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.channels.listConnectable)),
        },

        auth: {
          listProviders: builder.ironclaw.auth.listProviders
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.auth.listProviders)),
          exchangeLoginTicket: builder.ironclaw.auth.exchangeLoginTicket
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.auth.exchangeLoginTicket)),
          logout: builder.ironclaw.auth.logout
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h0(services, (ic) => ic.auth.logout)),
        },

        operator: {
          createAccessSession: builder.ironclaw.operator.createAccessSession
            .use(requireAuth)
            .use(ic.credentials)
            .handler(h1(services, (ic) => ic.operator.createAccessSession)),
        },
      },

      conversation: {
        listThreads: builder.conversation.listThreads
          .use(requireAuth)
          .use(ic.credentials)
          .handler(async ({ context }: any) => {
            const ic = s.ironclaw(context);
            const raw = await ic.threads.list({ limit: 50 });
            return { data: (raw.data ?? []).map(normalizeThread) };
          }),

        getMessages: builder.conversation.getMessages
          .use(requireAuth)
          .use(ic.credentials)
          .handler(async ({ input, context }: any) => {
            const ic = s.ironclaw(context);
            const raw = await ic.threads.getTimeline({
              id: input.threadId,
              limit: input.limit ?? 100,
              cursor: input.cursor,
            });
            return normalizeTimelinePage(raw, input.threadId);
          }),

        sendMessage: builder.conversation.sendMessage
          .use(requireAuth)
          .use(ic.credentials)
          .handler(async ({ input, context }: any) => {
            const ic = s.ironclaw(context);
            const raw = await ic.threads.sendMessage({
              id: input.threadId,
              content: input.content,
              clientActionId: input.clientActionId,
              attachments: input.attachments,
            });
            return {
              threadId: input.threadId,
              runId: raw.runId ?? raw.activeRunId ?? undefined,
              outcome: raw.outcome,
              status: raw.status,
              activeRunId: raw.activeRunId,
              acceptedMessageRef: raw.acceptedMessageRef ?? "",
              pendingMessageId: `pending-${crypto.randomUUID()}`,
              submittedAt: new Date().toISOString(),
              eventCursor: raw.eventCursor ?? undefined,
            };
          }),

        live: builder.conversation.live.use(requireAuth).handler(async () => {
          throw new ORPCError("BAD_REQUEST", {
            message: "The live endpoint is deprecated. Use conversation.threadChat instead.",
          });
        }),

        threadChat: builder.conversation.threadChat
          .use(requireAuth)
          .use(ic.credentials)
          .handler(createThreadChatBridge(s)),

        threadApprove: builder.conversation.threadApprove
          .use(requireAuth)
          .use(ic.credentials)
          .handler(async ({ input, context }: any) => {
            const ic = s.ironclaw(context);
            await ic.threads.resolveGate({
              id: input.threadId,
              runId: input.runId,
              gateRef: input.gateRef,
              resolution: input.approved ? "approved" : "denied",
            });
            return { success: true };
          }),
      },
    };
  },
});
