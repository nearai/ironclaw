import "dotenv/config";
import type { PluginConfigInput } from "every-plugin";
import packageJson from "./package.json" with { type: "json" };
import type Plugin from "./src/index";

export default {
  pluginId: packageJson.name,
  port: Number(process.env.PORT) || 3001,
  config: {
    variables: {},
    secrets: {
      API_DATABASE_URL: process.env.API_DATABASE_URL || "pglite:.bos/api/:memory:",
    },
  } satisfies PluginConfigInput<typeof Plugin>,
};
