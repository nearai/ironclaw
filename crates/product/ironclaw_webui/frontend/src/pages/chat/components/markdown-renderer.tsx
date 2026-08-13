// @ts-nocheck
import React from "react";
import "streamdown/styles.css";
import { toast } from "../../../lib/toast";
import { useT } from "../../../lib/i18n";
import { workspaceFilePathFromHref } from "../../../lib/workspace-file-links";

const COLLAPSE_PX = 360;

/* Enhance rendered <pre> code blocks in place: syntax highlight, a hover
   toolbar (copy + soft-wrap toggle), and collapse for very tall blocks.
   Runs imperatively because the markdown is injected via innerHTML. */
function codeBlockLabels(t) {
  return {
    copy: t("common.copy"),
    copied: t("common.copied"),
    codeCopied: t("markdown.codeCopied"),
    wrap: t("markdown.wrap"),
    noWrap: t("markdown.noWrap"),
    showMore: t("markdown.showMore"),
    showLess: t("markdown.showLess"),
  };
}

function enhanceCodeBlocks(root, t) {
  if (!root) return () => {};
  const labels = codeBlockLabels(t);
  const cleanups = [];
  const resetTimers = new Set();
  const listen = (target, type, handler) => {
    target.addEventListener(type, handler);
    cleanups.push(() => target.removeEventListener(type, handler));
  };
  const resetCopyLabelLater = (button) => {
    const timer = setTimeout(() => {
      resetTimers.delete(timer);
      button.dataset.copied = "0";
      button.textContent = button.dataset.labelCopy || labels.copy;
    }, 1400);
    resetTimers.add(timer);
  };
  root.querySelectorAll("pre").forEach((pre) => {
    if (pre.dataset.enhanced === "1") {
      syncCodeBlockLabels(pre, labels);
      return;
    }
    pre.dataset.enhanced = "1";
    pre.dataset.wrapped = "0";

    const codeEl = pre.querySelector("code");

    const wrap = document.createElement("div");
    wrap.className = "markdown-code-frame";
    pre.parentNode.insertBefore(wrap, pre);
    wrap.appendChild(pre);

    const bar = document.createElement("div");
    bar.style.cssText =
      "position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0";
    listen(wrap, "mouseenter", () => (bar.style.opacity = "1"));
    listen(wrap, "mouseleave", () => (bar.style.opacity = "0"));

    const mkBtn = (label) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.style.cssText =
        "font-family:var(--font-mono,monospace);font-size:11px;border:1px solid var(--v2-panel-border);background:var(--v2-surface);color:var(--v2-text-muted);border-radius:6px;padding:2px 7px;cursor:pointer";
      return b;
    };

    const wrapBtn = mkBtn(labels.wrap);
    wrapBtn.dataset.codeBlockRole = "wrap";
    listen(wrapBtn, "click", () => {
      const wrapped = pre.dataset.wrapped !== "1";
      pre.dataset.wrapped = wrapped ? "1" : "0";
      pre.style.whiteSpace = wrapped ? "pre-wrap" : "";
      wrapBtn.textContent = wrapped
        ? wrapBtn.dataset.labelNoWrap || labels.noWrap
        : wrapBtn.dataset.labelWrap || labels.wrap;
    });

    const copyBtn = mkBtn(labels.copy);
    copyBtn.dataset.codeBlockRole = "copy";
    listen(copyBtn, "click", async () => {
      try {
        await navigator.clipboard.writeText(codeEl ? codeEl.innerText : pre.innerText);
        copyBtn.dataset.copied = "1";
        copyBtn.textContent = copyBtn.dataset.labelCopied || labels.copied;
        toast(copyBtn.dataset.labelCodeCopied || labels.codeCopied, { tone: "success" });
        resetCopyLabelLater(copyBtn);
      } catch {
        // clipboard unavailable
      }
    });

    bar.appendChild(wrapBtn);
    bar.appendChild(copyBtn);
    wrap.appendChild(bar);

    if (pre.scrollHeight > COLLAPSE_PX) {
      pre.style.maxHeight = `${COLLAPSE_PX}px`;
      pre.style.overflowX = "auto";
      pre.style.overflowY = "hidden";
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.dataset.codeBlockRole = "expand";
      toggle.dataset.expanded = "0";
      toggle.textContent = labels.showMore;
      toggle.style.cssText =
        "display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer";
      listen(toggle, "click", () => {
        const expanded = toggle.dataset.expanded !== "1";
        toggle.dataset.expanded = expanded ? "1" : "0";
        pre.style.maxHeight = expanded ? "none" : `${COLLAPSE_PX}px`;
        pre.style.overflowY = expanded ? "visible" : "hidden";
        toggle.textContent = expanded
          ? toggle.dataset.labelShowLess || labels.showLess
          : toggle.dataset.labelShowMore || labels.showMore;
      });
      wrap.appendChild(toggle);
    }
    syncCodeBlockLabels(pre, labels);
  });
  return () => {
    cleanups.forEach((cleanup) => cleanup());
    resetTimers.forEach((timer) => clearTimeout(timer));
    resetTimers.clear();
  };
}

