# Live Tool Failure Notes

This note captures recurring tool-misuse patterns seen while iterating on the
live 20+ turn persona workflows in `tests/e2e_live_personas.rs`.

The point is not just to make tests pass. These failures show where tool
descriptions and routing affordances are weak enough that the model regularly
chooses the wrong action or claims success before persistence.

## Scope

Observed from live persona traces and logs, especially:

- `tests/fixtures/llm_traces/live/ceo_full_workflow.log`
- `tests/fixtures/llm_traces/live/content_creator_full_workflow.log`
- `tests/fixtures/llm_traces/live/trader_full_workflow.log`

## Failure Patterns

### 1. Claims success without a write

Most common failure. The assistant says it "tracked", "created", "parked", or
"recorded" something, but there is no matching `memory_write` for the implied
workspace file.

Seen in:

- CEO late-turn commitments before prompt tightening
- Creator trend commitments
- Creator parked ideas
- Creator sponsored/due-soon content tracking

Why it happens:

- The model treats a plausible natural-language confirmation as sufficient.
- Tool descriptions do not always make "write before confirm" feel mandatory.
- Persona bundle instructions were previously too soft about persistence.

What helped:

- Strengthening skill instructions to explicitly forbid confirmation before a
  successful `memory_write`.
- Rephrasing prompts from abstract "note this" language to explicit
  "track this commitment" / "park this idea" wording.

Recommended tool-description improvement:

- `memory_write` should explicitly say:
  "If the user asked to track, save, record, or park something, you have not
  completed the task until this tool succeeds."

### 2. Wrong patch-mode usage for `memory_write`

The model often tries patch mode with `old_string` but omits `new_string`, uses
an empty `old_string`, or provides an `old_string` that does not match the file.

Seen errors:

- `new_string is required when old_string is provided`
- `old_string cannot be empty`
- `Patch failed ... old_string not found in document`

Seen in:

- Creator pipeline updates
- Trader historical trace

Why it happens:

- The model guesses patch mode when simple overwrite/append would be safer.
- Tool description does not push hard enough toward full-content writes when the
  caller does not know the exact existing text.

Recommended tool-description improvement:

- `memory_write` should include a short rule:
  "Use patch mode only when you have just read the target file and can quote
  the exact `old_string`. Otherwise use full `content` writes."

### 3. Probing with `memory_read` for files that do not exist yet

This is common and usually recoverable:

- `memory_read(commitments/README.md)` before setup
- similar first-read probes in trader and creator setup

Why it happens:

- The model uses `memory_read` as an existence check.
- The tool returns a hard error instead of a softer not-found sentinel.

Current handling:

- The harness now treats these as benign recoverable errors.

Potential improvement:

- Consider documenting in `memory_read`:
  "A missing document is normal during setup. If you are checking whether a file
  exists before writing it, expect a not-found error and then call
  `memory_write`."

### 4. Choosing creative-generation tools when the user asked only to track

The creator workflow repeatedly triggered content generation behavior when the
user wanted deadline tracking only.

Seen in:

- `image_generate(...)` calls on thumbnail deadline turns
- generated TikTok/Twitter copy instead of just tracking distribution deadlines

Why it happens:

- The content-creator persona naturally invites creative execution.
- Tool descriptions for creative tools do not strongly distinguish "make the
  asset" from "track the obligation to make the asset".

What helped:

- Explicit prompt phrasing: "Track this commitment only" / "Do not create assets."
- Stronger persona-bundle rule to avoid asset generation unless explicitly asked.

Recommended tool-description improvement:

- `image_generate` and related creative tools could state:
  "Do not use this tool when the user is only asking to track a deadline,
  obligation, or workflow stage."

### 5. Accidental `__codeact__` / script execution on workflow-tracking tasks

The model sometimes reaches for CodeAct-style script execution when plain memory
tools are enough.

Seen failures:

- SyntaxError traces
- unsupported Python constructs in Monty
- OS-restricted operations in CodeAct scripts

Why it happens:

- The model appears to treat structured workspace updates as mini-programming
  tasks rather than simple persistence.

Recommended tool-description improvement:

- Add stronger language to the tool-discovery surface around workspace tools:
  "For workspace tracking, prefer `memory_read` / `memory_write` / `memory_tree`.
  Do not use CodeAct or shell scripting unless the user asked for computation or
  transformation that memory tools cannot do."

### 6. Writing to the wrong namespace/path family

The model sometimes wrote to paths that were semantically close but outside the
expected commitments workspace structure:

