import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import path from "node:path";
import { startMockLlm } from "./mock-llm";

const REPO_ROOT = path.resolve(import.meta.dirname, "../../../../..");
const APP_DIR = path.resolve(REPO_ROOT, "app/ironclaw.everything.dev");
const REBORN_BIN = path.resolve(REPO_ROOT, "target/debug/ironclaw-reborn");

const AUTH_TOKEN = "e2e-reborn-real-token-0123456789";
const USER_ID = "reborn-cli";

async function waitForReady(url: string, timeout: number, interval: number): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(5000) });
      if (res.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, interval));
  }
  throw new Error(`Timed out waiting for ${url}`);
}

function readStreamLines(stream: NodeJS.ReadableStream | null, buffer: string[], maxLines: number) {
  if (!stream) return;
  stream.on("data", (chunk: Buffer) => {
    const lines = chunk.toString().split("\n");
    for (const line of lines) {
      buffer.push(line);
      if (buffer.length > maxLines) buffer.shift();
    }
  });
}

function getLastLines(buffer: string[], count: number): string {
  return buffer.slice(-count).join("\n");
}

function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const port = (server.address() as any).port;
      server.close(() => resolve(port));
    });
    server.on("error", reject);
  });
}

export interface RealStackHandle {
  appBaseUrl: string;
  rebornBaseUrl: string;
  rebornToken: string;
  mockLlmUrl: string;
  logs: {
    app: string[];
    reborn: string[];
    mockLlm: string[];
  };
  stop: () => Promise<void>;
}

export async function startRealStack(): Promise<RealStackHandle> {
  const appLogs: string[] = [];
  const rebornLogs: string[] = [];
  const tempDir = mkdtempSync("/tmp/ironclaw-reborn-real-");

  const mock = await startMockLlm();
  const mockLlmUrl = mock.baseUrl;
  await waitForReady(`${mockLlmUrl}/v1/models`, 15000, 500);

  writeFileSync(
    path.join(tempDir, "config.toml"),
    `api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "${USER_ID}"
tenant = "e2e-reborn"
default_agent = "e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "${mockLlmUrl}/v1"
`,
  );

  const rebornPort = await findFreePort();
  const rebornEnv: Record<string, string> = {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: process.env.HOME ?? "/tmp",
    IRONCLAW_REBORN_HOME: tempDir,
    IRONCLAW_REBORN_PROFILE: "local-dev",
    IRONCLAW_REBORN_WEBUI_TOKEN: AUTH_TOKEN,
    IRONCLAW_REBORN_WEBUI_USER_ID: USER_ID,
    MOCK_LLM_API_KEY: "mock-api-key",
    RUST_LOG: "warn",
    RUST_BACKTRACE: "1",
    NO_PROXY: "127.0.0.1,localhost,::1",
    no_proxy: "127.0.0.1,localhost,::1",
  };
  const rebornProc = spawn(
    REBORN_BIN,
    ["serve", "--host", "127.0.0.1", "--port", String(rebornPort)],
    {
      cwd: REPO_ROOT,
      stdio: ["ignore", "pipe", "pipe"],
      env: rebornEnv,
    },
  );
  readStreamLines(rebornProc.stdout, rebornLogs, 500);
  readStreamLines(rebornProc.stderr, rebornLogs, 500);

  const rebornBaseUrl = `http://127.0.0.1:${rebornPort}`;
  await waitForReady(`${rebornBaseUrl}/api/health`, 30000, 500);

  const sessionRes = await fetch(`${rebornBaseUrl}/api/webchat/v2/session`, {
    headers: { Authorization: `Bearer ${AUTH_TOKEN}` },
  });
  if (!sessionRes.ok) {
    throw new Error(
      `Reborn session check failed: ${sessionRes.status}\nReborn logs:\n${getLastLines(rebornLogs, 30)}`,
    );
  }

  // Start app dev stack (bun run dev on port 3000)
  const appDevProc = spawn("bun", ["run", "dev"], {
    cwd: APP_DIR,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env },
    shell: true,
  });
  readStreamLines(appDevProc.stdout, appLogs, 500);
  readStreamLines(appDevProc.stderr, appLogs, 500);

  const appPort = 3000;
  const appBaseUrl = `http://127.0.0.1:${appPort}`;
  try {
    // Wait for login page to be served
    await waitForReady(`${appBaseUrl}/login`, 120000, 2000);

    // Wait for API health to confirm plugins are loaded
    await waitForReady(`${appBaseUrl}/api/_health`, 60000, 1000);
  } catch {
    throw new Error(
      `App stack failed to start.\nReborn logs:\n${getLastLines(rebornLogs, 20)}\nApp logs:\n${getLastLines(appLogs, 20)}`,
    );
  }

  const stop = async () => {
    await mock.stop();
    for (const proc of [appDevProc, rebornProc]) {
      try {
        proc.kill("SIGINT");
      } catch {}
    }
    await new Promise((r) => setTimeout(r, 1000));
    for (const proc of [appDevProc, rebornProc]) {
      try {
        proc.kill("SIGKILL");
      } catch {}
    }
  };

  return {
    appBaseUrl,
    rebornBaseUrl,
    rebornToken: AUTH_TOKEN,
    mockLlmUrl,
    logs: { app: appLogs, reborn: rebornLogs, mockLlm: [] },
    stop,
  };
}
