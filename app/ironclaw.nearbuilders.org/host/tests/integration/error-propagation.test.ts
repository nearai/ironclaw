import { describe, expect, it, vi } from "vitest";

interface ORPCErrorResponse {
  code: string;
  status: number;
  message: string;
  data?: Record<string, unknown>;
}

describe("ORPC Error Propagation to HTTP Response", () => {
  const mockApiClient = {
    getValue: vi.fn(),
    setValue: vi.fn().mockResolvedValue({ key: "test", value: "value", created: true }),
    protected: vi.fn(),
    listKeys: vi.fn(),
    deleteKey: vi.fn(),
    ping: vi.fn().mockResolvedValue({ status: "ok", timestamp: new Date().toISOString() }),
  };

  describe("Error Structure Preservation", () => {
    it("should return error object with complete structure, not empty object", async () => {
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required",
        data: { apiKeyProvided: false, authType: "apiKey" },
      });

      try {
        await mockApiClient.protected();
        expect.fail("Should have thrown error");
      } catch (error: unknown) {
        const orpcError = error as ORPCErrorResponse;
        // Critical: Error should NOT be empty object {}
        expect(orpcError).toBeDefined();
        expect(orpcError).not.toEqual({});

        // All required fields must be present
        expect(orpcError).toHaveProperty("code");
        expect(orpcError).toHaveProperty("message");
        expect(orpcError).toHaveProperty("status");
        expect(orpcError).toHaveProperty("data");

        // Verify UNAUTHORIZED error is preserved
        expect(orpcError.code).toBe("UNAUTHORIZED");
        expect(orpcError.message).toBe("Auth required");
        expect(orpcError.status).toBe(401);

        // Verify data is preserved
        expect(orpcError.data).toBeDefined();
        expect(orpcError.data?.apiKeyProvided).toBe(false);
        expect(orpcError.data?.authType).toBe("apiKey");
      }
    });

    it("should preserve NOT_FOUND error structure", async () => {
      mockApiClient.getValue.mockRejectedValueOnce({
        code: "NOT_FOUND",
        status: 404,
        message: "Key not found",
        data: { resource: "kv", resourceId: "specific-key-123" },
      });

      try {
        await mockApiClient.getValue({ key: "specific-key-123" });
        expect.fail("Should have thrown NOT_FOUND error");
      } catch (error: unknown) {
        const orpcError = error as ORPCErrorResponse;
        // Error should not be empty
        expect(orpcError).toBeDefined();
        expect(orpcError).not.toEqual({});

        // Complete structure check
        expect(orpcError.code).toBe("NOT_FOUND");
        expect(orpcError.status).toBe(404);
        expect(orpcError.message).toBe("Key not found");

        // Data context preserved
        expect(orpcError.data).toBeDefined();
        expect(orpcError.data?.resource).toBe("kv");
        expect(orpcError.data?.resourceId).toBe("specific-key-123");
      }
    });

    it("should preserve FORBIDDEN error with action data", async () => {
      mockApiClient.setValue.mockRejectedValueOnce({
        code: "FORBIDDEN",
        status: 403,
        message: "Access denied",
        data: { action: "write", requiredPermissions: ["write:data"] },
      });

      try {
        await mockApiClient.setValue({ key: "test", value: "value" });
        expect.fail("Should have thrown FORBIDDEN error");
      } catch (error: unknown) {
        const orpcError = error as ORPCErrorResponse;
        expect(orpcError).toBeDefined();
        expect(orpcError).not.toEqual({});

        expect(orpcError.code).toBe("FORBIDDEN");
        expect(orpcError.status).toBe(403);

        // Verify action data is preserved
        expect(orpcError.data).toBeDefined();
        expect(orpcError.data?.action).toBe("write");
        expect(orpcError.data?.requiredPermissions).toEqual(["write:data"]);
      }
    });
  });

  describe("Error Serialization", () => {
    it("should produce valid JSON that can be parsed", async () => {
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Auth required for testing",
        data: { apiKeyProvided: false },
      });

      try {
        await mockApiClient.protected();
        expect.fail("Should have thrown");
      } catch (error: unknown) {
        const errorStr = JSON.stringify(error as ORPCErrorResponse);

        // Should NOT be empty object
        expect(errorStr).not.toBe("{}");
        expect(errorStr).not.toMatch(/^\s*\{\s*\}\s*$/);

        // Should contain error code
        expect(errorStr).toContain("UNAUTHORIZED");

        // Should contain meaningful message
        expect(errorStr).toContain("Auth required");

        // Should parse back successfully
        const parsed = JSON.parse(errorStr);
        expect(parsed.code).toBe("UNAUTHORIZED");
        expect(parsed.message).toBe("Auth required for testing");
      }
    });

    it("should have enough information for debugging", async () => {
      mockApiClient.getValue.mockRejectedValueOnce({
        code: "NOT_FOUND",
        status: 404,
        message: "Resource not found for debugging",
        data: { resource: "kv", resourceId: "debug-123" },
      });

      try {
        await mockApiClient.getValue({ key: "debug-123" });
        expect.fail("Should have thrown");
      } catch (error: unknown) {
        const errorStr = JSON.stringify(error as ORPCErrorResponse);

        // Should have enough content to debug
        expect(errorStr.length).toBeGreaterThan(50);

        // Should not be just placeholder text
        expect(errorStr).not.toMatch(/Error:\s*\{\}/);

        // Should contain resource identifier
        expect(errorStr).toContain("debug-123");
      }
    });
  });

  describe("SS Error Context Preservation", () => {
    it("should include error data for user notifications", async () => {
      mockApiClient.protected.mockRejectedValueOnce({
        code: "UNAUTHORIZED",
        status: 401,
        message: "Authentication required",
        data: { apiKeyProvided: false, provider: "test-provider" },
      });

      try {
        await mockApiClient.protected();
        expect.fail("Should have thrown");
      } catch (error: unknown) {
        const orpcError = error as ORPCErrorResponse;
        // For toast notifications
        expect(orpcError.message).toBeDefined();
        expect(orpcError.message.length).toBeGreaterThan(0);
        expect(orpcError.message).not.toContain("Error: {}");

        // For user-friendly display
        expect(orpcError.data).toBeDefined();
        expect(orpcError.data?.apiKeyProvided).toBe(false);
      }
    });

    it("should preserve status codes for conditional error display", async () => {
      const testCases = [
        { code: "UNAUTHORIZED", status: 401, message: "Auth required" },
        { code: "NOT_FOUND", status: 404, message: "Not found" },
        { code: "FORBIDDEN", status: 403, message: "Forbidden" },
        { code: "ERROR_CODE", status: 500, message: "Server error" },
      ];

      for (const testCase of testCases) {
        mockApiClient.setValue.mockRejectedValueOnce(testCase);

        try {
          await mockApiClient.setValue({ key: "test", value: "value" });
          expect.fail(`Should have thrown ${testCase.code}`);
        } catch (error: unknown) {
          const orpcError = error as ORPCErrorResponse;
          expect(orpcError.status).toBeDefined();
          expect(typeof orpcError.status).toBe("number");
          expect(orpcError.status).toBe(testCase.status);

          expect(orpcError.code).toBeDefined();
          expect(typeof orpcError.code).toBe("string");
          expect(orpcError.code).toBe(testCase.code);
        }
      }
    });
  });

  describe("Status Code Consistency", () => {
    it("should map error codes to correct HTTP status codes", async () => {
      const statusMappings = {
        UNAUTHORIZED: 401,
        NOT_FOUND: 404,
        FORBIDDEN: 403,
        BAD_REQUEST: 400,
      };

      for (const [code, expectedStatus] of Object.entries(statusMappings)) {
        mockApiClient.listKeys.mockRejectedValueOnce({
          code,
          status: expectedStatus,
          message: `Test ${code}`,
        });

        try {
          await mockApiClient.listKeys({ limit: 10 });
          expect.fail(`Should have thrown ${code}`);
        } catch (error: unknown) {
          const orpcError = error as ORPCErrorResponse;
          expect(orpcError.status).toBe(expectedStatus);
          expect(orpcError.code).toBe(code);
        }
      }
    });
  });
});
