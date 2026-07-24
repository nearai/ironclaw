import { describe, expect, it } from "vitest";

import { buildRuntimeContext } from "./runtime-context";

describe("buildRuntimeContext", () => {
  it("presents the default runtime as IronClaw", () => {
    const context = buildRuntimeContext({
      gatewayStatus: {
        total_connections: 1,
        llm_model: "test-model",
        llm_backend: "test-provider",
      },
      activeThread: {
        title: "Test thread",
        turn_count: 2,
      },
    });

    expect(context.engineLabel).toBe("IronClaw");
  });
});
