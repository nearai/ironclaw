import http from "node:http";

const SMOKE_REPLY = "real-stack smoke reply verified. Hello! I'm your assistant!";

export async function startMockLlm(): Promise<{
  baseUrl: string;
  port: number;
  stop: () => Promise<void>;
}> {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", `http://${req.headers.host}`);
    const pathname = url.pathname;

    if (pathname === "/v1/models" && req.method === "GET") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ data: [{ id: "mock-model", object: "model" }] }));
      return;
    }

    if (pathname === "/v1/chat/completions" && req.method === "POST") {
      let body = "";
      req.on("data", (chunk) => (body += chunk));
      req.on("end", () => {
        const parsed = JSON.parse(body);
        const isStreaming = parsed.stream === true;

        if (isStreaming) {
          res.writeHead(200, {
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
            Connection: "keep-alive",
          });
          res.write(
            `data: ${JSON.stringify({ id: "mock-cid", object: "chat.completion.chunk", model: "mock-model", choices: [{ delta: { role: "assistant" }, index: 0 }] })}\n\n`,
          );
          res.write(
            `data: ${JSON.stringify({ id: "mock-cid", object: "chat.completion.chunk", model: "mock-model", choices: [{ delta: { content: SMOKE_REPLY }, index: 0 }] })}\n\n`,
          );
          res.write("data: [DONE]\n\n");
          res.end();
        } else {
          res.writeHead(200, { "Content-Type": "application/json" });
          res.end(
            JSON.stringify({
              id: "mock-cid",
              object: "chat.completion",
              model: "mock-model",
              choices: [{ message: { role: "assistant", content: SMOKE_REPLY }, index: 0 }],
            }),
          );
        }
      });
      return;
    }

    res.writeHead(404);
    res.end("Not found");
  });

  server.listen(0, "127.0.0.1");
  await new Promise<void>((resolve) => server.on("listening", () => resolve()));

  const assignedPort = (server.address() as any).port;
  const baseUrl = `http://127.0.0.1:${assignedPort}`;
  return {
    baseUrl,
    port: assignedPort,
    stop: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}
