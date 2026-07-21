/**
 * Library build for non-source consumers (`pnpm build` → dist/).
 *
 * The primary consumption path is source-first via the `exports` map
 * (workspace app + any Vite/Tailwind host compile the TSX directly);
 * this config additionally emits a compiled ESM bundle with externals
 * for hosts that cannot compile TSX. Declarations come from
 * `tsc -p tsconfig.build.json` in the same `build` script.
 */
import react from "@vitejs/plugin-react";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    target: "es2022",
    lib: {
      entry: {
        index: resolve(here, "src/index.ts"),
        tokens: resolve(here, "src/tokens.ts"),
        motion: resolve(here, "src/motion.ts"),
      },
      formats: ["es"],
    },
    rollupOptions: {
      external: [
        /^react($|\/)/,
        /^react-dom($|\/)/,
        /^@radix-ui\//,
        "class-variance-authority",
        "clsx",
        "lucide-react",
        /^motion($|\/)/,
        "tailwind-merge",
      ],
    },
  },
});
