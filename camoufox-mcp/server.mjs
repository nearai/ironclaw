import Fastify from "fastify";
import { z } from "zod";

const app = Fastify({ logger: true });

const port = Number(process.env.CAMOUFOX_MCP_PORT || "8790");
const camoufoxBaseUrl = process.env.CAMOUFOX_TOOL_BASE_URL || "http://camoufox-tool:8788";
const protocolVersion = "2024-11-05";

const callParamsSchema = z.object({
  name: z.string().min(1),
  arguments: z.record(z.any()).optional()
});

const sessionNewArgsSchema = z.object({
  headless: z.boolean().optional(),
  timeoutMs: z.number().int().positive().optional(),
  viewport: z.object({
    width: z.number().int().positive(),
    height: z.number().int().positive()
  }).optional()
});

const sessionCloseArgsSchema = z.object({
  sessionId: z.string().uuid()
});

const sessionActionBaseSchema = z.object({
  sessionId: z.string().uuid()
});

const gotoArgsSchema = sessionActionBaseSchema.extend({
  url: z.string().url()
});

const clickArgsSchema = sessionActionBaseSchema.extend({
  selector: z.string().min(1)
});

const fillArgsSchema = sessionActionBaseSchema.extend({
  selector: z.string().min(1),
  value: z.string()
});

const pressArgsSchema = sessionActionBaseSchema.extend({
  selector: z.string().min(1),
  key: z.string().min(1)
});

const clickXYArgsSchema = sessionActionBaseSchema.extend({
  x: z.number(),
  y: z.number()
});

const waitForSelectorArgsSchema = sessionActionBaseSchema.extend({
  selector: z.string().min(1),
  timeoutMs: z.number().int().positive().optional()
});

const waitArgsSchema = sessionActionBaseSchema.extend({
  timeoutMs: z.number().int().positive()
});

const screenshotArgsSchema = sessionActionBaseSchema.extend({
  label: z.string().min(1),
  fullPage: z.boolean().optional()
});

const tools = [
  {
    name: "browser.session_new",
    description: "Start a new Camoufox browser session.",
    inputSchema: {
      type: "object",
      properties: {
        headless: { type: "boolean" },
        timeoutMs: { type: "integer", minimum: 1 },
        viewport: {
          type: "object",
          properties: {
            width: { type: "integer", minimum: 1 },
            height: { type: "integer", minimum: 1 }
          },
          required: ["width", "height"],
          additionalProperties: false
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "browser.session_close",
    description: "Close a Camoufox browser session.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" }
      },
      required: ["sessionId"],
      additionalProperties: false
    }
  },
  {
    name: "browser.goto",
    description: "Navigate the page to a URL.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        url: { type: "string", format: "uri" }
      },
      required: ["sessionId", "url"],
      additionalProperties: false
    }
  },
  {
    name: "browser.click",
    description: "Click an element using a CSS selector.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        selector: { type: "string" }
      },
      required: ["sessionId", "selector"],
      additionalProperties: false
    }
  },
  {
    name: "browser.fill",
    description: "Fill an input using a CSS selector and value.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        selector: { type: "string" },
        value: { type: "string" }
      },
      required: ["sessionId", "selector", "value"],
      additionalProperties: false
    }
  },
  {
    name: "browser.press",
    description: "Press a key on an element using a CSS selector.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        selector: { type: "string" },
        key: { type: "string" }
      },
      required: ["sessionId", "selector", "key"],
      additionalProperties: false
    }
  },
  {
    name: "browser.click_xy",
    description: "Click absolute page coordinates.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        x: { type: "number" },
        y: { type: "number" }
      },
      required: ["sessionId", "x", "y"],
      additionalProperties: false
    }
  },
  {
    name: "browser.wait_for_selector",
    description: "Wait for a selector to appear.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        selector: { type: "string" },
        timeoutMs: { type: "integer", minimum: 1 }
      },
      required: ["sessionId", "selector"],
      additionalProperties: false
    }
  },
  {
    name: "browser.wait",
    description: "Wait for a fixed number of milliseconds.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        timeoutMs: { type: "integer", minimum: 1 }
      },
      required: ["sessionId", "timeoutMs"],
      additionalProperties: false
    }
  },
  {
    name: "browser.screenshot",
    description: "Capture a screenshot and return artifact path.",
    inputSchema: {
      type: "object",
      properties: {
        sessionId: { type: "string", format: "uuid" },
        label: { type: "string" },
        fullPage: { type: "boolean" }
      },
      required: ["sessionId", "label"],
      additionalProperties: false
    }
  }
];

const jsonRpcResult = (id, result) => ({
  jsonrpc: "2.0",
  id,
  result
});

