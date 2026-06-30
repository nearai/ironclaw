import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// `renderMarkdown` is a pure string->string function gated on the
// browser globals `window.marked` / `window.DOMPurify` / `document`.
// It is now memo-cached by the renderer (markdown-renderer.js), so the
// security-relevant invariant — every parsed payload passes through
// DOMPurify.sanitize — must be pinned so a future change to the memo
// deps or to this function cannot silently drop sanitization.
//
// Follow the crate's vm-context harness: stub the globals per test so
// each case is isolated and the real DOM/CDN libraries are never needed.
function loadRenderMarkdown(globals) {
  let source = readFileSync(new URL("./markdown.js", import.meta.url), "utf8");
  source = source.replace("export function renderMarkdown", "function renderMarkdown");
  source += "\nglobalThis.__testExports = { renderMarkdown };";
  const context = { ...globals, globalThis: {} };
  vm.runInNewContext(source, context);
  return context.globalThis.__testExports.renderMarkdown;
}

test("renderMarkdown routes parsed HTML through DOMPurify.sanitize, stripping handlers", () => {
  const calls = { parse: [], sanitize: [] };
  const renderMarkdown = loadRenderMarkdown({
    window: {
      marked: {
        // Pass the dangerous markup straight through so the only thing
        // that can strip it is the sanitize step.
        parse: (content, opts) => {
          calls.parse.push({ content, opts });
          return `<p>${content}</p>`;
        },
      },
      DOMPurify: {
        sanitize: (raw) => {
          calls.sanitize.push(raw);
          return raw.replace(/ onerror="[^"]*"/g, "");
        },
      },
    },
  });

  const out = renderMarkdown('<img src=x onerror="alert(1)">');

  assert.equal(calls.parse.length, 1, "content is parsed once");
  // Field-wise rather than deepEqual: the opts object is created inside
  // the vm realm, so its prototype differs and a strict deep-equal would
  // reject an otherwise-identical object.
  assert.equal(calls.parse[0].opts.gfm, true, "marked is called with gfm: true");
  assert.equal(calls.parse[0].opts.breaks, true, "marked is called with breaks: true");
  assert.equal(calls.sanitize.length, 1, "parsed output passes through sanitize exactly once");
  assert.equal(
    calls.sanitize[0],
    '<p><img src=x onerror="alert(1)"></p>',
    "sanitize receives the PARSED HTML, not the raw input — order is parse-then-sanitize",
  );
  assert.ok(!out.includes("onerror"), "the dangerous handler is stripped by the sanitize pass");
  assert.equal(out, "<p><img src=x></p>", "renderMarkdown returns sanitize's output, never raw markup");
});

test("renderMarkdown escapes via textContent when marked/DOMPurify are absent", () => {
  // The fallback branch must escape rather than emit live markup. Mimic
  // the browser's textContent -> innerHTML HTML-escaping with a stub
  // element so we assert the branch routes through textContent, not
  // innerHTML.
  const renderMarkdown = loadRenderMarkdown({
    window: {},
    document: {
      createElement: () => {
        const el = { _html: "" };
        Object.defineProperty(el, "textContent", {
          set(value) {
            el._html = String(value)
              .replace(/&/g, "&amp;")
              .replace(/</g, "&lt;")
              .replace(/>/g, "&gt;");
          },
        });
        Object.defineProperty(el, "innerHTML", {
          get() {
            return el._html;
          },
        });
        return el;
      },
    },
  });

  const out = renderMarkdown("<script>alert(1)</script>");
  assert.equal(
    out,
    "&lt;script&gt;alert(1)&lt;/script&gt;",
    "fallback escapes markup instead of returning live HTML",
  );
  assert.ok(!out.includes("<script>"));
});

test("renderMarkdown returns an empty string for falsy content", () => {
  const renderMarkdown = loadRenderMarkdown({
    window: { marked: { parse: () => "X" }, DOMPurify: { sanitize: () => "X" } },
  });
  assert.equal(renderMarkdown(""), "");
  assert.equal(renderMarkdown(null), "");
  assert.equal(renderMarkdown(undefined), "");
});
