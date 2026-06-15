import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import DrizzleORMMigrations from "@proj-airi/unplugin-drizzle-orm-migrations/rspack";
import {
  EmitPluginManifest,
  EveryPluginDevServer,
  FixMfDataUriPlugin,
} from "every-plugin/build/rspack";
import { computeSriHashForUrl } from "everything-dev/integrity";
import { withZephyr } from "zephyr-rspack-plugin";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const shouldDeploy = process.env.DEPLOY === "true";

const resolvedConfigPath = path.resolve(__dirname, "../.bos/bos.resolved-config.json");
const bosConfigPath = path.resolve(__dirname, "../bos.config.json");

function readBosConfig() {
  const configPath = fs.existsSync(resolvedConfigPath) ? resolvedConfigPath : bosConfigPath;
  const raw = JSON.parse(fs.readFileSync(configPath, "utf8"));
  if (raw._resolved) {
    const { _resolved, ...data } = raw;
    return data;
  }
  return raw;
}

const _bosConfig = readBosConfig();

function updateHostConfig(url, integrity) {
  try {
    const configPath = path.resolve(__dirname, "../bos.config.json");
    const config = JSON.parse(fs.readFileSync(configPath, "utf8"));

    config.app.api.production = url;
    if (integrity) {
      config.app.api.integrity = integrity;
    } else {
      delete config.app.api.integrity;
    }
    fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    console.log(`   ✅ Updated bos.config.json: app.api.production`);
    if (integrity) {
      console.log(`   ✅ Updated bos.config.json: app.api.integrity`);
    }
  } catch (err) {
    console.error("   ❌ Failed to update bos.config.json:", err.message);
  }
}

const baseConfig = {
  externals: ["pg", "@electric-sql/pglite"],
  devtool: shouldDeploy ? false : "source-map",
  plugins: [
    new EmitPluginManifest(),
    new EveryPluginDevServer({ dts: false }),
    new FixMfDataUriPlugin(),
    DrizzleORMMigrations(),
  ],
  infrastructureLogging: {
    level: "error",
  },
  stats: "errors-warnings",
};

export default shouldDeploy
  ? withZephyr({
      hooks: {
        onDeployComplete: async (info) => {
          console.log("🚀 API Deployed:", info.url);
          const integrity = await computeSriHashForUrl(info.url);
          updateHostConfig(info.url, integrity ?? undefined);
        },
      },
    })(baseConfig)
  : baseConfig;