- `content/...` instead of `commitments/content-pipeline/...`
- `commitments/signals/...` vs `commitments/open/...`
- `commitments/distribution/...` vs `commitments/open/...`

Why it happens:

- Path conventions exist in skills, but tool descriptions do not reinforce the
  workspace contract.
- The model generalizes from filenames it invents in prose.

Recommended tool-description improvement:

- `memory_write` could mention:
  "When a skill or workflow specifies a target directory, write exactly there.
  Do not invent sibling top-level namespaces."

### 7. Rate limits and optional backend gaps produce noisy but recoverable errors

Seen in live runs:

- transient tool rate limits
- image generation model unavailable (`flux-1.1-pro` not found)

These are real environment issues, but in the observed runs they did not always
invalidate the business outcome of the test.

Current handling:

- The harness now filters some of these as benign when the run clearly recovers.

Recommended product improvement:

- Tool result text for temporary provider/tool outages should be explicit about
  retryability and preferred fallback:
  - "temporary rate limit; retry later"
  - "optional creative backend unavailable; continue with tracking-only flow"

## What Should Change Where

After cross-checking against `#2025` (the coding/file-tools PR), the right
split is:

- **Tool descriptions** should clarify tool mechanics and boundaries.
- **Skills/persona bundles** should define workflow obligations like
  "persist before confirm".
- **Runtime checks** should catch high-confidence mismatches between what the
  assistant claimed and what actually happened.

`#2025` improved file/coding tools by making their operational contract clearer
(`write_file` vs workspace memory, `apply_patch` precision, file history). It
did **not** make tool descriptions carry higher-level workflow policy. That same
principle should apply here.

So the earlier recommendation to make `memory_write` itself say things like
"you are not done until this succeeds" is too strong. That rule belongs in the
skills and possibly in runtime verification, not in the generic memory tool
description.

## Better Tool-Description Ideas

### `memory_write`

Keep this focused on mechanics and safe usage:

- "Prefer full `content` writes unless you have just read the file and know the
  exact text to replace."
- "Do not use patch mode with an empty `old_string`."
- "Do not invent alternate workspace roots when the skill specifies a path."

This is analogous to `#2025` tightening file-tool operational guidance without
embedding task policy in the tool description.

### `memory_read`

Add guidance like:

- "Missing documents are common during setup. A not-found error often means you
  should create the file with `memory_write`."

### `image_generate`

Add guidance like:

- "Use only when the user wants an image asset created or edited. Not for
  deadline tracking, workflow updates, or commitment capture."

### Tool discovery / meta guidance

Wherever tool summaries are surfaced:

- "For commitment/workflow tracking, default to memory tools."
- "Do not use CodeAct or shell for simple workspace updates."

This is still tool-selection guidance, not workflow policy.

## What Should NOT Be a Memory-Tool Change

These should not be solved by stuffing more policy into `memory_write`:

- "Do not claim 'tracked' unless the write succeeded"
- "Parking an idea is not complete until persisted"
- "Decision capture is only successful if both decision and intel docs were written"

Those are skill-level contracts, and the repo is already moving in that
direction with stronger `SKILL.md` language.

## Better Product-Level Fixes

### 1. Keep persistence policy in skills

The strongest improvements came from tightening:

- `decision-capture`
- `commitment-triage`
- `idea-parking`
- `content-creator-assistant`

That is the right layer for "write before confirm" behavior.

### 2. Add runtime claim-vs-effect guardrails

The recurring bad pattern was:

- assistant says "tracked/recorded/parked"
- no matching workspace write happened

This is a runtime consistency problem more than a tool-description problem.
A good fix would be a lightweight post-tool-loop check for high-confidence
phrases like:

- "tracked"
- "recorded"
- "parked"
- "created commitment"

If those appear without any matching `memory_write`, the agent should get one
more repair pass instead of returning that response to the user.

### 3. Keep tool descriptions mechanical, not workflow-heavy

This keeps memory tools aligned with the spirit of `#2025`:

- file tools explain file mechanics
- memory tools explain workspace mechanics
- skills explain task workflows
- runtime enforces claim/effect consistency where needed

## Recommended Follow-Ups

1. Tighten `memory_write` patch-mode guidance, not its workflow policy.
2. Add a short "tracking-only vs creation" warning to creative tools.
3. Add a runtime guard for high-confidence tracking phrases:
   if the assistant says "tracked/recorded/parked" and no matching write
   happened, treat that as a recoverable tool-loop miss and force another pass.
4. Keep persistence rules in the skills/persona bundles.
5. Keep logging skill activations and turn-local tool events in live session
   logs; they made these failures diagnosable.
