import { Effect } from "every-plugin/effect";
import { describe, expect, it, vi } from "vitest";
import { IronclawService } from "../src/service";

describe("IronclawService", () => {
  describe("sendMessage", () => {
    it("includes attachments in POST body when provided", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch");
      const mockResponse = new Response(
        JSON.stringify({
          outcome: "submitted",
          thread_id: "t-1",
          accepted_message_ref: "ref-1",
          status: "running",
          run_id: "run-1",
          turn_id: "turn-1",
          event_cursor: 1,
        }),
        { status: 200 },
      );
      fetchSpy.mockResolvedValue(mockResponse);

      const svc = new IronclawService("http://localhost", "tok");
      await Effect.runPromise(
        svc.sendMessage("t-1", "hello", "action-1", [
          { mimeType: "image/png", filename: "img.png", dataBase64: "iVBORw0KGgo=" },
        ]),
      );

      const callUrl = fetchSpy.mock.calls[0]![0] as string;
      const callInit = fetchSpy.mock.calls[0]![1] as RequestInit;
      const body = JSON.parse(callInit.body as string) as Record<string, unknown>;

      expect(callUrl).toContain("/api/webchat/v2/threads/t-1/messages");
      expect(body.attachments).toBeDefined();
      expect(Array.isArray(body.attachments)).toBe(true);
      const atts = body.attachments as Array<Record<string, unknown>>;
      expect(atts).toHaveLength(1);
      expect(atts[0]!.mime_type).toBe("image/png");
      expect(atts[0]!.filename).toBe("img.png");
      expect(atts[0]!.data_base64).toBe("iVBORw0KGgo=");

      fetchSpy.mockRestore();
    });

    it("does not include attachments when not provided", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch");
      const mockResponse = new Response(
        JSON.stringify({
          outcome: "submitted",
          thread_id: "t-1",
          accepted_message_ref: "ref-1",
          status: "running",
          run_id: "run-1",
          turn_id: "turn-1",
          event_cursor: 1,
        }),
        { status: 200 },
      );
      fetchSpy.mockResolvedValue(mockResponse);

      const svc = new IronclawService("http://localhost", "tok");
      await Effect.runPromise(svc.sendMessage("t-1", "hello"));

      const callInit = fetchSpy.mock.calls[0]![1] as RequestInit;
      const body = JSON.parse(callInit.body as string) as Record<string, unknown>;
      expect(body.attachments).toBeUndefined();

      fetchSpy.mockRestore();
    });
  });

  describe("getSession", () => {
    it("returns attachment capabilities when present upstream", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch");
      const mockResponse = new Response(
        JSON.stringify({
          tenant_id: "tenant-1",
          user_id: "user-1",
          capabilities: {
            operator_webui_config: true,
            attachments: {
              accept: ["image/png", "image/jpeg", "application/pdf"],
              max_count: 5,
              max_file_bytes: 10_485_760,
              max_total_bytes: 52_428_800,
            },
          },
        }),
        { status: 200 },
      );
      fetchSpy.mockResolvedValue(mockResponse);

      const svc = new IronclawService("http://localhost", "tok");
      const result = await Effect.runPromise(svc.getSession());

      expect(result.capabilities.attachments).toBeDefined();
      expect(result.capabilities.attachments!.accept).toEqual([
        "image/png",
        "image/jpeg",
        "application/pdf",
      ]);
      expect(result.capabilities.attachments!.maxCount).toBe(5);
      expect(result.capabilities.attachments!.maxFileBytes).toBe(10_485_760);
      expect(result.capabilities.attachments!.maxTotalBytes).toBe(52_428_800);

      fetchSpy.mockRestore();
    });

    it("omits attachments when not present upstream", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch");
      const mockResponse = new Response(
        JSON.stringify({
          tenant_id: "tenant-1",
          user_id: "user-1",
          capabilities: { operator_webui_config: true },
        }),
        { status: 200 },
      );
      fetchSpy.mockResolvedValue(mockResponse);

      const svc = new IronclawService("http://localhost", "tok");
      const result = await Effect.runPromise(svc.getSession());

      expect(result.capabilities.attachments).toBeUndefined();

      fetchSpy.mockRestore();
    });
  });
});
