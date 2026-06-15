import "dotenv/config";
import type { PluginConfigInput } from "every-plugin";
import packageJson from "./package.json" with { type: "json" };
import type Plugin from "./src/index";

export default {
  pluginId: packageJson.name,
  port: Number(process.env.PORT) || 3010,
  config: {
    variables: {
      baseUrl: "https://api.example.com",
      timeout: 10000,
    },
    secrets: {
      apiKey: process.env.TEMPLATE_PLUGIN_API_KEY || "dev-key-12345",
    },
  } satisfies PluginConfigInput<typeof Plugin>,
};