function syncCodeBlockLabelsInRoot(root, t) {
  if (!root) return;
  const labels = codeBlockLabels(t);
  root.querySelectorAll("pre").forEach((pre) => {
    if (pre.dataset.enhanced === "1") syncCodeBlockLabels(pre, labels);
  });
}

function syncCodeBlockLabels(pre, labels) {
  const frame = pre.closest(".markdown-code-frame");
  if (!frame) return;
  const wrapBtn = frame.querySelector('[data-code-block-role="wrap"]');
  if (wrapBtn) {
    wrapBtn.dataset.labelWrap = labels.wrap;
    wrapBtn.dataset.labelNoWrap = labels.noWrap;
    wrapBtn.textContent = pre.dataset.wrapped === "1" ? labels.noWrap : labels.wrap;
  }
  const copyBtn = frame.querySelector('[data-code-block-role="copy"]');
  if (copyBtn) {
    copyBtn.dataset.labelCopy = labels.copy;
    copyBtn.dataset.labelCopied = labels.copied;
    copyBtn.dataset.labelCodeCopied = labels.codeCopied;
    copyBtn.textContent = copyBtn.dataset.copied === "1" ? labels.copied : labels.copy;
  }
  const toggle = frame.querySelector('[data-code-block-role="expand"]');
  if (toggle) {
    toggle.dataset.labelShowMore = labels.showMore;
    toggle.dataset.labelShowLess = labels.showLess;
    toggle.textContent = toggle.dataset.expanded === "1" ? labels.showLess : labels.showMore;
  }
}

