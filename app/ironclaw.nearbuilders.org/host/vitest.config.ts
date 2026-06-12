import tsconfigPaths from "vite-tsconfig-paths";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    testTimeout: 30000,
    include: ["tests/**/*.test.ts"],
    globalSetup: ["./tests/global-setup.ts"],
  },
  plugins: [
    tsconfigPaths({
      projects: ["./tsconfig.json"],
    }),
  ],
});
