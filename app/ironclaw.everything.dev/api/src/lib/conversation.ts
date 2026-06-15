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

export type ConversationEventType =
  | "snapshot"
  | "messages_changed"
  | "message_added"
  | "run_pending"
  | "run_finished"
  | "error"
  | "keep_alive"
  | "accepted"
  | "running"
  | "gate"
  | "auth_required"
  | "failed"
  | "cancelled"
  | "final_reply"
  | "capability_progress"
  | "capability_activity"
  | "capability_display_preview";

export interface ConversationEvent {
  type: ConversationEventType;
  threadId: string;
  messages?: ConversationMessage[];
  message?: ConversationMessage;
  runId?: string;
  error?: string;
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

function roleFromKind(kind: string): "user" | "assistant" {
  if (kind === "user") return "user";
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
    role: roleFromKind(raw.kind ?? ""),
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

export function diffMessageSets(
  prev: Set<string>,
  next: ConversationMessage[],
): ConversationMessage[] {
  const added: ConversationMessage[] = [];
  for (const msg of next) {
    if (!prev.has(msg.id)) {
      added.push(msg);
    }
  }
  return added;
}
