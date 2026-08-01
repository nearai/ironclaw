// Single source of truth for gate "kind" discriminators. These arrive on
// the wire as `GatePromptView.gate_kind` / `ProjectionGate.gate_kind` and
// are compared in several surfaces (gate normalization, approval card
// layout). Keeping them as frozen constants avoids the bare string
// literals that previously drifted across files.
export const GATE_KIND = Object.freeze({
  // Tool-approval gate (the default for live gate prompts).
  APPROVAL: "approval",
  // Resource/budget gate — rendered with a compact, non-scrolling detail
  // layout in the approval card.
  RESOURCE: "resource",
  // Generic projection gate (the default for durable projection gates).
  GENERIC: "generic",
  // Authentication / credential gate — routed to the auth cards.
  AUTH: "auth",
} as const);
