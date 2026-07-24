import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

interface ManifestChunk {
  file: string;
  imports?: string[];
}

type Manifest = Record<string, ManifestChunk>;

const here = dirname(fileURLToPath(import.meta.url));
const distDir = resolve(here, "..", "dist");
const manifestPath = resolve(distDir, ".vite", "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Manifest;

const LOGIN_GZIP_BUDGET = 180_000;
const CHAT_GZIP_BUDGET = 280_000;
const CHUNK_RAW_BUDGET = 500_000;

function importClosure(roots: string[]): Set<string> {
  const visited = new Set<string>();

  function visit(key: string) {
    if (visited.has(key)) return;
    const chunk = manifest[key];
    if (!chunk) {
      throw new Error(`Vite manifest is missing expected chunk: ${key}`);
    }

    visited.add(key);
    for (const dependency of chunk.imports ?? []) {
      visit(dependency);
    }
  }

  for (const root of roots) {
    visit(root);
  }
  return visited;
}

function javascriptFiles(keys: Iterable<string>): Set<string> {
  const files = new Set<string>();
  for (const key of keys) {
    const file = manifest[key]?.file;
    if (file?.endsWith(".js")) files.add(file);
  }
  return files;
}

function gzipBytes(files: Iterable<string>): number {
  let total = 0;
  for (const file of files) {
    total += gzipSync(readFileSync(resolve(distDir, file))).byteLength;
  }
  return total;
}

function assertAtMost(label: string, actual: number, budget: number) {
  if (actual > budget) {
    throw new Error(
      `${label} is ${(actual / 1_000).toFixed(1)} KB, exceeding the ` +
        `${(budget / 1_000).toFixed(1)} KB budget`,
    );
  }
}

const loginFiles = javascriptFiles(importClosure(["index.html"]));
const chatFiles = javascriptFiles(
  importClosure([
    "index.html",
    "src/layout/gateway-layout.tsx",
    "src/pages/chat/chat-page.tsx",
  ]),
);
const loginGzipBytes = gzipBytes(loginFiles);
const chatGzipBytes = gzipBytes(chatFiles);

assertAtMost("Login entry JavaScript (gzip)", loginGzipBytes, LOGIN_GZIP_BUDGET);
assertAtMost("Initial /chat JavaScript (gzip)", chatGzipBytes, CHAT_GZIP_BUDGET);

const emittedJavascript = new Set(
  Object.values(manifest)
    .map(({ file }) => file)
    .filter((file) => file.endsWith(".js")),
);
let largestChunk = { file: "", bytes: 0 };
for (const file of emittedJavascript) {
  const bytes = readFileSync(resolve(distDir, file)).byteLength;
  if (bytes > largestChunk.bytes) largestChunk = { file, bytes };
  assertAtMost(`JavaScript chunk ${file} (raw)`, bytes, CHUNK_RAW_BUDGET);
}

console.log(
  [
    `Bundle budgets passed: login ${(loginGzipBytes / 1_000).toFixed(1)} KB gzip`,
    `/chat ${(chatGzipBytes / 1_000).toFixed(1)} KB gzip`,
    `largest chunk ${largestChunk.file} ${(largestChunk.bytes / 1_000).toFixed(1)} KB raw`,
  ].join("; "),
);
