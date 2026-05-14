import { React, html } from "../../../lib/html.js";
import { renderMarkdown } from "../../../lib/markdown.js";

export function MarkdownRenderer({ content, className = "" }) {
  const ref = React.useRef(null);

  React.useEffect(() => {
    if (ref.current && window.hljs) {
      ref.current.querySelectorAll("pre code").forEach((block) => {
        window.hljs.highlightElement(block);
      });
    }
  }, [content]);

  return html`
    <div
      ref=${ref}
      className=${["markdown-body", className].join(" ")}
      dangerouslySetInnerHTML=${{ __html: renderMarkdown(content) }}
    />
  `;
}
