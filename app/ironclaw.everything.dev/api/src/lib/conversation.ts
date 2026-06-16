export interface ConversationThread {
  threadId: string;
  title: string | null;
  tenantId: string;
  agentId: string;
  projectId: string | null;
  createdByActorId: string;
  createdAt: string | null;
  updatedAt: string | null;
}

export interface ConversationAttachmentRef {
  id: string;
  kind: "audio" | "image" | "document";
  mimeType: string;
  filename?: string;
  sizeBytes?: number;
}

export interface ConversationMessage {
  id: string;
  threadId: string;
  role: "user" | "assistant";
  text: string;
  createdAt: string | null;
  status: "submitted" | "finalized" | "failed";
  sequence: number;
  runId: string | null;
  attachments?: ConversationAttachmentRef[];
}

export interface ConversationMessagePage {
  messages: ConversationMessage[];
  nextCursor: string | null;
  hasMore: boolean;
  total: number;
}

export interface ConversationSendAck {
  threadId: string;
  runId: string;
  acceptedMessageRef: string;
  pendingMessageId: string;
  submittedAt: string;
  eventCursor?: number;
}

export function normalizeThread(raw: any): ConversationThread {
  const scope = raw.scope ?? {};
  return {
    threadId: raw.threadId ?? raw.thread_id ?? "",
    title: raw.title ?? null,
    tenantId: scope.tenantId ?? scope.tenant_id ?? "",
    agentId: scope.agentId ?? scope.agent_id ?? "",
    projectId: scope.projectId ?? scope.project_id ?? null,
    createdByActorId: raw.createdByActorId ?? raw.created_by_actor_id ?? "",
    createdAt: raw.createdAt ?? raw.created_at ?? null,
    updatedAt: raw.updatedAt ?? raw.updated_at ?? null,
  };
}

function roleFromKind(raw: any): "user" | "assistant" {
  const kind = raw.kind ?? raw.Kind ?? "";
  const role = raw.role ?? raw.Role;
  if (role === "user" || role === "assistant") return role;
  const lower = kind.toLowerCase();
  if (lower === "user" || lower === "user_message") return "user";
  if (lower === "assistant" || lower === "assistant_message" || lower === "tool_result")
    return "assistant";
  if (raw.actorId ?? raw.actor_id) return "user";
  return "assistant";
}

function statusFromString(s: string | undefined): "submitted" | "finalized" | "failed" {
  if (s === "finalized") return "finalized";
  if (s === "failed") return "failed";
  return "submitted";
}

export function normalizeTimelineEntry(raw: any, threadId: string): ConversationMessage {
  return {
    id: raw.messageId ?? raw.message_id ?? raw.id ?? "",
    threadId,
    role: roleFromKind(raw),
    text: raw.content ?? "",
    createdAt: raw.createdAt ?? raw.created_at ?? null,
    status: statusFromString(raw.status),
    sequence: raw.sequence ?? 0,
    runId: raw.turnRunId ?? raw.turn_run_id ?? null,
    attachments: (raw.attachments ?? []).map((att: any) => ({
      id: att.id ?? att.attachment_id ?? "",
      kind: att.kind ?? "document",
      mimeType: att.mime_type ?? "application/octet-stream",
      filename: att.filename ?? undefined,
      sizeBytes: att.size_bytes ?? undefined,
    })),
  };
}

export function normalizeTimelinePage(raw: any, threadId: string): ConversationMessagePage {
  const data: any[] = raw.data ?? [];
  const meta = raw.meta ?? {};
  return {
    messages: data.map((entry: any) => normalizeTimelineEntry(entry, threadId)),
    nextCursor: meta.nextCursor ?? meta.next_cursor ?? null,
    hasMore: meta.hasMore ?? meta.has_more ?? false,
    total: meta.total ?? data.length,
  };
}
