# google-docs.read_excerpt

Use this when a whole Google Doc would be too large or the user asks about a specific section.

Prefer this over `google-docs.read_content` unless the user explicitly needs the full document. Use `query` for the section/topic when known; otherwise use `start_char` for pagination through a document.

The result includes `truncated_before`, `truncated_after`, and a compact outline so you can fetch another excerpt if needed.
