import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

// Keep this set focused on the languages most commonly emitted in assistant
// responses. Highlight.js aliases cover variants such as js/jsx, ts/tsx,
// sh/shell, html, and cs; unsupported fences remain readable plain code.
export const SUPPORTED_LANGUAGES = {
  bash,
  c,
  cpp,
  csharp,
  css,
  diff,
  go,
  java,
  javascript,
  json,
  kotlin,
  markdown,
  python,
  ruby,
  rust,
  sql,
  typescript,
  xml,
  yaml,
};

for (const [name, language] of Object.entries(SUPPORTED_LANGUAGES)) {
  hljs.registerLanguage(name, language);
}

export function highlightCodeBlocks(root: ParentNode): void {
  root.querySelectorAll<HTMLElement>("pre code").forEach((codeElement) => {
    if (codeElement.dataset.highlighted === "yes") return;
    try {
      hljs.highlightElement(codeElement);
    } catch {
      // Unknown languages and malformed code stay readable as plain text.
    }
  });
}
