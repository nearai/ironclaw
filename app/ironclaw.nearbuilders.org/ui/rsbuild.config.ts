import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ModuleFederationPlugin } from "@module-federation/enhanced/rspack";
import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import { TanStackRouterRspack } from "@tanstack/router-plugin/rspack";
import { FixMfDataUriPlugin } from "every-plugin/build/rspack";
import { computeSriHashForUrl } from "everything-dev/integrity";
import { withZephyr } from "zephyr-rsbuild-plugin";
import pkg from "./package.json";

const require = createRequire(import.meta.url);
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const normalizedName = pkg.name;
const shouldDeploy = process.env.DEPLOY === "true";
const buildTarget = process.env.BUILD_TARGET as "client" | "server" | undefined;
const isServerBuild = buildTarget === "server";

const resolvedConfigPath = path.resolve(__dirname, "../.bos/bos.resolved-config.json");
const bosConfigPath = path.resolve(__dirname, "../bos.config.json");
const bosConfig = fs.existsSync(resolvedConfigPath)
  ? (() => {
      const raw = JSON.parse(fs.readFileSync(resolvedConfigPath, "utf8"));
      const { _resolved, ...data } = raw;
      return data;
    })()
  : JSON.parse(fs.readFileSync(bosConfigPath, "utf8"));

function getInstalledVersion(pkgName: string, fallback: string): string {
  try {
    let currentDir = path.dirname(require.resolve(pkgName));
    for (let i = 0; i < 5; i += 1) {
      const packageJsonPath = path.join(currentDir, "package.json");
      if (fs.existsSync(packageJsonPath)) {
        return (JSON.parse(fs.readFileSync(packageJsonPath, "utf8")) as { version: string })
          .version;
      }
      currentDir = path.dirname(currentDir);
    }

    throw new Error(`Could not resolve installed version for ${pkgName}`);
  } catch {
    const match = fallback.match(/\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?/);
    return match ? match[0] : fallback.replace(/^[\^~>=<\s]+/, "");
  }
}

const SHARE_DEFAULTS = {
  requiredVersion: false,
  singleton: true,
  strictVersion: false,
  eager: false,
  shareScope: "default",
} as const;

const uiSharedDeps = {
  react: { version: getInstalledVersion("react", pkg.dependencies.react), ...SHARE_DEFAULTS },
  "react-dom": {
    version: getInstalledVersion("react-dom", pkg.dependencies["react-dom"]),
    ...SHARE_DEFAULTS,
  },
  "@orpc/client": {
    version: getInstalledVersion("@orpc/client", pkg.dependencies["@orpc/client"]),
    ...SHARE_DEFAULTS,
  },
  "@orpc/contract": {
    version: getInstalledVersion("@orpc/contract", pkg.dependencies["@orpc/contract"]),
    ...SHARE_DEFAULTS,
  },
  "@tanstack/react-query": {
    version: getInstalledVersion(
      "@tanstack/react-query",
      pkg.dependencies["@tanstack/react-query"],
    ),
    ...SHARE_DEFAULTS,
  },
  "@tanstack/react-router": {
    version: getInstalledVersion(
      "@tanstack/react-router",
      pkg.dependencies["@tanstack/react-router"],
    ),
    ...SHARE_DEFAULTS,
  },
};

function updateBosConfig(field: "production" | "ssr", url: string, integrity?: string) {
  try {
    const configPath = path.resolve(__dirname, "../bos.config.json");
    const config = JSON.parse(fs.readFileSync(configPath, "utf8"));

    if (!config.app.ui) {
      console.error("   ❌ app.ui not found in bos.config.json");
      return;
    }

    config.app.ui[field] = url;
    const integrityField = field === "production" ? "integrity" : "ssrIntegrity";
    if (integrity) {
      config.app.ui[integrityField] = integrity;
    } else {
      delete config.app.ui[integrityField];
    }
    fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
    console.log(`   ✅ Updated bos.config.json: app.ui.${field}`);
    if (integrity) {
      console.log(`   ✅ Updated bos.config.json: app.ui.${integrityField}`);
    }
  } catch (err) {
    console.error("   ❌ Failed to update bos.config.json:", (err as Error).message);
  }
}

