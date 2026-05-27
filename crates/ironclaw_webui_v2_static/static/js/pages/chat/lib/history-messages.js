// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Kept narrow: the v2
// timeline contract has no attachments, no generated images, no
// turn-grouping metadata — those v1 affordances are out of scope
// for issue #3886.

export function messagesFromTimeline(records, pendingMessages = []) {
  const seen = new Set();
  const messages = [];

  for (const record of records || []) {
    const id = `msg-${record.message_id}`;
    if (seen.has(id)) continue;
    seen.add(id);
    messages.push({
      id,
      role: roleForRecord(record),
      content: record.content || "",
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status: record.status,
      sequence: record.sequence,
      turnRunId: record.turn_run_id || null,
    });
  }

  for (const pending of pendingMessages) {
    if (seen.has(pending.id)) continue;
    messages.push(pending);
  }

  return messages;
}

function roleForRecord(record) {
  switch (record.kind) {
    case "user":
    case "user_message":
      return "user";
    case "assistant":
    case "assistant_message":
    case "tool_result":
      return "assistant";
    case "system":
      return "system";
    default:
      return record.actor_id ? "user" : "assistant";
  }
}

function timestampForRecord(record) {
  // ThreadMessageRecord has no top-level timestamp; surfaces use
  // the sequence ordering for now. Browsers render the wall-clock
  // when an event arrives (FinalReplyView.generated_at).
  return record.received_at || record.created_at || null;
}
