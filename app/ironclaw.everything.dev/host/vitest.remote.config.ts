import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    tsconfigPaths: true,
  },
  test: {
    environment: "node",
    testTimeout: 60000,
    include: ["tests/integration/runtime-remote.test.ts"],
  },
});
