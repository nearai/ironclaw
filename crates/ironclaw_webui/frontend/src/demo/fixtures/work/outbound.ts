// DEMO outbound-delivery fixtures for `/api/webchat/v2/outbound/*`.
//
// Wire shapes mirror `RebornOutboundPreferencesResponse` /
// `RebornOutboundDeliveryTargetListResponse`: preferences carry the resolved
// target summary + status, targets pair a summary with capability flags. The
// delivery-defaults panel filters on `capabilities.final_replies` and renders
// `target.status`, so the seed includes one unavailable channel.

type TargetSummary = {
  target_id: string;
  channel: string;
  display_name: string;
  description?: string;
  status: "available" | "unavailable";
};

type TargetOption = {
  target: TargetSummary;
  capabilities: {
    final_replies: boolean;
    gate_prompts: boolean;
    auth_prompts: boolean;
  };
};

const targets: TargetOption[] = [
  {
    target: {
      target_id: "slack:C0292OPS",
      channel: "slack",
      display_name: "Slack — #ops",
      description: "Posts final replies to the #ops channel in the Near AI workspace.",
      status: "available",
    },
    capabilities: { final_replies: true, gate_prompts: true, auth_prompts: true },
  },
  {
    target: {
      target_id: "telegram:8841",
      channel: "telegram",
      display_name: "Telegram — @operator",
      description: "Direct message to the paired Telegram account.",
      status: "available",
    },
    capabilities: { final_replies: true, gate_prompts: true, auth_prompts: false },
  },
  {
    target: {
      target_id: "whatsapp:bridge-1",
      channel: "whatsapp",
      display_name: "WhatsApp bridge",
      description: "Pairing expired — re-link the WhatsApp bridge to resume delivery.",
      status: "unavailable",
    },
    capabilities: { final_replies: true, gate_prompts: false, auth_prompts: false },
  },
];

const state = {
  finalReplyTargetId: "slack:C0292OPS" as string | null,
};

export function listDeliveryTargets() {
  return { targets };
}

export function outboundPreferences() {
  const selected = state.finalReplyTargetId
    ? targets.find((option) => option.target.target_id === state.finalReplyTargetId)
    : undefined;
  if (!selected) {
    return { final_reply_target: null, final_reply_target_status: "none_configured" };
  }
  return {
    final_reply_target: selected.target,
    final_reply_target_status: selected.target.status,
  };
}

export function setFinalReplyTarget(targetId: unknown) {
  state.finalReplyTargetId = typeof targetId === "string" && targetId ? targetId : null;
  return outboundPreferences();
}
