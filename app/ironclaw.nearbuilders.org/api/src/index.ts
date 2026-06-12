import { count, desc, eq } from "drizzle-orm";
import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";
import { contract } from "./contract";
import { loadMigrations } from "./db/load-migrations";
import { migrate } from "./db/migrator";
import { registrations, submissions } from "./db/schema";
import { createAuthMiddleware } from "./lib/auth";
import type { PluginsClient } from "./lib/plugins-types.gen";

function generateId(): string {
  return `hc_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

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

      const { auth, ...restPlugins } = plugins;
      console.log("[API] Services Initialized");

      return { auth, plugins: restPlugins, db: driver.db, driver };
    }),

  shutdown: (services) =>
    Effect.promise(async () => {
      console.log("[API] Shutdown");
      await (services as any).driver?.close?.();
    }),

  createRouter: (services, builder) => {
    const { requireAuth } = createAuthMiddleware(builder);

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
    };
  },
});
