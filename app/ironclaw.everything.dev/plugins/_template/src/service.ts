import { Effect } from "every-plugin/effect";
import type { z } from "every-plugin/zod";

// Import types from contract
import type { ItemSchema, SearchResultSchema } from "./contract";

// Infer the types from the schemas
type Item = z.infer<typeof ItemSchema>;
type SearchResult = z.infer<typeof SearchResultSchema>;

/**
 * Template Service - Wraps external API calls with Effect-based error handling.
 */
export class TemplateService {
  constructor(
    private readonly baseUrl: string,
    readonly _apiKey: string,
    private readonly timeout: number,
  ) {}

  getById(id: string) {
    return Effect.tryPromise({
      try: async () => {
        // In a real plugin, use this.baseUrl, this.apiKey, this.timeout
        console.log(
          `[TemplateService] Fetching from ${this.baseUrl} with timeout ${this.timeout}ms`,
        );

        // Simulate API call
        await new Promise((resolve) => setTimeout(resolve, 50));

        if (id === "not-found") {
          throw new Error("Item not found");
        }

        return {
          id,
          title: `Item ${id}`,
          createdAt: new Date().toISOString(),
        } satisfies Item;
      },
      catch: (error: unknown) =>
        new Error(
          `Failed to fetch item: ${error instanceof Error ? error.message : String(error)}`,
        ),
    });
  }

  search(query: string, limit: number) {
    return Effect.gen(function* () {
      // Simulate API call
      yield* Effect.sleep("100 millis");

      // Mock streaming search results
      const generator: AsyncGenerator<SearchResult> = (async function* () {
        for (let i = 0; i < limit; i++) {
          yield {
            item: {
              id: `${query}-${i}`,
              title: `${query} result ${i + 1}`,
              createdAt: new Date(Date.now() - i * 24 * 60 * 60 * 1000).toISOString(),
            },
            score: Math.max(0.1, 1 - i * 0.1),
          };
        }
      })();

      return generator;
    });
  }

  ping() {
    return Effect.tryPromise({
      try: async () => {
        await new Promise((resolve) => setTimeout(resolve, 10));
        return {
          status: "ok" as const,
          timestamp: new Date().toISOString(),
        };
      },
      catch: (error: unknown) =>
        new Error(`Health check failed: ${error instanceof Error ? error.message : String(error)}`),
    });
  }
}
