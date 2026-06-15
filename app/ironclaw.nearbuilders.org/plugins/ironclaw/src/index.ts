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
    reqHeaders: z.custom<Headers>().optional(),
    getRawBody: z.custom<() => Promise<string>>().optional(),
  }),

  contract,

  initialize: (config) =>
    Effect.gen(function* () {
      const service = new IronclawService(
        config.variables.baseUrl.href,
        config.secrets.apiToken,
      );
      yield* service.ping();
      return { service };
    }),

  shutdown: () => Effect.void,

  createRouter: (context, builder) => {
    const { service } = context;

    const requireAuth = builder.middleware(async ({ context, next }) => {
      if (!context.userId) {
        throw new ORPCError("UNAUTHORIZED", { message: "User ID required" });
      }
      return next({ context: { ...context, userId: context.userId } });
    });

    return {
      ping: builder.ping.handler(async () => {
        return await Effect.runPromise(service.ping());
      }),

      session: builder.session.use(requireAuth).handler(async () => {
        return await Effect.runPromise(service.getSession());
      }),

      threads: {
        list: builder.threads.list.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.listThreads(input.limit, input.cursor));
        }),

        create: builder.threads.create.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.createThread());
        }),

        delete: builder.threads.delete.use(requireAuth).handler(async ({ input }) => {
          await Effect.runPromise(service.deleteThread(input.id));
          return { success: true };
        }),

        sendMessage: builder.threads.sendMessage.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(
            service.sendMessage(input.id, input.content, input.clientActionId),
          );
        }),

        getTimeline: builder.threads.getTimeline.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(
            service.getTimeline(input.id, input.limit, input.cursor),
          );
        }),

        cancelRun: builder.threads.cancelRun.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.cancelRun(input.id, input.runId));
        }),

        resolveGate: builder.threads.resolveGate.use(requireAuth).handler(async ({ input }) => {
          await Effect.runPromise(
            service.resolveGate(input.id, input.runId, input.gateRef, input.resolution, input.always),
          );
          return { success: true };
        }),

        streamEvents: builder.threads.streamEvents.use(requireAuth).handler(async function* ({
          input,
          signal,
        }) {
          const generator = service.streamEvents(input.id, input.afterCursor);

          for await (const event of generator) {
            if (signal?.aborted) break;
            yield event;
          }
        }),
      },

      automations: {
        list: builder.automations.list.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.listAutomations(input.limit, input.runLimit));
        }),
      },

      outbound: {
        getPreferences: builder.outbound.getPreferences.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.getOutboundPreferences());
        }),

        setPreferences: builder.outbound.setPreferences.use(requireAuth).handler(async ({ input }) => {
          await Effect.runPromise(service.setOutboundPreferences(input));
          return { success: true };
        }),

        listTargets: builder.outbound.listTargets.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.listOutboundTargets());
        }),
      },

      extensions: {
        list: builder.extensions.list.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.listExtensions());
        }),

        listRegistry: builder.extensions.listRegistry.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.listExtensionRegistry());
        }),

        install: builder.extensions.install.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.installExtension(input.packageRef));
        }),

        activate: builder.extensions.activate.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.activateExtension(input.name));
        }),

        remove: builder.extensions.remove.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.removeExtension(input.name));
        }),

        getSetup: builder.extensions.getSetup.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.getExtensionSetup(input.name));
        }),

        setup: builder.extensions.setup.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(
            service.setupExtension(input.name, input.action, input.payload),
          );
        }),
      },

      skills: {
        list: builder.skills.list.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.listSkills());
        }),

        search: builder.skills.search.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.searchSkills(input.query));
        }),

        install: builder.skills.install.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.installSkill(input.name, input.content));
        }),

        get: builder.skills.get.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.getSkill(input.name));
        }),

        update: builder.skills.update.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.updateSkill(input.name, input.content));
        }),

        remove: builder.skills.remove.use(requireAuth).handler(async ({ input }) => {
          return await Effect.runPromise(service.removeSkill(input.name));
        }),
      },

      channels: {
        listConnectable: builder.channels.listConnectable.use(requireAuth).handler(async () => {
          return await Effect.runPromise(service.listConnectableChannels());
        }),
      },

      auth: {
        listProviders: builder.auth.listProviders.handler(async () => {
          return await Effect.runPromise(service.listAuthProviders());
        }),

        exchangeLoginTicket: builder.auth.exchangeLoginTicket.handler(async ({ input }) => {
          return await Effect.runPromise(service.exchangeLoginTicket(input.loginTicket));
        }),

        logout: builder.auth.logout.use(requireAuth).handler(async () => {
          await Effect.runPromise(service.logout());
          return { success: true };
        }),
      },
    };
  },
});
