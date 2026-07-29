/**
 * Minimal zero-dependency static file server used by the Playwright e2e
 * suite to serve the built Storybook (`packages/ui/storybook-static`) and
 * the demo-mode SPA build (`dist-demo`).
 *
 * `vite preview` is unsuitable for the SPA case because it inherits the dev
 * `server.proxy` config, which forwards /assets and /vendor to the (absent)
 * Rust backend on :3000 — so we serve the build output directly instead.
 *
 * Usage: node --experimental-strip-types scripts/serve-static.ts <dir> <port> [--spa]
 *   --spa  serve index.html for extensionless paths (history-API fallback)
 */
import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";

const [, , dirArg, portArg, ...flags] = process.argv;
if (!dirArg || !portArg) {
  console.error("usage: serve-static.ts <dir> <port> [--spa]");
  process.exit(1);
}

const spaFallback = flags.includes("--spa");

const root = resolve(dirArg);
const port = Number(portArg);

const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".gif": "image/gif",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".map": "application/json",
  ".txt": "text/plain; charset=utf-8",
};

const server = createServer((req, res) => {
  const urlPath = decodeURIComponent((req.url ?? "/").split("?")[0]);
  // Normalize and confine to the served root.
  let filePath = normalize(join(root, urlPath));
  if (!filePath.startsWith(root)) {
    res.writeHead(403);
    res.end("forbidden");
    return;
  }
  if (existsSync(filePath) && statSync(filePath).isDirectory()) {
    filePath = join(filePath, "index.html");
  }
  if (!existsSync(filePath)) {
    if (spaFallback && extname(urlPath) === "") {
      filePath = join(root, "index.html");
    } else {
      res.writeHead(404);
      res.end("not found");
      return;
    }
  }
  res.writeHead(200, {
    "content-type": MIME[extname(filePath).toLowerCase()] ?? "application/octet-stream",
    "cache-control": "no-store",
  });
  createReadStream(filePath).pipe(res);
});

server.listen(port, "127.0.0.1", () => {
  console.log(`serving ${root} at http://127.0.0.1:${port}`);
});
