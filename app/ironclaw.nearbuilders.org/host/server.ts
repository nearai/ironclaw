import "dotenv/config";
import { runServerBlocking } from "./src/program";
import type { RuntimeConfig } from "./src/services/config";

const configJson = process.env.BOS_RUNTIME_CONFIG;

if (!configJson) {
  console.error(`
╔═══════════════════════════════════════════════════════════════════╗
║  BOS_RUNTIME_CONFIG environment variable is required.             ║
║                                                                   ║
║  The host must be started through the BOS CLI:                    ║
║    bos dev                  # Full local development              ║
║    bos dev --host=local     # Local host with remote UI/API       ║
║    bos start                # Production mode                     ║
║                                                                   ║
║  Direct 'bun run dev' is not supported.                           ║
╚═══════════════════════════════════════════════════════════════════╝
`);
  process.exit(1);
}

let config: RuntimeConfig;
try {
  config = JSON.parse(configJson) as RuntimeConfig;
} catch (e) {
  console.error("Failed to parse BOS_RUNTIME_CONFIG:", e);
  process.exit(1);
}

runServerBlocking({ config });
