const PENDING_MESSAGE_TTL_MS = 30_000;
const GENERATED_IMAGE_THREAD_CAP = 20;
const GENERATED_IMAGES_PER_THREAD_CAP = 80;

const generatedImagesByThread = new Map();

export function turnsToMessages(turns, { threadId, pendingMessages = [] } = {}) {
  const messages = [];
  const matchedPendingIds = new Set();

  for (const turn of turns) {
    if (turn.user_input) {
      const parsed = parseUserMessageContent(turn.user_input);
      const pending = findMatchingPending(pendingMessages, parsed, matchedPendingIds);
      if (pending) matchedPendingIds.add(pending.id);
      messages.push({
        id: `turn-${turn.turn_number}-user`,
        role: "user",
        content: pending?.content || parsed.text || "(attachment)",
        copyText: pending?.copyText || parsed.copyText,
        timestamp: turn.started_at,
        turnNumber: turn.turn_number,
        images: pending?.images || [],
        attachments: pending?.attachments || parsed.attachments,
      });
    }

    if (turn.tool_calls && turn.tool_calls.length > 0) {
      messages.push({
        id: `turn-${turn.turn_number}-tools`,
        role: "tool_activity",
        content: "",
        timestamp: turn.started_at,
        turnNumber: turn.turn_number,
        toolCalls: turn.tool_calls.map(normalizeToolCall),
      });
    }

    if (turn.generated_images && turn.generated_images.length > 0) {
      for (const img of turn.generated_images) {
        const resolved = resolveGeneratedImage(threadId, img);
        rememberGeneratedImage(
          threadId,
          img.event_id,
          resolved.data_url,
          resolved.path
        );
        messages.push({
          id: `turn-${turn.turn_number}-img-${img.event_id || Math.random().toString(36).slice(2)}`,
          role: "image",
          content: "",
          timestamp: turn.started_at,
          generatedImages: [resolved],
        });
      }
    }

    if (turn.response) {
      messages.push({
        id: `turn-${turn.turn_number}-assistant`,
        role: "assistant",
        content: turn.response,
        timestamp: turn.completed_at || turn.started_at,
        turnNumber: turn.turn_number,
      });
    }
  }

  const remainingPending = freshPendingMessages(pendingMessages).filter(
    (pending) => !matchedPendingIds.has(pending.id)
  );
  for (const pending of remainingPending) {
    messages.push(pendingToMessage(pending));
  }

  return { messages, remainingPending };
}

export function appendInProgressMessage(messages, inProgress, pendingMessages = []) {
  if (!inProgress?.user_input) return messages;
  const parsed = parseUserMessageContent(inProgress.user_input);
  const content = parsed.text || "(attachment)";
  const alreadyRendered = messages.some(
    (message) =>
      message.role === "user" &&
      (message.content === content || message.copyText === parsed.copyText)
  );
  if (alreadyRendered) return messages;

  const pending = findMatchingPending(pendingMessages, parsed, new Set());
  return [
    ...messages,
    pending
      ? pendingToMessage(pending)
      : {
          id: `in-progress-${inProgress.user_message_id || inProgress.turn_number || Date.now()}`,
          role: "user",
          content,
          copyText: parsed.copyText,
          timestamp: inProgress.started_at || new Date().toISOString(),
          attachments: parsed.attachments,
          isOptimistic: true,
        },
  ];
}

export function parseUserMessageContent(content = "") {
  const match = content.match(
    /^([\s\S]*?)(?:\n\n)?<attachments>([\s\S]*?)<\/attachments>\s*$/
  );
  if (!match) {
    return { text: content, attachments: [], copyText: content };
  }

  const attachments = [];
  const attachmentRegex = /<attachment\b([^>]*)>([\s\S]*?)<\/attachment>/g;
  let attachmentMatch;
  while ((attachmentMatch = attachmentRegex.exec(match[2])) !== null) {
    const attrs = parseAttachmentAttributes(attachmentMatch[1]);
    attachments.push({
      kind: attrs.type === "image" ? "image" : "document",
      filename: attrs.filename || "attachment",
      mime_type: attrs.mime || "",
      size_label: attrs.size || "",
      preview_text: decodeXmlText(attachmentMatch[2].trim()),
      preview_url: null,
    });
  }

  if (attachments.length === 0) {
    return { text: content, attachments: [], copyText: content };
  }

  const text = match[1].replace(/\s+$/, "");
  const copyParts = text ? [text] : [];
  for (const attachment of attachments) {
    const suffix = [attachment.mime_type, attachment.size_label]
      .filter(Boolean)
      .join(" / ");
    copyParts.push(
      suffix
        ? `[Attachment] ${attachment.filename} (${suffix})`
        : `[Attachment] ${attachment.filename}`
    );
  }

  return { text, attachments, copyText: copyParts.join("\n") };
}

