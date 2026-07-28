const WRITE_RE = /(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/;
const EXEC_RE = /(bash|shell|exec|run|command|terminal|spawn|process)/;
const NETWORK_RE = /(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;

/* Heuristic risk classification. Tool name is the primary signal for high-risk
   write/exec categories. Description/parameters are only used as fallback for
   exec/network hints, preventing safe read tools whose description mentions
   "edit" from turning into a red write badge. */
export function classifyRisk(toolName, description, parameters) {
  const name = String(toolName || "").toLowerCase();
  const context = [description, parameters].filter(Boolean).join(" ").toLowerCase();

  if (WRITE_RE.test(name)) return { tone: "danger" as const, key: "tool.riskWrite" };
  if (EXEC_RE.test(name)) return { tone: "warning" as const, key: "tool.riskExec" };
  if (NETWORK_RE.test(name)) return { tone: "info" as const, key: "tool.riskNetwork" };

  if (EXEC_RE.test(context)) return { tone: "warning" as const, key: "tool.riskExec" };
  if (NETWORK_RE.test(context)) return { tone: "info" as const, key: "tool.riskNetwork" };

  return { tone: "muted" as const, key: "tool.riskRead" };
}
