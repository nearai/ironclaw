import DOMPurify from "dompurify";
import { marked } from "marked";
import {
  workspaceFileHrefFromPath,
  workspaceFilePathFromHref,
} from "./workspace-file-links";

let linkHooksInstalled = false;
let workspaceFileLinksEnabled = false;

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
    if (
      !workspaceFileLinksEnabled ||
      node.tagName !== "A" ||
      data.attrName !== "href"
    ) {
      return;
    }
    const workspacePath = workspaceFilePathFromHref(data.attrValue);
    const canonicalHref = workspaceFileHrefFromPath(workspacePath);
    if (canonicalHref) data.attrValue = canonicalHref;
  });
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName !== "A") return;
    // DOMPurify intentionally preserves data-* attributes, so remove any
    // assistant-authored preview metadata before deriving the trusted value.
    node.removeAttribute("data-workspace-path");
    const href = node.getAttribute("href");
    if (!href) return;
    if (workspaceFileLinksEnabled) {
      const workspacePath = workspaceFilePathFromHref(href);
      if (workspacePath) {
        node.setAttribute("data-workspace-path", workspacePath);
      }
    }
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  });
  linkHooksInstalled = true;
}

type RenderMarkdownOptions = {
  workspaceFileLinks?: boolean;
};

export function renderMarkdown(
  content: string | null | undefined,
  { workspaceFileLinks = false }: RenderMarkdownOptions = {},
): string {
  if (!content) return "";
  ensureLinkHooks();
  const previousWorkspaceFileLinksEnabled = workspaceFileLinksEnabled;
  workspaceFileLinksEnabled = workspaceFileLinks;
  try {
    const raw = marked.parse(content, {
      async: false,
      gfm: true,
      breaks: true,
    }) as string;
    return DOMPurify.sanitize(raw);
  } finally {
    workspaceFileLinksEnabled = previousWorkspaceFileLinksEnabled;
  }
}