const jsonRpcError = (id, code, message, data) => ({
  jsonrpc: "2.0",
  id,
  error: {
    code,
    message,
    ...(data ? { data } : {})
  }
});

const textContent = (payload) => [{ type: "text", text: JSON.stringify(payload, null, 2) }];

const callCamoufox = async (path, init) => {
  const response = await fetch(`${camoufoxBaseUrl}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...init
  });

  const bodyText = await response.text();
  let body;
  try {
    body = bodyText ? JSON.parse(bodyText) : {};
  } catch {
    body = { raw: bodyText };
  }

  if (!response.ok) {
    return {
      ok: false,
      status: response.status,
      body
    };
  }

  return {
    ok: true,
    status: response.status,
    body
  };
};

const callAction = async (toolName, args) => {
  switch (toolName) {
    case "browser.session_new": {
      const parsed = sessionNewArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox("/session/new", {
        method: "POST",
        body: JSON.stringify(parsed.data)
      });
      return result.ok
        ? { isError: false, payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.session_close": {
      const parsed = sessionCloseArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}`, {
        method: "DELETE"
      });
      return result.ok
        ? { isError: false, payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.goto": {
      const parsed = gotoArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.goto", url: parsed.data.url })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.click": {
      const parsed = clickArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.click", selector: parsed.data.selector })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.fill": {
      const parsed = fillArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.fill", selector: parsed.data.selector, value: parsed.data.value })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.press": {
      const parsed = pressArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.press", selector: parsed.data.selector, key: parsed.data.key })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.click_xy": {
      const parsed = clickXYArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.click_xy", x: parsed.data.x, y: parsed.data.y })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.wait_for_selector": {
      const parsed = waitForSelectorArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.wait_for_selector", selector: parsed.data.selector, timeoutMs: parsed.data.timeoutMs })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.wait": {
      const parsed = waitArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.wait", timeoutMs: parsed.data.timeoutMs })
      });
      return result.ok
        ? { isError: false, payload: result.body }
        : { isError: true, payload: result };
    }

    case "browser.screenshot": {
      const parsed = screenshotArgsSchema.safeParse(args ?? {});
      if (!parsed.success) {
        return { isError: true, payload: parsed.error.flatten() };
      }
      const result = await callCamoufox(`/session/${parsed.data.sessionId}/action`, {
        method: "POST",
        body: JSON.stringify({ type: "browser.screenshot", label: parsed.data.label, fullPage: parsed.data.fullPage })
      });
      return result.ok
        ? { isError: result.body.status === "human_required", payload: result.body }
        : { isError: true, payload: result };
    }

    default:
      return {
        isError: true,
        payload: {
          message: `Unknown tool: ${toolName}`
        }
      };
  }
};

app.get("/healthz", async () => ({
  status: "ok",
  service: "camoufox-mcp",
  camoufoxBaseUrl,
  toolCount: tools.length,
  timestamp: new Date().toISOString()
}));

const handleMcpRequest = async (request, reply) => {
  const body = request.body;
  if (!body || typeof body !== "object") {
    return reply.code(400).send(jsonRpcError(0, -32600, "Invalid Request"));
  }

  const method = body.method;
  const id = Number.isInteger(body.id) ? body.id : 0;

  switch (method) {
    case "initialize":
      return reply.send(
        jsonRpcResult(id, {
          protocolVersion: protocolVersion,
          capabilities: {
            tools: { listChanged: false }
          },
          serverInfo: {
            name: "camoufox-mcp",
            version: "1.0.0"
          },
          instructions: "Use browser.session_new, then browser.* actions with the returned sessionId."
        })
      );

    case "notifications/initialized":
      return reply.send(jsonRpcResult(id, {}));

    case "tools/list":
      return reply.send(jsonRpcResult(id, { tools }));

    case "tools/call": {
      const parsed = callParamsSchema.safeParse(body.params ?? {});
      if (!parsed.success) {
        return reply.send(
          jsonRpcResult(id, {
            content: textContent(parsed.error.flatten()),
            is_error: true
          })
        );
      }

      const { name, arguments: args } = parsed.data;
      const result = await callAction(name, args ?? {});

      return reply.send(
        jsonRpcResult(id, {
          content: textContent(result.payload),
          is_error: result.isError
        })
      );
    }

    default:
      return reply.send(jsonRpcError(id, -32601, `Method not found: ${method}`));
  }
};

// Support both legacy root MCP path and explicit /mcp path.
app.post("/", handleMcpRequest);
app.post("/mcp", handleMcpRequest);

app.listen({ host: "0.0.0.0", port })
  .then(() => {
    app.log.info({ port, camoufoxBaseUrl }, "camoufox-mcp started");
  })
  .catch((error) => {
    app.log.error({ err: error }, "failed to start camoufox-mcp");
    process.exit(1);
  });
