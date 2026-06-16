import { React, html } from "../../../lib/html.js";
import { renderMarkdown } from "../../../lib/markdown.js";
import { toast } from "../../../lib/toast.js";

const COLLAPSE_PX = 360;

/* Enhance rendered <pre> code blocks in place: syntax highlight, a hover
   toolbar (copy + soft-wrap toggle), and collapse for very tall blocks.
   Runs imperatively because the markdown is injected via innerHTML. */
function enhanceCodeBlocks(root) {
  if (!root) return;
  root.querySelectorAll("pre").forEach((pre) => {
    if (pre.dataset.enhanced === "1") return;
    pre.dataset.enhanced = "1";

    const codeEl = pre.querySelector("code");
    if (window.hljs && codeEl) {
      try {
        window.hljs.highlightElement(codeEl);
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

    let wrapped = false;
    const wrapBtn = mkBtn("Wrap");
    wrapBtn.addEventListener("click", () => {
      wrapped = !wrapped;
      pre.style.whiteSpace = wrapped ? "pre-wrap" : "";
      wrapBtn.textContent = wrapped ? "No wrap" : "Wrap";
    });

    const copyBtn = mkBtn("Copy");
    copyBtn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(codeEl ? codeEl.innerText : pre.innerText);
        copyBtn.textContent = "Copied";
        toast("Code copied", { tone: "success" });
        setTimeout(() => (copyBtn.textContent = "Copy"), 1400);
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
      let expanded = false;
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.textContent = "Show more";
      toggle.style.cssText =
        "display:block;width:100%;text-align:center;font-family:var(--font-mono,monospace);font-size:11px;color:var(--v2-accent-text);background:var(--v2-surface-soft);border:0;border-top:1px solid var(--v2-panel-border);padding:5px;cursor:pointer";
      toggle.addEventListener("click", () => {
        expanded = !expanded;
        pre.style.maxHeight = expanded ? "none" : `${COLLAPSE_PX}px`;
        pre.style.overflowY = expanded ? "visible" : "hidden";
        toggle.textContent = expanded ? "Show less" : "Show more";
      });
      wrap.appendChild(toggle);
    }
  });
}

function MarkdownRendererImpl({ content, className = "" }) {
  const ref = React.useRef(null);

  // marked.parse + DOMPurify.sanitize are expensive; only re-run when
  // the source content actually changes, not on every parent render
  // (during streaming the message list re-renders on every token).
  const rendered = React.useMemo(() => renderMarkdown(content), [content]);

  React.useEffect(() => {
    enhanceCodeBlocks(ref.current);
  }, [rendered]);

  return html`
    <div
      ref=${ref}
      className=${["markdown-body", className].join(" ")}
      dangerouslySetInnerHTML=${{ __html: rendered }}
    />
  `;
}

// Memoized so a bubble whose `content`/`className` are unchanged skips
// re-rendering when sibling messages update (e.g. a new streaming chunk
// elsewhere in the list).
export const MarkdownRenderer = React.memo(MarkdownRendererImpl);
