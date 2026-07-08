export function buildRuntimeContext({ gatewayStatus, activeThread }) {
  const turnCount = activeThread?.turn_count || 0;
  const connections = gatewayStatus?.total_connections;
  // Reborn has one engine; the v1/v2 split is gone.
  const engineLabel = "Reborn";

  return {
    mode: "Auto-review",
    runtime: "Work locally",
    workspace: "ironclaw",
    model: gatewayStatus?.llm_model,
    backend: gatewayStatus?.llm_backend,
    threadLabel: activeThread?.title || "New thread",
    turnCountLabel: `${turnCount} ${turnCount === 1 ? "turn" : "turns"}`,
    engineLabel,
    connectionLabel:
      typeof connections === "number"
        ? `${connections} live ${
            connections === 1 ? "connection" : "connections"
          }`
        : null,
  };
}