function createClientConfig() {
  const plugins = [
    pluginReact({ fastRefresh: false }),
    pluginModuleFederation({
      name: normalizedName,
      filename: "remoteEntry.js",
      dts: false,
      exposes: {
        "./Router": "./src/router.tsx",
        "./Hydrate": "./src/hydrate.tsx",
        "./components": "./src/components/index.ts",
        "./providers": "./src/providers/index.tsx",
        "./hooks": "./src/hooks/index.ts",
      },
      shared: uiSharedDeps,
    }),
  ];

  if (shouldDeploy) {
    plugins.push(
      withZephyr({
        hooks: {
          onDeployComplete: async (info) => {
            console.log("🚀 UI Client Deployed:", info.url);
            const integrity = await computeSriHashForUrl(info.url);
            updateBosConfig("production", info.url, integrity ?? undefined);
          },
        },
      }),
    );
  }

  return defineConfig({
    plugins,
    source: {
      entry: {
        index: "./src/hydrate.tsx",
      },
      define: {
        "import.meta.env.APP_NAME": JSON.stringify(bosConfig.domain),
        "import.meta.env.APP_ACCOUNT": JSON.stringify(bosConfig.account),
      },
    },
    resolve: {
      alias: {
        "@": "./src",
      },
    },
    dev: {
      lazyCompilation: false,
      progressBar: false,
      client: {
        overlay: false,
      },
    },
    server: {
      port: isServerBuild ? 3004 : 3003,
      printUrls: ({ urls }) => urls.filter((url) => url.includes("localhost")),
      headers: {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type",
      },
    },
    tools: {
      rspack: {
        target: "web",
        output: {
          uniqueName: normalizedName,
          chunkFilename: "static/js/async/[name].[contenthash].js",
        },
        resolve: {
          fallback: { bufferutil: false, "utf-8-validate": false },
        },
        infrastructureLogging: { level: "error" },
        stats: "errors-warnings",
        plugins: [
          TanStackRouterRspack({
            target: "react",
            autoCodeSplitting: true,
          }),
          new FixMfDataUriPlugin(),
        ],
      },
    },
    output: {
      distPath: { root: "dist", css: "static/css", js: "static/js" },
      assetPrefix: "auto",
      filename: { js: "[name].js", css: "style.css" },
      copy: [{ from: path.resolve(__dirname, "public"), to: "./" }],
    },
  });
}

function createServerConfig() {
  const plugins = [pluginReact({ fastRefresh: false })];

  plugins.push({
    name: "restore-manifest-public-path",
    setup(api) {
      api.onAfterBuild(() => {
        const manifestPath = path.resolve(__dirname, "dist/mf-manifest.json");
        if (!fs.existsSync(manifestPath)) return;
        const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
        if (manifest.metaData?.publicPath && manifest.metaData.publicPath !== "auto") {
          manifest.metaData.publicPath = "auto";
          fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
        }
      });
    },
  });

  if (shouldDeploy) {
    plugins.push(
      withZephyr({
        hooks: {
          onDeployComplete: async (info) => {
            console.log("🚀 UI SSR Deployed:", info.url);
            const ssrEntryUrl = `${info.url.replace(/\/$/, "")}/remoteEntry.server.js`;
            const integrity = await computeSriHashForUrl(ssrEntryUrl, { resolveEntryUrl: false });
            updateBosConfig("ssr", info.url, integrity ?? undefined);
          },
        },
      }),
    );
  }

  return defineConfig({
    plugins,
    source: {
      entry: {
        index: "./src/router.server.tsx",
      },
    },
    resolve: {
      alias: {
        "@": "./src",
        "@tanstack/react-devtools": false,
        "@tanstack/react-router-devtools": false,
      },
    },
    server: {
      port: 3004,
      printUrls: ({ urls }) => urls.filter((url) => url.includes("localhost")),
      headers: {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type",
      },
    },
    tools: {
      rspack: {
        target: "async-node",
        output: {
          uniqueName: `${normalizedName}_server`,
          library: { type: "commonjs-module" },
        },
        resolve: {
          fallback: { bufferutil: false, "utf-8-validate": false },
        },
        externals: [/^node:/],
        infrastructureLogging: { level: "error" },
        stats: "errors-warnings",
        plugins: [
          TanStackRouterRspack({ target: "react", autoCodeSplitting: false }),
          new FixMfDataUriPlugin(),
          new ModuleFederationPlugin({
            name: normalizedName,
            filename: "remoteEntry.server.js",
            dts: false,
            runtimePlugins: [require.resolve("@module-federation/node/runtimePlugin")],
            library: { type: "commonjs-module" },
            exposes: { "./Router": "./src/router.server.tsx" },
            shared: uiSharedDeps,
          }),
        ],
      },
    },
    output: {
      distPath: { root: "dist" },
      assetPrefix: "/",
      cleanDistPath: false,
    },
  });
}

export default isServerBuild ? createServerConfig() : createClientConfig();
