import { count, desc, eq } from "drizzle-orm";
import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";
import { contract } from "./contract";
import { loadMigrations } from "./db/load-migrations";
import { migrate } from "./db/migrator";
import { registrations, submissions, tenantCredentials } from "./db/schema";
import { createAuthMiddleware } from "./lib/auth";
import type { PluginsClient } from "./lib/plugins-types.gen";
import type { ContractRouterClient } from "@orpc/contract";
import type { ContractType as IronclawContract } from "../../plugins/ironclaw/src/contract";

function generateId(): string {
  return `hc_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

type Ic = ContractRouterClient<IronclawContract>;

const h0 = (services: { ironclaw: (ctx: any) => Ic }, select: (ic: Ic) => () => any) =>
  async ({ context }: any) => {
    const ic = services.ironclaw(context);
    return await select(ic)();
  };

const h1 = (services: { ironclaw: (ctx: any) => Ic }, select: (ic: Ic) => (input: any) => any) =>
  async ({ input, context }: any) => {
    const ic = services.ironclaw(context);
    return await select(ic)(input);
  };

const hStream = (services: { ironclaw: (ctx: any) => Ic }, select: (ic: Ic) => (input: any) => any) =>
  async function* ({ input, signal, context }: any) {
    const ic = services.ironclaw(context);
    console.log("[stream] start", { input });
    try {
      const events = await select(ic)(input);
      for await (const event of events) {
        if (signal?.aborted) break;
        yield event;
      }
      console.log("[stream] end");
    } catch (error) {
      console.error("[stream] error:", error);
      throw error;
    }
  };

export default createPlugin.withPlugins<PluginsClient>()({
  variables: z.object({}),

  secrets: z.object({
    API_DATABASE_URL: z.string().default("pglite:.bos/api/:memory:"),
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

      return { ironclaw, auth, plugins: restPlugins, db: driver.db, driver };
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
      const tenantId = context.organizationId ?? context.userId;
      if (tenantId && s.db) {
        try {
          const creds = await s.db
            .select()
            .from(tenantCredentials)
            .where(eq(tenantCredentials.tenantId, tenantId));
          if (creds.length > 0) {
            return next({
              context: {
                ...context,
                baseUrl: creds[0].tunnelUrl,
                apiToken: creds[0].apiToken,
              },
            });
          }
        } catch {
          // DB error, continue without credentials
        }
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

        submit: builder.hackathon.submit
          .use(requireAuth)
          .handler(async ({ input, context }) => {
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

          const nameMap = new Map(
            participantNames.map((r: any) => [r.agentId, r.participantName]),
          );

          return {
            entries: results.map((r: any) => ({
              agentId: r.agentId,
              participantName: nameMap.get(r.agentId) ?? r.agentId,
              projectTitle: r.projectTitle,
              submittedAt:
                r.createdAt instanceof Date
                  ? r.createdAt.toISOString()
                  : String(r.createdAt),
            })),
          };
        }),
      },

      ironclaw: {
        ping: builder.ironclaw.ping.use(ic.credentials).handler(
          h0(services, ic => ic.ping),
        ),

        session: builder.ironclaw.session.use(requireAuth).use(ic.credentials).handler(
          h0(services, ic => ic.session),
        ),

        settings: {
          get: builder.ironclaw.settings.get.use(requireAuth).handler(async ({ context }) => {
            const tenantId = context.organizationId ?? context.userId;
            if (!tenantId) {
              throw new ORPCError("UNAUTHORIZED", { message: "No tenant or user context" });
            }
            const db = s.db;
            const creds = await db
              .select()
              .from(tenantCredentials)
              .where(eq(tenantCredentials.tenantId, tenantId));
            if (creds.length === 0) {
              throw new ORPCError("NOT_FOUND", { message: "No ironclaw settings configured" });
            }
            return {
              tunnelUrl: creds[0].tunnelUrl,
              apiToken: creds[0].apiToken,
              updatedAt: creds[0].updatedAt?.toISOString(),
            };
          }),

          update: builder.ironclaw.settings.update.use(requireAuth).handler(async ({ input, context }) => {
            const tenantId = context.organizationId ?? context.userId;
            if (!tenantId) {
              throw new ORPCError("UNAUTHORIZED", { message: "No tenant or user context" });
            }
            const db = s.db;
            await db
              .insert(tenantCredentials)
              .values({
                tenantId,
                tunnelUrl: input.tunnelUrl,
                apiToken: input.apiToken,
                updatedBy: context.userId,
              })
              .onConflictDoUpdate({
                target: tenantCredentials.tenantId,
                set: {
                  tunnelUrl: input.tunnelUrl,
                  apiToken: input.apiToken,
                  updatedBy: context.userId,
                },
              });
            return { success: true };
          }),
        },

        threads: {
          list: builder.ironclaw.threads.list.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.list),
          ),
          create: builder.ironclaw.threads.create.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.create),
          ),
          delete: builder.ironclaw.threads.delete.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.delete),
          ),
          sendMessage: builder.ironclaw.threads.sendMessage.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.sendMessage),
          ),
          getTimeline: builder.ironclaw.threads.getTimeline.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.getTimeline),
          ),
          cancelRun: builder.ironclaw.threads.cancelRun.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.cancelRun),
          ),
          resolveGate: builder.ironclaw.threads.resolveGate.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.threads.resolveGate),
          ),
          streamEvents: builder.ironclaw.threads.streamEvents.use(requireAuth).use(ic.credentials).handler(
            hStream(services, ic => ic.threads.streamEvents),
          ),
        },

        automations: {
          list: builder.ironclaw.automations.list.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.automations.list),
          ),
        },

        outbound: {
          getPreferences: builder.ironclaw.outbound.getPreferences.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.outbound.getPreferences),
          ),
          setPreferences: builder.ironclaw.outbound.setPreferences.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.outbound.setPreferences),
          ),
          listTargets: builder.ironclaw.outbound.listTargets.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.outbound.listTargets),
          ),
        },

        extensions: {
          list: builder.ironclaw.extensions.list.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.extensions.list),
          ),
          listRegistry: builder.ironclaw.extensions.listRegistry.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.extensions.listRegistry),
          ),
          install: builder.ironclaw.extensions.install.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.extensions.install),
          ),
          activate: builder.ironclaw.extensions.activate.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.extensions.activate),
          ),
          remove: builder.ironclaw.extensions.remove.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.extensions.remove),
          ),
          getSetup: builder.ironclaw.extensions.getSetup.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.extensions.getSetup),
          ),
          setup: builder.ironclaw.extensions.setup.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.extensions.setup),
          ),
        },

        skills: {
          list: builder.ironclaw.skills.list.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.skills.list),
          ),
          search: builder.ironclaw.skills.search.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.skills.search),
          ),
          install: builder.ironclaw.skills.install.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.skills.install),
          ),
          get: builder.ironclaw.skills.get.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.skills.get),
          ),
          update: builder.ironclaw.skills.update.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.skills.update),
          ),
          remove: builder.ironclaw.skills.remove.use(requireAuth).use(ic.credentials).handler(
            h1(services, ic => ic.skills.remove),
          ),
        },

        channels: {
          listConnectable: builder.ironclaw.channels.listConnectable.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.channels.listConnectable),
          ),
        },

        auth: {
          listProviders: builder.ironclaw.auth.listProviders.use(ic.credentials).handler(
            h0(services, ic => ic.auth.listProviders),
          ),
          exchangeLoginTicket: builder.ironclaw.auth.exchangeLoginTicket.use(ic.credentials).handler(
            h1(services, ic => ic.auth.exchangeLoginTicket),
          ),
          logout: builder.ironclaw.auth.logout.use(requireAuth).use(ic.credentials).handler(
            h0(services, ic => ic.auth.logout),
          ),
        },
      },
    };
  },
});