export function rememberGeneratedImage(threadId, eventId, dataUrl, path) {
  if (!threadId || !eventId || !isSafeGeneratedImageDataUrl(dataUrl)) return;
  let images = generatedImagesByThread.get(threadId);
  if (!images) {
    if (generatedImagesByThread.size >= GENERATED_IMAGE_THREAD_CAP) {
      generatedImagesByThread.delete(generatedImagesByThread.keys().next().value);
    }
    images = [];
  } else {
    generatedImagesByThread.delete(threadId);
  }
  generatedImagesByThread.set(threadId, images);

  const existing = images.find((image) => image.eventId === eventId);
  if (existing) {
    existing.data_url = dataUrl;
    existing.path = path || existing.path || null;
    return;
  }

  images.push({ eventId, data_url: dataUrl, path: path || null });
  while (images.length > GENERATED_IMAGES_PER_THREAD_CAP) images.shift();
}

export function resolveGeneratedImage(threadId, image = {}) {
  const path = image.path || null;
  if (isSafeGeneratedImageDataUrl(image.data_url)) {
    return { event_id: image.event_id, data_url: image.data_url, path };
  }

  const remembered = getRememberedGeneratedImage(threadId, image.event_id);
  if (remembered) {
    return {
      event_id: image.event_id,
      data_url: remembered.data_url,
      path: remembered.path || path,
    };
  }

  return { event_id: image.event_id, data_url: null, path };
}

function normalizeToolCall(toolCall) {
  return {
    callId: toolCall.call_id || null,
    toolName: toolCall.name || "tool",
    toolStatus: toolCall.has_error
      ? "error"
      : toolCall.has_result
        ? "success"
        : "running",
    toolError: toolCall.error || "",
    toolResultPreview: toolCall.result_preview || toolCall.result || "",
    toolParameters: toolCall.parameters || "",
    toolRationale: toolCall.rationale || "",
  };
}

function findMatchingPending(pendingMessages, parsed, matchedIds) {
  const candidates = freshPendingMessages(pendingMessages);
  return candidates.find(
    (pending) =>
      !matchedIds.has(pending.id) &&
      (pending.content === parsed.text ||
        pending.copyText === parsed.copyText ||
        attachmentsMatch(pending.attachments, parsed.attachments))
  );
}

function attachmentsMatch(pendingAttachments = [], persistedAttachments = []) {
  if (pendingAttachments.length === 0 || persistedAttachments.length === 0) {
    return false;
  }
  const pendingKeys = new Set(pendingAttachments.map(attachmentKey));
  return persistedAttachments.every((attachment) => pendingKeys.has(attachmentKey(attachment)));
}

function attachmentKey(attachment) {
  return [
    attachment.filename || "attachment",
    attachment.mime_type || "",
    attachment.size_label || "",
  ].join("\u001f");
}

function freshPendingMessages(pendingMessages) {
  const now = Date.now();
  return (pendingMessages || []).filter(
    (pending) => !pending.timestamp || now - pending.timestamp < PENDING_MESSAGE_TTL_MS
  );
}

function pendingToMessage(pending) {
  return {
    id: `pending-${pending.id}`,
    role: "user",
    content: pending.content || "(attachment)",
    copyText: pending.copyText || pending.content,
    timestamp: pending.createdAt || new Date(pending.timestamp || Date.now()).toISOString(),
    images: pending.images || [],
    attachments: pending.attachments || [],
    isOptimistic: true,
    retryPayload: pending.retryPayload,
  };
}

function parseAttachmentAttributes(raw = "") {
  const attrs = {};
  const attrRegex = /([a-zA-Z_:-]+)="([^"]*)"/g;
  let match;
  while ((match = attrRegex.exec(raw)) !== null) {
    attrs[match[1]] = decodeXmlText(match[2]);
  }
  return attrs;
}

function decodeXmlText(value = "") {
  return value
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

function isSafeGeneratedImageDataUrl(dataUrl) {
  return typeof dataUrl === "string" && /^data:image\//i.test(dataUrl);
}

function getRememberedGeneratedImage(threadId, eventId) {
  if (!threadId || !eventId) return null;
  const images = generatedImagesByThread.get(threadId);
  return images?.find((image) => image.eventId === eventId) || null;
}
