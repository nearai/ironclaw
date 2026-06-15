import { Effect } from "every-plugin/effect";
import { describe, expect, it } from "vitest";
import { TemplateService } from "@/service";

describe("TemplateService", () => {
  const service = new TemplateService("https://api.example.com", "test-api-key", 5000);

  describe("getById", () => {
    it("should fetch item by id successfully", async () => {
      const result = await Effect.runPromise(service.getById("test-123"));

      expect(result).toEqual({
        id: "test-123",
        title: "Item test-123",
        createdAt: expect.any(String),
      });
    });

    it("should handle not found error", async () => {
      await expect(Effect.runPromise(service.getById("not-found"))).rejects.toThrow(
        "Failed to fetch item: Item not found",
      );
    });
  });

  describe("search", () => {
    it("should return search results as async generator", async () => {
      const generator = await Effect.runPromise(service.search("test-query", 3));

      const results = [];
      for await (const result of generator) {
        results.push(result);
      }

      expect(results).toHaveLength(3);
      expect(results[0]).toEqual({
        item: {
          id: "test-query-0",
          title: "test-query result 1",
          createdAt: expect.any(String),
        },
        score: 1,
      });
      expect(results[1]?.score).toBe(0.9);
      expect(results[2]?.score).toBe(0.8);
    });

    it("should respect limit parameter", async () => {
      const generator = await Effect.runPromise(service.search("limited", 2));

      const results = [];
      for await (const result of generator) {
        results.push(result);
      }

      expect(results).toHaveLength(2);
    });
  });

  describe("ping", () => {
    it("should return healthy status", async () => {
      const result = await Effect.runPromise(service.ping());

      expect(result).toEqual({
        status: "ok",
        timestamp: expect.any(String),
      });
    });
  });
});
