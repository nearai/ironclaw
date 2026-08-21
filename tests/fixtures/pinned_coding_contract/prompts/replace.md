Performs a single string replacement in a file with fuzzy whitespace matching.

<instruction>
- You MUST use the smallest `old_string` that uniquely identifies the change
- If `old_string` is not unique, you MUST expand it with more context or use `replace_all: true` to replace all occurrences
- Use `replace_all: true` when renaming a string across the file
- You SHOULD prefer editing existing files over creating new ones
</instruction>

<output>
Returns success/failure status. On success, file modified in place with replacement applied. On failure (e.g., `old_string` not found or matches multiple locations without `replace_all: true`), returns error describing issue.
</output>

<critical>
- You MUST read the file at least once in the conversation before editing. Tool errors if you attempt edit without reading file first.
</critical>

<bash-alternatives>
Replace is content-addressed — you identify *what* to change by its text.

For pattern-addressed bulk changes, bash is more efficient:

|Operation|Command|
|---|---|
|Regex replace|`sd 'pattern' 'replacement' file`|
|Bulk replace across files|`sd 'pattern' 'replacement' **/*.ts`|

Use Replace when _content itself_ identifies location; use `ast_edit` for structure-aware codemods.
For in-place edits prefer this tool or `write` — you get a diff preview and fuzzy matching.
</bash-alternatives>
