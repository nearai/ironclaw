// Pure slash-command helpers for the composer. Everything is derived from the
// server inventory (`listChatCommands`) — the frontend never hardcodes a
// command name or renders a command-specific shape.

// Returns the inventory descriptor the draft's first token names (by
// canonical name), or null when the text is not a known slash command and
// should submit as an ordinary message.
export function matchCommand(text, commands) {
  const trimmed = (text || "").trim();
  if (!trimmed.startsWith("/")) return null;
  const first = trimmed.slice(1).split(/\s+/, 1)[0]?.toLowerCase();
  if (!first) return null;
  return (commands || []).find((command) => command.name === first) || null;
}

// Inventory rows whose name starts with the draft's command prefix — the
// derived composer menu.
export function commandMenuMatches(text, commands) {
  const trimmed = (text || "").trimStart();
  if (!trimmed.startsWith("/")) return [];
  // Once arguments follow a complete command word, stop suggesting.
  if (/\s/.test(trimmed)) return [];
  const prefix = trimmed.slice(1).toLowerCase();
  return (commands || []).filter((command) => command.name.startsWith(prefix));
}

// Render an executed command's response as markdown for the SYSTEM notice
// bubble: one generic shape for every command (title, label/value fields,
// plain lines) or the rejection message.
export function renderCommandResultMarkdown(response) {
  if (response?.rejection?.message) return response.rejection.message;
  const view = response?.result;
  if (!view) return `/${response?.command ?? ""} completed.`;
  const parts = [];
  if (view.title) parts.push(`**${view.title}**`);
  for (const field of view.fields || []) {
    parts.push(`${field.label}: ${field.value}`);
  }
  for (const line of view.lines || []) {
    parts.push(line);
  }
  return parts.join("\n");
}
