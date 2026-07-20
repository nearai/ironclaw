import DOMPurify from "dompurify";
import { marked } from "marked";

let linkHookInstalled = false;

// Open every rendered link in a new tab so clicking a PR / diff / issue
// link in a response never navigates away from the active conversation.
// Registered once against the shared DOMPurify instance; the hook runs
// on every sanitize pass after attributes are processed.
function ensureLinkTargetHook(): void {
  if (linkHookInstalled) return;
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A" && node.getAttribute("href")) {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
  });
  linkHookInstalled = true;
}

export function renderMarkdown(content: string | null | undefined): string {
  if (!content) return "";
  ensureLinkTargetHook();
  const raw = marked.parse(content, { async: false, gfm: true, breaks: true }) as string;
  return DOMPurify.sanitize(raw);
}
