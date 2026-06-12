import tsconfigPaths from "vite-tsconfig-paths";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    testTimeout: 60000,
    include: ["tests/integration/runtime-remote.test.ts"],
  },
  plugins: [
    tsconfigPaths({
      projects: ["./tsconfig.json"],
    }),
  ],
});
