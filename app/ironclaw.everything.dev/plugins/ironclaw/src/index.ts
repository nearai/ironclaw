import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";

import { contract } from "./contract";
import { IronclawService, IronclawUpstreamError } from "./service";

const PLACEHOLDER_RE = /^\{\{[A-Z0-9_]+\}\}$/;

function isConfigured(value: string | undefined): value is string {
  if (!value || value.length === 0) return false;
  if (PLACEHOLDER_RE.test(value)) return false;
  return true;
}

export default createPlugin({
  variables: z.object({
    baseUrl: z.url().default("http://localhost:3001"),
  }),

  secrets: z.object({
    IRONCLAW_API_TOKEN: z.string().optional(),
    IRONCLAW_BASE_URL: z.string().optional(),
  }),

  context: z.object({
    userId: z.string().optional(),
    sessionId: z.string().optional(),
    baseUrl: z.string().optional(),
    apiToken: z.string().optional(),
    reqHeaders: z.custom<Headers>().optional(),
    getRawBody: z.custom<() => Promise<string>>().optional(),
  }),

  contract,

  initialize: (config) =>
    Effect.gen(function* () {
      return { config };
    }),

  shutdown: () => Effect.void,

  createRouter: (context, builder) => {
    const { config } = context;

    const defaultToken = isConfigured(config.secrets.IRONCLAW_API_TOKEN)
      ? config.secrets.IRONCLAW_API_TOKEN
      : undefined;

    const resolveService = (reqCtx: { baseUrl?: string; apiToken?: string }) => {
      const baseUrl =
        reqCtx.baseUrl ?? config.secrets.IRONCLAW_BASE_URL ?? config.variables.baseUrl;
      const apiToken = reqCtx.apiToken ?? defaultToken;
      if (!isConfigured(apiToken)) {
        throw new ORPCError("PRECONDITION_FAILED", {
          message:
            "IronClaw is not configured. Set the API token in Settings → IronClaw, " +
            "or provide IRONCLAW_API_TOKEN via the plugin secrets configuration.",
        });
      }
      return new IronclawService(baseUrl, apiToken);
    };

    const requireAuth = builder.middleware(async ({ context, next }) => {
      if (!context.userId) {
        throw new ORPCError("UNAUTHORIZED", { message: "User ID required" });
      }
      return next({ context: { ...context, userId: context.userId } });
    });

    const toOrpcError = (error: unknown): never => {
      if (error instanceof IronclawUpstreamError) {
        const code: any =
          {
            400: "BAD_REQUEST",
            401: "UNAUTHORIZED",
            403: "FORBIDDEN",
            404: "NOT_FOUND",
            409: "CONFLICT",
            412: "PRECONDITION_FAILED",
          }[error.status] ?? "GATEWAY_ERROR";
        throw new ORPCError(code, {
          message: error.message,
          data: {
            upstreamStatus: error.status,
            upstreamPath: error.path,
            upstreamMethod: error.method,
            upstreamBody: error.upstreamBody,
          },
        });
      }
      if (error && typeof error === "object" && "code" in error) {
        throw error;
      }
      throw new ORPCError("GATEWAY_ERROR", {
        message: error instanceof Error ? error.message : String(error),
      });
    };

    const r =
      (fn: (svc: IronclawService, ctx: any) => any) =>
      async ({ context: ctx }: any) => {
        try {
          return await fn(resolveService(ctx), ctx);
        } catch (error) {
          toOrpcError(error);
        }
      };

    const ri =
      (fn: (svc: IronclawService, input: any, ctx: any) => any) =>
      async ({ input, context: ctx }: any) => {
        try {
          return await fn(resolveService(ctx), input, ctx);
        } catch (error) {
          toOrpcError(error);
        }
      };

    const rStream = (fn: (svc: IronclawService, input: any, ctx: any) => any) =>
      async function* ({ input, signal, context: ctx }: any) {
        const svc = resolveService(ctx);
        const gen = fn(svc, input, ctx);
        try {
          for await (const event of gen) {
            if (signal?.aborted) break;
            yield event;
          }
        } catch (error) {
          toOrpcError(error);
        }
      };

    return {
      ping: builder.ping.handler(r((svc) => Effect.runPromise(svc.ping()))),

      session: builder.session
        .use(requireAuth)
        .handler(r((svc) => Effect.runPromise(svc.getSession()))),

      threads: {
        list: builder.threads.list
          .use(requireAuth)
          .handler(
            ri((svc, input) => Effect.runPromise(svc.listThreads(input.limit, input.cursor))),
          ),

        create: builder.threads.create
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.createThread(input.clientActionId)))),

        delete: builder.threads.delete.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(svc.deleteThread(input.id));
            return { success: true };
          }),
        ),

        sendMessage: builder.threads.sendMessage
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(
                svc.sendMessage(input.id, input.content, input.clientActionId, input.attachments),
              ),
            ),
          ),

        getTimeline: builder.threads.getTimeline
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(svc.getTimeline(input.id, input.limit, input.cursor)),
            ),
          ),

        cancelRun: builder.threads.cancelRun
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.cancelRun(input.id, input.runId)))),

        resolveGate: builder.threads.resolveGate.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(
              svc.resolveGate(input.id, input.runId, input.gateRef, input.resolution, input.always, input.credentialRef),
            );
            return { success: true };
          }),
        ),

        streamEvents: builder.threads.streamEvents
          .use(requireAuth)
          .handler(rStream((svc, input) => svc.streamEvents(input.id, input.afterCursor))),

        getState: builder.threads.getState
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.getThreadState(input.id)))),

        listFiles: builder.threads.listFiles
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(svc.listProjectFiles(input.id, input.path)),
            ),
          ),

        statFile: builder.threads.statFile
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(svc.statProjectFile(input.id, input.path)),
            ),
          ),

        downloadFile: builder.threads.downloadFile
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(svc.fetchFileContent(input.id, input.path)),
            ),
          ),

        getAttachment: builder.threads.getAttachment
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(
                svc.getAttachment(input.id, input.messageId, input.attachmentId),
              ),
            ),
          ),
      },

      automations: {
        list: builder.automations.list
          .use(requireAuth)
          .handler(
            ri((svc, input) => Effect.runPromise(svc.listAutomations(input.limit, input.runLimit))),
          ),
      },

      outbound: {
        getPreferences: builder.outbound.getPreferences
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.getOutboundPreferences()))),

        setPreferences: builder.outbound.setPreferences.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(svc.setOutboundPreferences(input));
            return { success: true };
          }),
        ),

        listTargets: builder.outbound.listTargets
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.listOutboundTargets()))),
      },

      extensions: {
        list: builder.extensions.list
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.listExtensions()))),

        listRegistry: builder.extensions.listRegistry
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.listExtensionRegistry()))),

        install: builder.extensions.install
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.installExtension(input.packageRef)))),

        activate: builder.extensions.activate
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.activateExtension(input.name)))),

        remove: builder.extensions.remove
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.removeExtension(input.name)))),

        getSetup: builder.extensions.getSetup
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.getExtensionSetup(input.name)))),

        setup: builder.extensions.setup
          .use(requireAuth)
          .handler(
            ri((svc, input) =>
              Effect.runPromise(svc.setupExtension(input.name, input.action, input.payload)),
            ),
          ),
      },

      skills: {
        list: builder.skills.list
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.listSkills()))),

        search: builder.skills.search
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.searchSkills(input.query)))),

        install: builder.skills.install
          .use(requireAuth)
          .handler(
            ri((svc, input) => Effect.runPromise(svc.installSkill(input.name, input.content))),
          ),

        get: builder.skills.get
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.getSkill(input.name)))),

        update: builder.skills.update
          .use(requireAuth)
          .handler(
            ri((svc, input) => Effect.runPromise(svc.updateSkill(input.name, input.content))),
          ),

        remove: builder.skills.remove
          .use(requireAuth)
          .handler(ri((svc, input) => Effect.runPromise(svc.removeSkill(input.name)))),
      },

      channels: {
        listConnectable: builder.channels.listConnectable
          .use(requireAuth)
          .handler(r((svc) => Effect.runPromise(svc.listConnectableChannels()))),
      },

      auth: {
        listProviders: builder.auth.listProviders.handler(
          r((svc) => Effect.runPromise(svc.listAuthProviders())),
        ),

        exchangeLoginTicket: builder.auth.exchangeLoginTicket.handler(
          ri((svc, input) => Effect.runPromise(svc.exchangeLoginTicket(input.loginTicket))),
        ),

        logout: builder.auth.logout.use(requireAuth).handler(
          r(async (svc) => {
            await Effect.runPromise(svc.logout());
            return { success: true };
          }),
        ),

        submitManualToken: builder.auth.submitManualToken.handler(
          ri((svc, input) =>
            Effect.runPromise(svc.submitManualToken(input)),
          ),
        ),
      },

      operator: {
        createAccessSession: builder.operator.createAccessSession.handler(
          ri(async (svc, input) => {
            return Effect.runPromise(
              svc.createAccessSession(input.tenantId, input.agentId, input.projectId),
            );
          }),
        ),
      },
    };
  },
});
