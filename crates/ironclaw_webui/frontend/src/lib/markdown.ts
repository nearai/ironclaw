import DOMPurify from "dompurify";
import { marked } from "marked";
import {
  workspaceFileHrefFromPath,
  workspaceFilePathFromHref,
} from "./workspace-file-links";

// Normalize the product's scoped `sandbox:/workspace/...` references before
// DOMPurify applies its URI allowlist. Only the strict workspace-file grammar is
// rewritten; other custom protocols remain stripped.
//
// After sanitization, mark workspace anchors for the chat renderer's delegated
// click handler. Keep the existing external-link hardening attributes as a safe
// fallback for renderer call sites that do not install that handler.
function createSanitizer() {
  const sanitizer = DOMPurify(window);
  const validatedWorkspacePaths = new WeakMap<Element, string>();
  sanitizer.addHook("uponSanitizeAttribute", (node, data, config) => {
    const workspaceFileLinks = Boolean(
      (config as { workspaceFileLinks?: boolean } | undefined)
        ?.workspaceFileLinks,
    );
    if (
      !workspaceFileLinks ||
      node.tagName !== "A" ||
      data.attrName !== "href"
    ) {
      return;
    }
    const workspacePath = workspaceFilePathFromHref(data.attrValue);
    const canonicalHref = workspaceFileHrefFromPath(workspacePath);
    if (canonicalHref && workspacePath) {
      data.attrValue = canonicalHref;
      validatedWorkspacePaths.set(node, workspacePath);
    }
  });
  sanitizer.addHook("afterSanitizeAttributes", (node, _data, config) => {
    const workspaceFileLinks = Boolean(
      (config as { workspaceFileLinks?: boolean } | undefined)
        ?.workspaceFileLinks,
    );
    if (node.tagName !== "A") return;
    // DOMPurify intentionally preserves data-* attributes, so remove any
    // assistant-authored preview metadata before deriving the trusted value.
    node.removeAttribute("data-workspace-path");
    const href = node.getAttribute("href");
    if (!href) {
      validatedWorkspacePaths.delete(node);
      return;
    }
    if (workspaceFileLinks) {
      const workspacePath = validatedWorkspacePaths.get(node);
      if (workspacePath) {
        node.setAttribute("data-workspace-path", workspacePath);
      }
    }
    validatedWorkspacePaths.delete(node);
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  });
  return sanitizer;
}

const sanitizer = createSanitizer();

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
  return String(sanitizer.sanitize(raw, { workspaceFileLinks } as never));
}
