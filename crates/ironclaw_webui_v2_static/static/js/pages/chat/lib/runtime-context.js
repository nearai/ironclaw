export function buildRuntimeContext({ gatewayStatus, activeThread }) {
  const turnCount = activeThread?.turn_count || 0;
  const connections = gatewayStatus?.total_connections;
  const engineLabel =
    gatewayStatus?.engine_v2_enabled === false ? "Engine v1" : "Engine v2";

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
