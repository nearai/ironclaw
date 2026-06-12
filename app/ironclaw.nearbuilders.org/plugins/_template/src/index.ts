import { createPlugin } from "every-plugin";
import { Effect } from "every-plugin/effect";
import { getEventMeta, MemoryPublisher, ORPCError } from "every-plugin/orpc";
import { z } from "every-plugin/zod";

import { contract } from "./contract";
import type { PluginsClient } from "./plugins-client.gen";
import { TemplateService } from "./service";

type BackgroundEvents = {
  "background-updates": {
    id: string;
    index: number;
    timestamp: number;
  };
};

/**
 * Template Plugin - Demonstrates core plugin patterns.
 *
 * Shows how to:
 * - Initialize a simple service
 * - Implement single fetch and streaming procedures
 * - Handle errors with CommonPluginErrors
 */
export default createPlugin.withPlugins<PluginsClient>()({
  variables: z.object({
    baseUrl: z.url().default("https://api.example.com"),
    timeout: z.number().min(1000).max(60000).default(10000),
    backgroundEnabled: z.boolean().default(false), // Enable background event broadcasting
    backgroundIntervalMs: z.number().min(50).max(60000).default(30000), // Background poll interval
  }),

  secrets: z.object({
    apiKey: z.string().min(1, "API key is required").default("template-dev-key"),
  }),

  context: z.object({
    userId: z.string().optional(),
    sessionId: z.string().optional(),
    reqHeaders: z.custom<Headers>().optional(),
    getRawBody: z.custom<() => Promise<string>>().optional(),
  }),

  contract, // START HERE: define your oRPC contract in ./contract

  initialize: (config) =>
    Effect.gen(function* () {
      // Create service instance with config
      const service = new TemplateService(
        config.variables.baseUrl,
        config.secrets.apiKey,
        config.variables.timeout,
      );

      // Test the connection during initialization
      yield* service.ping();

      // Create publisher for broadcasting background events
      const publisher = new MemoryPublisher<BackgroundEvents>({
        resumeRetentionSeconds: 60 * 2, // Retain events for 2 minutes to support resume
      });

      // Start background producer if enabled
      if (config.variables.backgroundEnabled) {
        yield* Effect.forkScoped(
          Effect.gen(function* () {
            let i = 0;
            while (true) {
              i++;
              // Simulate polling external API
              const event = {
                id: `bg-${i}`,
                index: i,
                timestamp: Date.now(),
              };

              // Publish to all subscribers
              yield* Effect.tryPromise(() => publisher.publish("background-updates", event)).pipe(
                Effect.catchAll((error) => {
                  console.log(`[TemplatePlugin] Publish failed for event ${i}:`, error);
                  return Effect.void;
                }),
              );

              // Wait before next poll
              yield* Effect.sleep(`${config.variables.backgroundIntervalMs} millis`);
            }
          }),
        );
      }

      return { service, publisher }; // This is context for "createRouter"
    }),

  shutdown: () => Effect.void,

  createRouter: (context, builder) => {
    // context: { service, publisher } from initialize
    // builder is pre-configured from oRPC: implement(contract).$context<TContext>()
    const { service, publisher } = context;

    // Middleware for authentication
    const requireAuth = builder.middleware(async ({ context, next }) => {
      if (!context.userId) {
        throw new ORPCError("UNAUTHORIZED", { message: "User ID required" });
      }
      return next({ context: { ...context, userId: context.userId } });
    });

    return {
      getById: builder.getById.use(requireAuth).handler(async ({ input, context }) => {
        try {
          const item = await Effect.runPromise(service.getById(input.id));
          return { item, userId: context.userId };
        } catch (error) {
          if (error instanceof Error && error.message.includes("Item not found")) {
            throw new ORPCError("NOT_FOUND", { message: "Failed to fetch item: Item not found" });
          }
          throw error;
        }
      }),

      search: builder.search.handler(async function* ({ input }) {
        const generator = await Effect.runPromise(service.search(input.query, input.limit));

        for await (const result of generator) {
          yield result;
        }
      }),

      ping: builder.ping.handler(async () => {
        return await Effect.runPromise(service.ping());
      }),

      // Background streaming with resume support
      listenBackground: builder.listenBackground.handler(async function* ({
        input,
        signal,
        lastEventId,
      }) {
        let count = 0;
        const maxResults = input.maxResults;
        const iterator = publisher.subscribe("background-updates", { signal, lastEventId });

        for await (const event of iterator) {
          if (maxResults && count >= maxResults) break;

          // Access optional event metadata (id, retry, comments) for debugging/monitoring
          const meta = getEventMeta(event);
          if (meta?.id) {
            console.log(`[Event] ID: ${meta.id}, Retry: ${meta.retry}ms`);
          }

          yield event;
          count++;
        }
      }),

      // Manual background event publishing
      enqueueBackground: builder.enqueueBackground.handler(async ({ input }) => {
        const event = {
          id: input.id || `manual-${Date.now()}`,
          index: -1,
          timestamp: Date.now(),
        };

        await publisher.publish("background-updates", event);
        return { ok: true };
      }),
    };
  },
});
