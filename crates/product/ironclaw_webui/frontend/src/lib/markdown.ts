import DOMPurify, {
  type DOMPurify as DOMPurifyInstance,
  type WindowLike,
} from "dompurify";
import { nameToEmoji } from "gemoji";
import { marked, type Token } from "marked";
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
function createSanitizer(currentWindow: WindowLike) {
  const sanitizer = DOMPurify(currentWindow);
  if (!sanitizer.isSupported) {
    throw new Error("Markdown sanitization is unavailable in this browser");
  }
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

const sanitizers = new WeakMap<object, DOMPurifyInstance>();

function sanitizerForCurrentWindow(): DOMPurifyInstance {
  const currentWindow = window;
  const existing = sanitizers.get(currentWindow);
  if (existing) return existing;
  const sanitizer = createSanitizer(currentWindow);
  sanitizers.set(currentWindow, sanitizer);
  return sanitizer;
}

type RenderMarkdownOptions = {
  workspaceFileLinks?: boolean;
};

const GEMOJI_SHORTCODE = /:(\+1|[-\w]+):/g;

function renderGemojiShortcodes(token: Token): void {
  if (token.type !== "text") return;
  token.text = token.text.replace(GEMOJI_SHORTCODE, (shortcode, name) =>
    Object.hasOwn(nameToEmoji, name) ? nameToEmoji[name] : shortcode,
  );
}

export function renderMarkdown(
  content: string | null | undefined,
  { workspaceFileLinks = false }: RenderMarkdownOptions = {},
): string {
  if (!content) return "";
  const raw = marked.parse(content, {
    async: false,
    gfm: true,
    breaks: true,
    walkTokens: renderGemojiShortcodes,
  }) as string;
  const sanitizer = sanitizerForCurrentWindow();
  return String(sanitizer.sanitize(raw, { workspaceFileLinks } as never));
}
