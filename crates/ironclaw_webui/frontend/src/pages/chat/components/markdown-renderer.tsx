// @ts-nocheck
import hljs from "highlight.js/lib/common";
import React from "react";
import { renderMarkdown } from "../../../lib/markdown";
import { toast } from "../../../lib/toast";
import { useT } from "../../../lib/i18n";

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
  if (!root) return;
  const labels = codeBlockLabels(t);
  root.querySelectorAll("pre").forEach((pre) => {
    if (pre.dataset.enhanced === "1") {
      syncCodeBlockLabels(pre, labels);
      return;
    }
    pre.dataset.enhanced = "1";
    pre.dataset.wrapped = "0";

    const codeEl = pre.querySelector("code");
    if (codeEl) {
      try {
        hljs.highlightElement(codeEl);
      } catch {
        // highlight failure is non-fatal
      }
    }

    const wrap = document.createElement("div");
    wrap.className = "markdown-code-frame";
    pre.parentNode.insertBefore(wrap, pre);
    wrap.appendChild(pre);

    const bar = document.createElement("div");
    bar.style.cssText =
      "position:absolute;top:6px;right:6px;display:flex;gap:4px;opacity:0";
    wrap.addEventListener("mouseenter", () => (bar.style.opacity = "1"));
    wrap.addEventListener("mouseleave", () => (bar.style.opacity = "0"));

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
    wrapBtn.addEventListener("click", () => {
      const wrapped = pre.dataset.wrapped !== "1";
      pre.dataset.wrapped = wrapped ? "1" : "0";
      pre.style.whiteSpace = wrapped ? "pre-wrap" : "";
      wrapBtn.textContent = wrapped
        ? wrapBtn.dataset.labelNoWrap || labels.noWrap
        : wrapBtn.dataset.labelWrap || labels.wrap;
    });

    const copyBtn = mkBtn(labels.copy);
    copyBtn.dataset.codeBlockRole = "copy";
    copyBtn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(codeEl ? codeEl.innerText : pre.innerText);
        copyBtn.dataset.copied = "1";
        copyBtn.textContent = copyBtn.dataset.labelCopied || labels.copied;
        toast(copyBtn.dataset.labelCodeCopied || labels.codeCopied, { tone: "success" });
        setTimeout(() => {
          copyBtn.dataset.copied = "0";
          copyBtn.textContent = copyBtn.dataset.labelCopy || labels.copy;
        }, 1400);
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
      toggle.addEventListener("click", () => {
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

function MarkdownRendererImpl({ content, className = "" }) {
  const t = useT();
  const ref = React.useRef(null);

  // marked.parse + DOMPurify.sanitize are expensive; only re-run when
  // the source content actually changes, not on every parent render
  // (during streaming the message list re-renders on every token).
  const rendered = React.useMemo(() => renderMarkdown(content), [content]);

  React.useEffect(() => {
    enhanceCodeBlocks(ref.current, t);
  }, [rendered, t]);

  return (
    <div
      ref={ref}
      className={["markdown-body", className].join(" ")}
      dangerouslySetInnerHTML={{ __html: rendered }}
    />
  );
}

// Memoized so a bubble whose `content`/`className` are unchanged skips
// re-rendering when sibling messages update (e.g. a new streaming chunk
// elsewhere in the list).
export const MarkdownRenderer = React.memo(MarkdownRendererImpl);
