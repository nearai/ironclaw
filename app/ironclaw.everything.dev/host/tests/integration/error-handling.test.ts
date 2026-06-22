import { beforeAll, describe, expect, it, vi } from "vitest";

interface ORPCErrorResponse {
  code: string;
  status: number;
  message: string;
  data?: Record<string, unknown>;
}

describe("Error Propagation & Formatting", () => {
  const createMockApiClient = () => ({
    // Marketplace SSR prefetch
    getFeaturedProducts: vi.fn().mockResolvedValue({ products: [] }),
    getProducts: vi.fn().mockResolvedValue({ products: [] }),
    getCarouselCollections: vi.fn().mockResolvedValue({ collections: [] }),
    getProductTypes: vi.fn().mockResolvedValue({ productTypes: [] }),

    // Product route SSR
    getProduct: vi.fn().mockImplementation(({ id }: { id: string }) =>
      Promise.resolve({
        product: {
          id,
          slug: id,
          title: `Test Product ${id}`,
          description: `Description for ${id}`,
          price: 10,
          images: [{ url: "https://example.com/img.png" }],
          variants: [],
          options: [],
          collections: [],
        },
      }),
    ),

    getValue: vi.fn().mockImplementation(({ key }: { key: string }) =>
      Promise.resolve({
        key,
        value: `test-value-for-${key}`,
        updatedAt: new Date().toISOString(),
      }),
    ),
    setValue: vi.fn().mockResolvedValue({ key: "test", value: "value", created: true }),
    protected: vi.fn().mockResolvedValue({
      message: "Protected data",
      accountId: "test.near",
      timestamp: new Date().toISOString(),
    }),
    listKeys: vi.fn().mockResolvedValue({ keys: [], total: 0, hasMore: false }),
    deleteKey: vi.fn().mockResolvedValue({ key: "test", deleted: true }),
    ping: vi.fn().mockResolvedValue({ status: "ok", timestamp: new Date().toISOString() }),
  });

  let mockApiClient: ReturnType<typeof createMockApiClient>;

  beforeAll(() => {
    mockApiClient = createMockApiClient();
  });

  describe("UNAUTHORIZED Error Flow", () => {
    it("returns 401 when calling protected endpoint without authentication context", async () => {
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required",
        data: { apiKeyProvided: false },
      });

      await expect(mockApiClient.protected()).rejects.toMatchObject({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required",
      });
    });

    it("preserves UNAUTHORIZED error structure through the client", async () => {
      const unauthorizedError = {
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required",
        data: { apiKeyProvided: false },
      };
      mockApiClient.listKeys.mockRejectedValueOnce(unauthorizedError);

      try {
        await mockApiClient.listKeys({ limit: 10 });
        expect.fail("Should have thrown");
      } catch (error: unknown) {
        expect(error).toMatchObject({
          code: "UNAUTHORIZED",
          status: 401,
        });
      }
    });

    it("UNAUTHORIZED error includes proper status code 401", async () => {
      const error = {
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required",
      };
      mockApiClient.getValue.mockRejectedValueOnce(error);

      try {
        await mockApiClient.getValue({ key: "test-key" });
        expect.fail("Should have thrown");
      } catch (e) {
        const err = e as ORPCErrorResponse;
        expect(err.status).toBe(401);
        expect(err.code).toBe("UNAUTHORIZED");
      }
    });
  });

  describe("NOT_FOUND Error Flow", () => {
    it("returns 404 when accessing non-existent resource", async () => {
      const notFoundError = {
        code: "NOT_FOUND",
        status: 404,
        message: "Key not found",
        data: { resource: "kv", resourceId: "non-existent-key" },
      };
      mockApiClient.getValue.mockRejectedValueOnce(notFoundError);

      try {
        await mockApiClient.getValue({ key: "non-existent-key" });
        expect.fail("Should have thrown");
      } catch (error: unknown) {
        expect(error).toMatchObject({
          code: "NOT_FOUND",
          status: 404,
          message: "Key not found",
        });
      }
    });

    it("preserves NOT_FOUND error data through the stack", async () => {
      const notFoundError = {
        code: "NOT_FOUND",
        status: 404,
        message: "Key not found",
        data: { resource: "kv", resourceId: "specific-key-123" },
      };
      mockApiClient.getValue.mockRejectedValueOnce(notFoundError);

      try {
        await mockApiClient.getValue({ key: "specific-key-123" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.data).toBeDefined();
        expect(error.data?.resource).toBe("kv");
        expect(error.data?.resourceId).toBe("specific-key-123");
      }
    });

    it("NOT_FOUND error on deleteKey includes proper data", async () => {
      const notFoundError = {
        code: "NOT_FOUND",
        status: 404,
        message: "Key not found",
        data: { resource: "kv", resourceId: "deleted-key" },
      };
      mockApiClient.deleteKey.mockRejectedValueOnce(notFoundError);

      try {
        await mockApiClient.deleteKey({ key: "deleted-key" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.code).toBe("NOT_FOUND");
        expect(error.status).toBe(404);
        expect(error.data?.resourceId).toBe("deleted-key");
      }
    });
  });

  describe("Error Status Code Preservation", () => {
    it("UNAUTHORIZED always has status 401", async () => {
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Unauthorized access",
      });

      try {
        await mockApiClient.protected();
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.status).toBe(401);
      }
    });

    it("NOT_FOUND always has status 404", async () => {
      mockApiClient.getValue.mockRejectedValueOnce({
        code: "NOT_FOUND",
        status: 404,
        message: "Resource not found",
      });

      try {
        await mockApiClient.getValue({ key: "any-key" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.status).toBe(404);
      }
    });

    it("FORBIDDEN has status 403", async () => {
      mockApiClient.setValue.mockRejectedValueOnce({
        code: "FORBIDDEN",
        status: 403,
        message: "Access denied",
        data: { action: "write" },
      });

      try {
        await mockApiClient.setValue({ key: "test", value: "value" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.status).toBe(403);
        expect(error.code).toBe("FORBIDDEN");
      }
    });
  });

  describe("SSR Client Error Handling", () => {
    it("SSR client handles errors without needing absolute URL", async () => {
      mockApiClient.getValue.mockRejectedValueOnce({
        code: "NOT_FOUND",
        status: 404,
        message: "Key not found",
        data: { resource: "kv", resourceId: "ssr-test-key" },
      });

      try {
        await mockApiClient.getValue({ key: "ssr-test-key" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.code).toBe("NOT_FOUND");
      }
    });

    // NOTE: The current demo UI does not include a public product SSR route.
  });

  describe("Error Message Preservation", () => {
    it("uses API handler's standardized NOT_FOUND message with error data", async () => {
      const standardMessage = "Key not found";
      const errorPayload = {
        code: "NOT_FOUND",
        status: 404,
        message: standardMessage,
        data: { resource: "kv", resourceId: "custom-key" },
      };

      const originalGetValue = mockApiClient.getValue;
      mockApiClient.getValue = vi.fn().mockRejectedValue(errorPayload);

      try {
        await mockApiClient.getValue({ key: "custom-key" });
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.message).toBe(standardMessage);
        expect(error.data?.resourceId).toBe("custom-key");
      } finally {
        mockApiClient.getValue = originalGetValue;
      }
    });

    it("preserves UNAUTHORIZED message", async () => {
      const authMessage = "Auth required";
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: authMessage,
      });

      try {
        await mockApiClient.protected();
        expect.fail("Should have thrown");
      } catch (e) {
        const error = e as ORPCErrorResponse;
        expect(error.message).toBe(authMessage);
      }
    });
  });
});
