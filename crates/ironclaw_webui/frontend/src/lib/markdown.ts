import DOMPurify from "dompurify";
import { marked } from "marked";
import {
  workspaceFileHrefFromPath,
  workspaceFilePathFromHref,
} from "./workspace-file-links";

const sanitizers = new Map<boolean, ReturnType<typeof DOMPurify>>();

// Normalize the product's scoped `sandbox:/workspace/...` references before
// DOMPurify applies its URI allowlist. Only the strict workspace-file grammar is
// rewritten; other custom protocols remain stripped.
//
// After sanitization, mark workspace anchors for the chat renderer's delegated
// click handler. Keep the existing external-link hardening attributes as a safe
// fallback for renderer call sites that do not install that handler.
function createSanitizer(workspaceFileLinks: boolean) {
  // Each capability mode owns an isolated DOMPurify instance. Hook behavior is
  // closure-captured and cannot be changed by another render.
  const sanitizer = DOMPurify(window);
  sanitizer.addHook("uponSanitizeAttribute", (node, data) => {
    if (
      !workspaceFileLinks ||
      node.tagName !== "A" ||
      data.attrName !== "href"
    ) {
      return;
    }
    const workspacePath = workspaceFilePathFromHref(data.attrValue);
    const canonicalHref = workspaceFileHrefFromPath(workspacePath);
    if (canonicalHref) data.attrValue = canonicalHref;
  });
  sanitizer.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName !== "A") return;
    // DOMPurify intentionally preserves data-* attributes, so remove any
    // assistant-authored preview metadata before deriving the trusted value.
    node.removeAttribute("data-workspace-path");
    const href = node.getAttribute("href");
    if (!href) return;
    if (workspaceFileLinks) {
      const workspacePath = workspaceFilePathFromHref(href);
      if (workspacePath) {
        node.setAttribute("data-workspace-path", workspacePath);
      }
    }
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  });
  return sanitizer;
}

function sanitizerForWorkspaceFileLinks(workspaceFileLinks: boolean) {
  const existing = sanitizers.get(workspaceFileLinks);
  if (existing) return existing;
  const sanitizer = createSanitizer(workspaceFileLinks);
  sanitizers.set(workspaceFileLinks, sanitizer);
  return sanitizer;
}

type RenderMarkdownOptions = {
  workspaceFileLinks?: boolean;
};

export function renderMarkdown(
  content: string | null | undefined,
  { workspaceFileLinks = false }: RenderMarkdownOptions = {},
): string {
  if (!content) return "";
  const raw = marked.parse(content, {
    async: false,
    gfm: true,
    breaks: true,
  }) as string;
  return sanitizerForWorkspaceFileLinks(workspaceFileLinks).sanitize(raw);
}
