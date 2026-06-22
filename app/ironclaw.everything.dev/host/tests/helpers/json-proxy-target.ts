import { createServer } from "node:http";
import { getAvailablePort } from "./ports";

export interface JsonProxyTarget {
  baseUrl: string;
  stop: () => Promise<void>;
}

export async function startJsonProxyTarget(): Promise<JsonProxyTarget> {
  const port = await getAvailablePort();
  const server = createServer((req, res) => {
    if (req.url === "/api/ping") {
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ status: "ok", proxied: true }));
      return;
    }

    if (req.url === "/api/_health") {
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ status: "ready", proxied: true }));
      return;
    }

    res.statusCode = 404;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ error: "Not Found" }));
  });

  await new Promise<void>((resolve, reject) => {
    server.on("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });

  return {
    baseUrl: `http://127.0.0.1:${port}`,
    stop: async () => {
      await new Promise<void>((resolve, reject) => {
        server.close((error) => {
          if (error) {
            reject(error);
            return;
          }
          resolve();
        });
      });
    },
  };
}
