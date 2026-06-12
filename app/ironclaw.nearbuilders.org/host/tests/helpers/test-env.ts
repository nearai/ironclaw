import { existsSync } from "node:fs";
import path from "node:path";
import { config as loadEnvFile } from "dotenv";

let loaded = false;

export function loadHostTestEnv(workspaceRoot: string) {
  if (loaded) return;
  loaded = true;

  const envPaths = [path.join(workspaceRoot, ".env"), path.join(workspaceRoot, ".env.test")];

  for (const envPath of envPaths) {
    if (!existsSync(envPath)) continue;
    loadEnvFile({ path: envPath, override: false, quiet: true });
  }

  const testEnvPath = path.join(workspaceRoot, ".env.test");
  if (existsSync(testEnvPath)) {
    loadEnvFile({ path: testEnvPath, override: true, quiet: true });
  }
}
