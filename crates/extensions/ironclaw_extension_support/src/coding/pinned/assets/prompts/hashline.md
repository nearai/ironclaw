Line-anchored patch language: name original lines/gaps to replace, insert, cut, or paste, then list new content. A header ending in `:` takes `+` body rows; colonless `PUT` (paste), `CUT`, `REM`, `MV` take none.

<headers>
Every file section starts `[PATH#TAG]`. `TAG` = 4-hex snapshot tag from your latest `read`/`search` — REQUIRED on every section. Create new files with `write`; hashline only edits existing files.
</headers>

<ops>
`PUT N.=M:` — replace original lines N through M (INCLUSIVE) with body rows.
`PUT N*:` — replace the syntactic block BEGINNING on line N; its closing line is resolved for you.
`PUT <N:` / `PUT >N:` — insert body rows before / after line N (`PUT <1:` = file head, `PUT >$:` = file tail).
`PUT >N*:` — insert body rows after the END of the block beginning at N (at sibling depth). Append inside a block → `PUT >M:`.
`PUT <N` / `PUT >N` / `PUT N.=M @name` / `PUT N* @name` — paste a captured register at a gap, over a range, or over a resolved block (no `:` header, no body rows). Unlabeled gap `PUT` pastes the anonymous register; span/block paste requires `@name`.
`CUT N.=M` / `CUT N*` — delete lines N through M / block N and capture them (anonymous, or `@name` when given).
`REM` — delete the whole section file. `MV DEST` — move/rename to `DEST` (quote paths with spaces); edits above `MV` land on the source first, final content written at `DEST`.
Single line: `PUT N.=N:` / `CUT N.=N`. Range = ORIGINAL lines touched (`N.=M`, inclusive); body length irrelevant.
</ops>

<body-rows>
Only under a `:` header. Every row is `+TEXT`, verbatim (leading whitespace kept); `+` alone = blank line. NEVER `-old` or bare/context rows — the range deletes; the body is only the final content. Keep a line: leave it out of every range. Literal leading `-`/`+` keeps the prefix: `- item` → `+- item`, `+ item` → `++ item`.
</body-rows>

<rules>
- Line numbers + `#TAG` come from your latest `read`/`search` (`LINE:TEXT` rows); numbers name ORIGINAL lines, never shifted by applied hunks.
- Applied edits renumber the file and change the `#TAG` — take the next edit's numbers from the edit response or a fresh `read`.
- Touch only displayed lines — hunks on undisplayed lines are REJECTED. Far from your read window? Re-`read`; confirm numbers map to the intended construct.
- Elided regions are UNSEEN (`…`/`..` markers, collapsed `N-M:` summary rows) — NEVER place or span a hunk inside one; `read` the range first.
- NEVER start or end a range mid-expression or mid-block.
- Ranges cover ONLY changed lines — never widen over keepers. Non-adjacent changes = separate hunks.
- Whole construct → `PUT N*:`; lines inside one → `PUT N.=M:`.
- `PUT N*:` resolves EXACTLY the node at N: leading decorators/attributes/doc-comments are separate nodes — point N at the FIRST decorator to sweep both; standalone line-comments are never swept (use `PUT N.=M:`).
- Block ops anchor the OPENING line of a MULTI-LINE construct — never the closer, last line, or a bare inner statement; one statement → plain op (`PUT N.=N:` / `CUT N.=N` / `PUT >N:`). Saw the closer? `PUT >M:`.
- Markdown: a heading IS a block opener — block ops on `##`/`###` resolve the WHOLE section (through deeper nested headings, up to the next same-or-higher heading). `PUT >N*:` after a section: end the body with a blank line to keep the next heading separated.
- Pure additions → `PUT <N:` / `PUT >N:`, never a widened `PUT N.=M:`.
- Move code with `CUT`+`PUT`: `CUT 5.=9 @fn` captures into `@fn`; `PUT >40 @fn` pastes it. Unlabeled `CUT` + `PUT >40` works for a single call-local move. Named registers persist across edit calls.
- NEVER format/restyle code with this tool; run the project formatter.
</rules>

<example>
`read` output shape:
```
[greet.py#A1B2]
1:def greet(name):
2:    msg = "Hello, " + name
3:    print(msg)
4:greet("world")
```

Edit, then move:
```
[greet.py#A1B2]
PUT 1.=3:
+def greet(name):
+    print(f"Hi, {name}")
MV lib/greet.py
```

Markdown bullets — the file receives `- task`:
```
[PLAN.md#A1B2]
PUT >2:
+- task
+  - nested task
```

Move `greet` to a sibling file using a named register — flows across sections:
```
[greet.py#A1B2]
CUT 1* @fn
[other.py#3C4D]
PUT <1 @fn
```

`PUT 1*:` resolves lines 1–3 (`def` header through `print(msg)`); line 4 is a separate statement and stays:
```
[greet.py#A1B2]
PUT 1*:
+def greet(name):
+    print(f"Hello, {name}")
```

Decorator/doc-comment = SEPARATE block — point N at the decorator to take both; anchoring the `def` (line 2) would orphan `@cache`:
```
[svc.py#C3D4]
PUT 1*:
+@cache
+def load(key):
+    return store[key]
```
</example>

<anti-patterns>
# WRONG — empty `PUT` to delete. RIGHT: `CUT 4.=4`
PUT 4.=4:

# WRONG — range sized to the post-edit content. RIGHT: `PUT 1.=1:` (body length irrelevant)
PUT 1.=2:
+def greet(name):

# WRONG — `-` rows / bare context lines do not exist; the range deletes, the body is only new content.
PUT 3.=3:
    msg = "Hello, " + name
-   print(msg)
+   return msg
# RIGHT
PUT 3.=3:
+   return msg

# WRONG — pure insertion as a widened `PUT`: retyped keepers get dropped (here line 4).
PUT 2.=4:
+    msg = "Hello, " + name
+    extra = compute(name)
+    print(msg)
# RIGHT — touch nothing you keep.
PUT >2:
+    extra = compute(name)

# WRONG — `PUT >N*:` anchored on the closing delimiter / last visible line. RIGHT: plain `PUT >M:`
PUT >3*:
+after()
# RIGHT
PUT >3:
+after()

# WRONG — body rows under register PUT; register pastes take no body. RIGHT: bodyless `PUT >20 @fn`.
PUT >20 @fn:
+function f() {}
</anti-patterns>

<critical>
1. RE-GROUND AFTER EVERY EDIT — applied edits renumber the file and change the `#TAG`; take next numbers from the edit response or a fresh `read`. Stale tag or surprise? STOP, re-`read`.
2. RANGES ARE TIGHT — cover only lines that change. Whole construct → `PUT N*:`.
3. BODY = FINAL CONTENT — every body row starts with `+`; Markdown bullets use `+- item`, not `- item`.
</critical>
