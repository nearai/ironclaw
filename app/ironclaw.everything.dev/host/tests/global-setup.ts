import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, statSync } from "node:fs";
import path from "node:path";

function ensureUiBuild(uiDir: string) {
  const distDir = path.join(uiDir, "dist");
  mkdirSync(distDir, { recursive: true });

  const clientEntry = path.join(distDir, "remoteEntry.js");
  const ssrEntry = path.join(distDir, "remoteEntry.server.js");

  const hasClient = existsSync(clientEntry) && statSync(clientEntry).size > 0;
  const hasSsr = existsSync(ssrEntry) && statSync(ssrEntry).size > 0;

  if (hasClient && hasSsr) return;

  const result = spawnSync("bun", ["run", "build"], {
    cwd: uiDir,
    stdio: "inherit",
    env: { ...process.env },
  });
  if (result.status !== 0) {
    throw new Error(`UI build failed (exit ${result.status ?? "unknown"})`);
  }
}

export default async function globalSetup() {
  const repoRoot = path.resolve(__dirname, "../..");
  const uiDir = path.join(repoRoot, "ui");

  ensureUiBuild(uiDir);
}
