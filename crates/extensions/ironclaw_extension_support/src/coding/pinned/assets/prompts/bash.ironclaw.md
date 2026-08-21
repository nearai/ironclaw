Runs commands in a shell.

Use ONLY for one binary or a short pipeline that computes a fact (`wc -l`, `sort | uniq -c`, `diff`).
Inline scripts, heredocs, `$(…)`, and complex control flow → a purpose-built tool or checked-in script.

<instruction>
- Set `cwd` instead of `cd`; use `env: { NAME: "…" }` for multiline/quote-heavy values.
- Order-dependent commands use `&&` in one call; independent calls may run concurrently.
- Internal URIs (`skill://`, `agent://`, …) auto-resolve to paths.
</instruction>

<critical>
- NEVER use shell `grep`/`rg`; use built-in `grep`.
- List directories with `read` and find paths with `glob`; NEVER use `ls`/`find`.
- Avoid `head`, `tail`, and redirection: output is captured, truncated, and linked as `artifact://<id>`.
</critical>

No truncation footer means the displayed output is complete.
