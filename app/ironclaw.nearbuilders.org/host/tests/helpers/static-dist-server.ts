import { existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { getAvailablePort } from "./ports";

export interface StaticDistServer {
  baseUrl: string;
  stop: () => Promise<void>;
}

const MIME_TYPES: Record<string, string> = {
  ".css": "text/css",
  ".html": "text/html",
  ".ico": "image/x-icon",
  ".js": "application/javascript",
  ".json": "application/json",
  ".mjs": "application/javascript",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain",
  ".webmanifest": "application/manifest+json",
};

function getContentType(filePath: string) {
  return MIME_TYPES[path.extname(filePath).toLowerCase()] ?? "application/octet-stream";
}

export async function startStaticDistServer(rootDir: string): Promise<StaticDistServer> {
  const port = await getAvailablePort();
  const normalizedRoot = path.resolve(rootDir);

  const server = createServer((req, res) => {
    const rawPath = (req.url ?? "/").split("?")[0] ?? "/";
    const relativePath = rawPath === "/" ? "/index.html" : rawPath;
    const filePath = path.resolve(normalizedRoot, `.${relativePath}`);

    if (!filePath.startsWith(normalizedRoot)) {
      res.statusCode = 403;
      res.end("forbidden");
      return;
    }

    if (!existsSync(filePath) || statSync(filePath).isDirectory()) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }

    res.statusCode = 200;
    res.setHeader("access-control-allow-origin", "*");
    res.setHeader("content-type", getContentType(filePath));
    res.end(readFileSync(filePath));
  });

  await new Promise<void>((resolve, reject) => {
    server.on("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });

  return {
    baseUrl: `http://127.0.0.1:${port}`,
    stop: async () => {
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      });
    },
  };
}