function MarkdownRendererImpl({
  content,
  className = "",
  streaming = false,
  onWorkspaceFileOpen = undefined,
}) {
  const t = useT();
  const ref = React.useRef(null);
  const normalizedContent = typeof content === "string" ? content : "";
  const [rendered, setRendered] = React.useState(null);
  const latestContentRef = React.useRef(normalizedContent);
  const workspaceFileLinksEnabled =
    typeof onWorkspaceFileOpen === "function";
  const latestWorkspaceFileLinksEnabledRef =
    React.useRef(workspaceFileLinksEnabled);
  latestWorkspaceFileLinksEnabledRef.current = workspaceFileLinksEnabled;
  const mountedRef = React.useRef(true);
  const renderInFlightRef = React.useRef(false);
  const markdownLoadFailedRef = React.useRef(false);
  const requestRenderRef = React.useRef(() => false);
  const wasStreamingRef = React.useRef(streaming);
  if (streaming) wasStreamingRef.current = true;
  const handleClick = React.useCallback(
    (event) => {
      if (typeof onWorkspaceFileOpen !== "function") return;
      const target = event.target;
      const anchor =
        target instanceof Element
          ? target.closest("a[data-workspace-path]")
          : null;
      const path = anchor?.getAttribute("data-workspace-path");
      const hrefPath = workspaceFilePathFromHref(anchor?.getAttribute("href"));
      if (!path || !hrefPath || hrefPath !== path) return;
      event.preventDefault();
      onWorkspaceFileOpen(path);
    },
    [onWorkspaceFileOpen],
  );

  const renderedHtml =
    normalizedContent && rendered &&
    rendered.source === normalizedContent &&
    rendered.workspaceFileLinks === workspaceFileLinksEnabled
      ? rendered.html
      : null;

  requestRenderRef.current = () => {
    if (
      !latestContentRef.current ||
      renderInFlightRef.current ||
      markdownLoadFailedRef.current
    ) {
      return false;
    }
    renderInFlightRef.current = true;
    import("../../../lib/markdown")
      .then(({ renderMarkdown }) => {
        const currentContent = latestContentRef.current;
        if (!mountedRef.current || !currentContent) return;
        const currentWorkspaceFileLinksEnabled =
          latestWorkspaceFileLinksEnabledRef.current;
        setRendered({
          source: currentContent,
          workspaceFileLinks: currentWorkspaceFileLinksEnabled,
          html: renderMarkdown(currentContent, {
            workspaceFileLinks: currentWorkspaceFileLinksEnabled,
          }),
        });
      })
      .catch(() => {
        markdownLoadFailedRef.current = true;
        if (mountedRef.current) setRendered(null);
      })
      .finally(() => {
        renderInFlightRef.current = false;
      });
    return true;
  };

  // Streaming projections carry the full accumulated reply. The product
  // boundary limits replaceable text snapshots to browser-paint cadence, so
  // Streamdown can reconcile incomplete Markdown without a transition being
  // continually superseded by provider microbursts. Completed replies keep the
  // existing marked + DOMPurify path and code-block enhancements.
  React.useEffect(() => {
    latestContentRef.current = normalizedContent;
  }, [normalizedContent]);

  React.useEffect(() => {
    if (!normalizedContent) {
      setRendered(null);
      return;
    }

    if (streaming) return;
    requestRenderRef.current();
  }, [normalizedContent, streaming, workspaceFileLinksEnabled]);

  React.useEffect(() => {
    if (streaming || renderedHtml === null) return undefined;
    const root = ref.current;
    const cleanupCodeBlocks = enhanceCodeBlocks(root, t);
    if (!root?.querySelector("pre code")) return cleanupCodeBlocks;

    let active = true;
    import("../../../lib/syntax-highlighting")
      .then(({ highlightCodeBlocks }) => {
        if (active && ref.current === root) highlightCodeBlocks(root);
      })
      .catch(() => {
        // Syntax highlighting is an optional enhancement.
      });
    return () => {
      active = false;
      cleanupCodeBlocks();
    };
  }, [renderedHtml, streaming]);

  React.useEffect(() => {
    if (streaming || renderedHtml === null) return;
    syncCodeBlockLabelsInRoot(ref.current, t);
  }, [renderedHtml, streaming, t]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const keepStreamingRendererUntilFinalIsReady =
    !streaming && wasStreamingRef.current && renderedHtml === null;
  if (streaming || keepStreamingRendererUntilFinalIsReady) {
    return (
      <div ref={ref} className={["markdown-body", className].join(" ")}>
        <React.Suspense
          fallback={(
            <div className="whitespace-pre-wrap">{normalizedContent}</div>
          )}
        >
          <EmojiStreamingMarkdown
            animated={{ duration: 100, easing: "ease-out", sep: "word", stagger: 15 }}
            controls={false}
            isAnimating={streaming}
            mode={streaming ? "streaming" : "static"}
          >
            {normalizedContent}
          </EmojiStreamingMarkdown>
        </React.Suspense>
      </div>
    );
  }

  if (renderedHtml === null) {
    return (
      <div
        ref={ref}
        onClick={handleClick}
        className={["markdown-body", "whitespace-pre-wrap", className].join(" ")}
      >
        {normalizedContent}
      </div>
    );
  }

  return (
    <div
      ref={ref}
      onClick={handleClick}
      className={["markdown-body", className].join(" ")}
      dangerouslySetInnerHTML={{ __html: renderedHtml }}
    />
  );
}

const EmojiStreamingMarkdown = React.lazy(() =>
  import("./emoji-streaming-markdown").then(({ EmojiStreamingMarkdown }) => ({
    default: EmojiStreamingMarkdown,
  }))
);

// Memoized so a bubble whose `content`/`className`/`streaming` are unchanged skips
// re-rendering when sibling messages update (e.g. a new streaming chunk
// elsewhere in the list).
export const MarkdownRenderer = React.memo(MarkdownRendererImpl);
