import DOMPurify from "dompurify";
import { marked } from "marked";
import { workspaceFilePathFromHref } from "./workspace-file-links";

let linkHooksInstalled = false;

// Normalize the product's scoped `sandbox:/workspace/...` references before
// DOMPurify applies its URI allowlist. Only the strict workspace-file grammar is
// rewritten; other custom protocols remain stripped.
//
// After sanitization, mark workspace anchors for the chat renderer's delegated
// click handler. Keep the existing external-link hardening attributes as a safe
// fallback for renderer call sites that do not install that handler.
function ensureLinkHooks(): void {
  if (linkHooksInstalled) return;
  DOMPurify.addHook("uponSanitizeAttribute", (node, data) => {
    if (node.tagName !== "A" || data.attrName !== "href") return;
    const workspacePath = workspaceFilePathFromHref(data.attrValue);
    if (workspacePath) data.attrValue = workspacePath;
  });
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName !== "A") return;
    const href = node.getAttribute("href");
    if (!href) return;
    const workspacePath = workspaceFilePathFromHref(href);
    if (workspacePath) {
      node.setAttribute("data-workspace-path", workspacePath);
    }
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  });
  linkHooksInstalled = true;
}

export function renderMarkdown(content: string | null | undefined): string {
  if (!content) return "";
  ensureLinkHooks();
  const raw = marked.parse(content, { async: false, gfm: true, breaks: true }) as string;
  return DOMPurify.sanitize(raw);
}
