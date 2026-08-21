Searches files and internal URLs with Rust regex plus PCRE2 fallback.

<instruction>
- Scope `path` to known files, directories, globs, or internal URLs; separate roots with `;`.
- Broad searches can time out; scope them narrowly or use `glob` first.
- One-file line selector: `src/foo.ts:50-100` (selectors never choose the search root).
- Literal `\n` or `\\n` enables cross-line patterns.
</instruction>

<critical>
- MUST use this instead of shell `grep`/`rg`.
- Open-ended multi-round search MUST use {{#if scoutAvailable}}Task + scout,{{else}}Task,{{/if}} not chained calls.
</critical>
