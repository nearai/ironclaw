import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";

import { contract } from "./contract";
import { IronclawService } from "./service";

export default createPlugin({
  variables: z.object({
    baseUrl: z.url().default("http://localhost:3001"),
  }),

  secrets: z.object({
    apiToken: z.string().min(1, "API token is required").default("dev-token"),
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
      const baseUrl = config.variables.baseUrl.href;
      const apiToken = config.secrets.apiToken;
      const defaultService = new IronclawService(baseUrl, apiToken);
      yield* defaultService.ping();
      return { config, defaultService };
    }),

  shutdown: () => Effect.void,

  createRouter: (context, builder) => {
    const { config, defaultService } = context;

    const resolveService = (reqCtx: {
      baseUrl?: string;
      apiToken?: string;
    }) => {
      const baseUrl = reqCtx.baseUrl ?? config.variables.baseUrl.href;
      const apiToken = reqCtx.apiToken ?? config.secrets.apiToken;
      if (baseUrl === config.variables.baseUrl.href && apiToken === config.secrets.apiToken) {
        return defaultService;
      }
      return new IronclawService(baseUrl, apiToken);
    };

    const requireAuth = builder.middleware(async ({ context, next }) => {
      if (!context.userId) {
        throw new ORPCError("UNAUTHORIZED", { message: "User ID required" });
      }
      return next({ context: { ...context, userId: context.userId } });
    });

    const r = (fn: (svc: IronclawService, ctx: any) => any) =>
      async ({ context: ctx }: any) => fn(resolveService(ctx), ctx);

    const ri = (fn: (svc: IronclawService, input: any, ctx: any) => any) =>
      async ({ input, context: ctx }: any) => fn(resolveService(ctx), input, ctx);

    const rStream = (fn: (svc: IronclawService, input: any, ctx: any) => any) =>
      async function* ({ input, signal, context: ctx }: any) {
        const svc = resolveService(ctx);
        const gen = fn(svc, input, ctx);
        for await (const event of gen) {
          if (signal?.aborted) break;
          yield event;
        }
      };

    return {
      ping: builder.ping.handler(r((svc) => Effect.runPromise(svc.ping()))),

      session: builder.session.use(requireAuth).handler(
        r((svc) => Effect.runPromise(svc.getSession())),
      ),

      threads: {
        list: builder.threads.list.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.listThreads(input.limit, input.cursor))),
        ),

        create: builder.threads.create.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.createThread())),
        ),

        delete: builder.threads.delete.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(svc.deleteThread(input.id));
            return { success: true };
          }),
        ),

        sendMessage: builder.threads.sendMessage.use(requireAuth).handler(
          ri((svc, input) =>
            Effect.runPromise(svc.sendMessage(input.id, input.content, input.clientActionId)),
          ),
        ),

        getTimeline: builder.threads.getTimeline.use(requireAuth).handler(
          ri((svc, input) =>
            Effect.runPromise(svc.getTimeline(input.id, input.limit, input.cursor)),
          ),
        ),

        cancelRun: builder.threads.cancelRun.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.cancelRun(input.id, input.runId))),
        ),

        resolveGate: builder.threads.resolveGate.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(
              svc.resolveGate(input.id, input.runId, input.gateRef, input.resolution, input.always),
            );
            return { success: true };
          }),
        ),

        streamEvents: builder.threads.streamEvents.use(requireAuth).handler(
          rStream((svc, input) => svc.streamEvents(input.id, input.afterCursor)),
        ),
      },

      automations: {
        list: builder.automations.list.use(requireAuth).handler(
          ri((svc, input) =>
            Effect.runPromise(svc.listAutomations(input.limit, input.runLimit)),
          ),
        ),
      },

      outbound: {
        getPreferences: builder.outbound.getPreferences.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.getOutboundPreferences())),
        ),

        setPreferences: builder.outbound.setPreferences.use(requireAuth).handler(
          ri(async (svc, input) => {
            await Effect.runPromise(svc.setOutboundPreferences(input));
            return { success: true };
          }),
        ),

        listTargets: builder.outbound.listTargets.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.listOutboundTargets())),
        ),
      },

      extensions: {
        list: builder.extensions.list.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.listExtensions())),
        ),

        listRegistry: builder.extensions.listRegistry.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.listExtensionRegistry())),
        ),

        install: builder.extensions.install.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.installExtension(input.packageRef))),
        ),

        activate: builder.extensions.activate.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.activateExtension(input.name))),
        ),

        remove: builder.extensions.remove.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.removeExtension(input.name))),
        ),

        getSetup: builder.extensions.getSetup.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.getExtensionSetup(input.name))),
        ),

        setup: builder.extensions.setup.use(requireAuth).handler(
          ri((svc, input) =>
            Effect.runPromise(svc.setupExtension(input.name, input.action, input.payload)),
          ),
        ),
      },

      skills: {
        list: builder.skills.list.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.listSkills())),
        ),

        search: builder.skills.search.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.searchSkills(input.query))),
        ),

        install: builder.skills.install.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.installSkill(input.name, input.content))),
        ),

        get: builder.skills.get.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.getSkill(input.name))),
        ),

        update: builder.skills.update.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.updateSkill(input.name, input.content))),
        ),

        remove: builder.skills.remove.use(requireAuth).handler(
          ri((svc, input) => Effect.runPromise(svc.removeSkill(input.name))),
        ),
      },

      channels: {
        listConnectable: builder.channels.listConnectable.use(requireAuth).handler(
          r((svc) => Effect.runPromise(svc.listConnectableChannels())),
        ),
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
      },
    };
  },
});
