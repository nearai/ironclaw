let linkHookInstalled = false;

// Open every rendered link in a new tab so clicking a PR / diff / issue
// link in a response never navigates away from the active conversation.
// Registered once against the shared DOMPurify instance; the hook runs
// on every sanitize pass after attributes are processed.
function ensureLinkTargetHook() {
  if (linkHookInstalled || !window.DOMPurify) return;
  window.DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A" && node.getAttribute("href")) {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
  });
  linkHookInstalled = true;
}

export function renderMarkdown(content) {
  if (!content) return "";
  if (!window.marked || !window.DOMPurify) {
    const div = document.createElement("div");
    div.textContent = content;
    return div.innerHTML;
  }
  ensureLinkTargetHook();
  const raw = window.marked.parse(content, { gfm: true, breaks: true });
  return window.DOMPurify.sanitize(raw);
}
