import "dotenv/config";
import type { PluginConfigInput } from "every-plugin";
import packageJson from "./package.json" with { type: "json" };
import type Plugin from "./src/index";

export default {
  pluginId: packageJson.name,
  port: Number(process.env.PORT) || 3010,
  config: {
    variables: {
      baseUrl: process.env.IRONCLAW_BASE_URL || "http://localhost:3001",
    },
    secrets: {
      apiToken: process.env.IRONCLAW_API_TOKEN || "dev-token",
    },
  } satisfies PluginConfigInput<typeof Plugin>,
};
