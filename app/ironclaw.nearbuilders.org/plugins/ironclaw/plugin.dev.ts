import "dotenv/config";
import type { PluginConfigInput } from "every-plugin";
import packageJson from "./package.json" with { type: "json" };
import type Plugin from "./src/index";

export default {
  pluginId: packageJson.name,
  port: Number(process.env.PORT) || 3010,
  config: {
    variables: {
      baseUrl: "http://localhost:3001",
    },
    secrets: {},
  } satisfies PluginConfigInput<typeof Plugin>,
};
