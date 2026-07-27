import { readFileSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
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

const LOGIN_GZIP_BUDGET = 180_000;
const CHAT_GZIP_BUDGET = 280_000;
const CHUNK_RAW_BUDGET = 500_000;

export function resolveBundleAsset(distRoot: string, file: string): string {
  const root = resolve(distRoot);
  const fullPath = resolve(root, file);
  const relativePath = relative(root, fullPath);
  if (
    relativePath === ".." ||
    relativePath.startsWith(`..${sep}`) ||
    isAbsolute(relativePath)
  ) {
    throw new Error(`Vite manifest asset escapes the dist directory: ${file}`);
  }
  return fullPath;
}

export function createBundleAssetReader(
  distRoot: string,
): (file: string) => Buffer {
  const cache = new Map<string, Buffer>();

  return (file) => {
    const fullPath = resolveBundleAsset(distRoot, file);
    const cached = cache.get(fullPath);
    if (cached) return cached;

    const contents = readFileSync(fullPath);
    cache.set(fullPath, contents);
    return contents;
  };
}

function importClosure(manifest: Manifest, roots: string[]): Set<string> {
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

function javascriptFiles(manifest: Manifest, keys: Iterable<string>): Set<string> {
  const files = new Set<string>();
  for (const key of keys) {
    const file = manifest[key]?.file;
    if (file?.endsWith(".js")) files.add(file);
  }
  return files;
}

function gzipBytes(
  files: Iterable<string>,
  readAsset: (file: string) => Buffer,
): number {
  let total = 0;
  for (const file of files) {
    total += gzipSync(readAsset(file)).byteLength;
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

function headroom(actual: number, budget: number): string {
  return `${((budget - actual) / 1_000).toFixed(1)} KB headroom`;
}

function runCli(): void {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Manifest;
  const readAsset = createBundleAssetReader(distDir);
  const loginFiles = javascriptFiles(
    manifest,
    importClosure(manifest, ["index.html"]),
  );
  const chatFiles = javascriptFiles(
    manifest,
    importClosure(manifest, [
      "index.html",
      "src/layout/gateway-layout.tsx",
      "src/pages/chat/chat-page.tsx",
    ]),
  );
  const loginGzipBytes = gzipBytes(loginFiles, readAsset);
  const chatGzipBytes = gzipBytes(chatFiles, readAsset);

  assertAtMost("Login entry JavaScript (gzip)", loginGzipBytes, LOGIN_GZIP_BUDGET);
  assertAtMost("Initial /chat JavaScript (gzip)", chatGzipBytes, CHAT_GZIP_BUDGET);

  const emittedJavascript = new Set(
    Object.values(manifest)
      .map(({ file }) => file)
      .filter((file) => file.endsWith(".js")),
  );
  let largestChunk = { file: "", bytes: 0 };
  for (const file of emittedJavascript) {
    const bytes = readAsset(file).byteLength;
    if (bytes > largestChunk.bytes) largestChunk = { file, bytes };
    assertAtMost(`JavaScript chunk ${file} (raw)`, bytes, CHUNK_RAW_BUDGET);
  }

  console.log(
    [
      `Bundle budgets passed: login ${(loginGzipBytes / 1_000).toFixed(1)} KB gzip (${headroom(loginGzipBytes, LOGIN_GZIP_BUDGET)})`,
      `/chat ${(chatGzipBytes / 1_000).toFixed(1)} KB gzip (${headroom(chatGzipBytes, CHAT_GZIP_BUDGET)})`,
      `largest chunk ${largestChunk.file} ${(largestChunk.bytes / 1_000).toFixed(1)} KB raw (${headroom(largestChunk.bytes, CHUNK_RAW_BUDGET)})`,
    ].join("; "),
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) runCli();
