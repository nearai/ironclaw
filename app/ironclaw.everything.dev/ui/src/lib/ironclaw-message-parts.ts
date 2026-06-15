import type { MessagePart, UIMessage } from "@tanstack/ai";
import type { ConversationMessageType } from "../../../api/src/contract";

export interface IronclawToolResultEnvelope {
  title: string;
  inputSummary: string | null;
  output: string;
  outputKind: string | null;
  truncated: boolean;
}

export interface RestMessageToPartsOptions {
  toolCallIdFallback?: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function asText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  return JSON.stringify(value);
}

function asOptionalText(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

export function serializeIronclawToolResultEnvelope(envelope: IronclawToolResultEnvelope): string {
  return JSON.stringify(envelope);
}

export function parseIronclawToolResultEnvelope(content: unknown): IronclawToolResultEnvelope | null {
  if (content == null) return null;

  if (isRecord(content)) {
    if (typeof content.output === "string" || typeof content.title === "string") {
      return {
        title: typeof content.title === "string" ? content.title : "unknown",
        inputSummary: asOptionalText(content.input_summary),
        output: asText(content.output ?? content.text ?? content.result ?? ""),
        outputKind: asOptionalText(content.output_kind),
        truncated: Boolean(content.truncated),
      };
    }

    return null;
  }

  if (typeof content !== "string" || !content) return null;

  try {
    const parsed = JSON.parse(content);
    if (!isRecord(parsed)) return null;

    if (parsed.version === 1 && typeof parsed.capability_id === "string" && typeof parsed.invocation_id === "string") {
      return {
        title: typeof parsed.title === "string" ? parsed.title : parsed.capability_id,
        inputSummary: asOptionalText(parsed.input_summary),
        output: asText(parsed.output_preview ?? parsed.output_summary ?? ""),
        outputKind: asOptionalText(parsed.output_kind),
        truncated: Boolean(parsed.truncated),
      };
    }

    if (typeof parsed.output === "string" || typeof parsed.title === "string") {
      return {
        title: typeof parsed.title === "string" ? parsed.title : "unknown",
        inputSummary: asOptionalText(parsed.input_summary),
        output: asText(parsed.output ?? parsed.text ?? parsed.result ?? ""),
        outputKind: asOptionalText(parsed.output_kind),
        truncated: Boolean(parsed.truncated),
      };
    }

    return null;
  } catch {
    return null;
  }
}

export function restMessageToParts(
  role: string,
  text: string,
  options: RestMessageToPartsOptions = {},
): MessagePart[] {
  const trimmed = text.trim();
  if (role !== "assistant" || !trimmed) {
    return [{ type: "text" as const, content: trimmed }];
  }

  try {
    const parsed = JSON.parse(trimmed);
    if (!isRecord(parsed)) {
      return [{ type: "text" as const, content: trimmed }];
    }

    const isVersionedTool =
      parsed.version === 1 &&
      typeof parsed.capability_id === "string" &&
      typeof parsed.invocation_id === "string";

    const looksLikeEnvelope =
      isVersionedTool || typeof parsed.output === "string" || typeof parsed.title === "string";

    if (!looksLikeEnvelope) {
      if (parsed.result_ref) {
        return [];
      }
      return [{ type: "text" as const, content: trimmed }];
    }

    const toolCallId =
      typeof parsed.invocation_id === "string"
        ? parsed.invocation_id
        : options.toolCallIdFallback ??
          (typeof parsed.capability_id === "string"
            ? parsed.capability_id
            : typeof parsed.title === "string"
              ? parsed.title
              : "tool-call");
    const displayName =
      typeof parsed.title === "string"
        ? parsed.title
        : typeof parsed.capability_id === "string"
          ? parsed.capability_id
          : "unknown";
    const outputText = asText(parsed.output_preview ?? parsed.output_summary ?? parsed.output ?? "");
    const status = typeof parsed.status === "string" ? parsed.status : undefined;
    const isError = status === "failed" || status === "error" || status === "killed";
    const toolOutput = {
      output: outputText,
      output_kind: asOptionalText(parsed.output_kind),
      truncated: Boolean(parsed.truncated),
      input_summary: asOptionalText(parsed.input_summary),
      title: displayName,
    };

    return [
      {
        type: "tool-call" as const,
        id: toolCallId,
        name: displayName,
        arguments: parsed.input_summary ? JSON.stringify({ input: parsed.input_summary }) : "{}",
        output: toolOutput,
        state: "input-complete" as const,
      },
      {
        type: "tool-result" as const,
        toolCallId,
        content: serializeIronclawToolResultEnvelope({
          output: outputText,
          outputKind: asOptionalText(parsed.output_kind),
          truncated: Boolean(parsed.truncated),
          inputSummary: asOptionalText(parsed.input_summary),
          title: displayName,
        }),
        state: isError ? ("error" as const) : ("complete" as const),
      },
    ];
  } catch {
    return [{ type: "text" as const, content: trimmed }];
  }

  return [{ type: "text" as const, content: trimmed }];
}

export function messagesToUIMessages(
  rawMessages: ConversationMessageType[],
): UIMessage[] {
  const result: UIMessage[] = [];
  let i = 0;
  while (i < rawMessages.length) {
    const raw = rawMessages[i];

    if (raw.role !== "assistant") {
      result.push({
        id: raw.id,
        role: raw.role,
        parts: restMessageToParts(raw.role, raw.text ?? "", { toolCallIdFallback: raw.id }),
        createdAt: raw.createdAt ? new Date(raw.createdAt) : undefined,
      });
      i++;
      continue;
    }

    const runId = raw.runId;
    if (!runId) {
      result.push({
        id: raw.id,
        role: "assistant",
        parts: restMessageToParts("assistant", raw.text ?? "", { toolCallIdFallback: raw.id }),
        createdAt: raw.createdAt ? new Date(raw.createdAt) : undefined,
      });
      i++;
      continue;
    }

    const group: ConversationMessageType[] = [];
    while (i < rawMessages.length && rawMessages[i].role === "assistant" && rawMessages[i].runId === runId) {
      group.push(rawMessages[i]);
      i++;
    }

    const allParts: MessagePart[] = [];
    let groupCreatedAt: string | null = null;
    for (const g of group) {
      const parts = restMessageToParts("assistant", g.text ?? "", { toolCallIdFallback: g.id });
      const textParts = parts.filter((p): p is MessagePart & { type: "text" } => p.type === "text");
      const toolParts = parts.filter((p) => p.type === "tool-call" || p.type === "tool-result");
      const otherParts = parts.filter((p) => p.type !== "text" && p.type !== "tool-call" && p.type !== "tool-result");

      if (toolParts.length > 0 || otherParts.length > 0) {
        allParts.push(...toolParts, ...otherParts);
      }
      if (textParts.length > 0) {
        allParts.push(...textParts);
      }
      if (g.createdAt && (!groupCreatedAt || g.createdAt > groupCreatedAt)) {
        groupCreatedAt = g.createdAt;
      }
    }

    if (allParts.length > 0) {
      result.push({
        id: runId ? `assistant:${runId}` : group[0].id,
        role: "assistant",
        parts: allParts,
        createdAt: groupCreatedAt ? new Date(groupCreatedAt) : undefined,
      });
    }
  }
  return result;
}
