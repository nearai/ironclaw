import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const rustStaticDir = resolve(here, "..", "static");

export default defineConfig({
  base: "/",
  plugins: [tailwindcss(), react()],
  publicDir: "public",
  resolve: {
    // Keep one React runtime without rewriting package ids through Node's
    // CommonJS resolver. Bare imports can now select each dependency's ESM
    // export, which lets Rolldown tree-shake unused code.
    dedupe: ["react", "react-dom"],
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    fs: {
      allow: [here, rustStaticDir],
    },
    proxy: {
      "/api/webchat/v2": "http://127.0.0.1:3000",
      "/api/reborn": "http://127.0.0.1:3000",
      "/auth": "http://127.0.0.1:3000",
      "/assets": "http://127.0.0.1:3000",
      "/vendor": "http://127.0.0.1:3000",
      "/wallet/connect": "http://127.0.0.1:3000",
      "/wallet-connect.js": "http://127.0.0.1:3000",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    manifest: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      input: {
        app: resolve(here, "index.html"),
        "wallet-connect": resolve(here, "src/wallet-connect.ts"),
      },
      external: ["@hot-labs/near-connect"],
      output: {
        entryFileNames: (chunk) =>
          chunk.name === "wallet-connect" ? "wallet-connect.js" : "assets/[name]-[hash].js",
      },
    },
  },
  test: {
    environment: "node",
    include: [
      "src/**/*.{test,spec}.{ts,tsx}",
      "src/pages/extensions/hooks/useExtensions-oauth.test.mjs",
    ],
    setupFiles: ["src/test/vm-tsx-setup.ts"],
  },
});
