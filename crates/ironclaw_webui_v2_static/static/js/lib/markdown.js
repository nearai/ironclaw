export function renderMarkdown(content) {
  if (!content) return "";
  if (!window.marked || !window.DOMPurify) {
    const div = document.createElement("div");
    div.textContent = content;
    return div.innerHTML;
  }
  const raw = window.marked.parse(content, { gfm: true, breaks: true });
  return window.DOMPurify.sanitize(raw);
}
