# Agent-authored skill bundles

Why an agent could not author a skill containing a script, and what was changed.

## The gap

Measured on the 31-task SkillsBench/SkillLearnBench subset (nearai/benchmarks#287):
**0 of 27** agent-authored skills shipped a single file besides `SKILL.md`, against
**18 of 31** human-curated ones (79 `.py` scripts, 78 `.xsd` schemas, 84 `.md`
references). An agent could only ever author the prose half of a skill, so a later run
re-derived the method and could re-make the same mistake — `lake_warming`'s self-authored
skill described its regression procedure in prose, the next run recomputed it slightly
differently and missed the grader's `p < 0.05` threshold.

## It was never a missing capability

`install_skill` has always taken `files: &[SkillInstallFile]`, and `parse_install_files`
has always read an `input["files"]` array. **Three stacked gates** made that unreachable,
and each one looked like the whole story until it was removed:

1. `schemas/builtin/skill_install.input.v1.json` advertised only `name`/`content`/`url`
   **and** set `additionalProperties: false` — so a model sending `files` was not merely
   uninformed, it was rejected. Across 112 observed calls, 111 used exactly
   `['content','name']`, which is all the schema permitted.
2. The only encodings were `bytes_base64` and a JSON array of byte integers. A bundle file
   an agent writes is a script, reference doc or schema fragment — all UTF-8. Base64 costs
   ~33% more tokens and turns one encoding slip into an `InputEncode` failure of the whole
   install.
3. `skill_install_input` gated its direct-install arm on `!contains_key("files")`, so
   `content` + `files` matched **no** arm and fell through to `_ => Err(InputEncode)`.
   `files` was reachable only on the URL-fetch arm, which builds the array itself.

With all three removed the model immediately authored `scripts/verify_bib.py`,
`references/dots_coefficients.json`, `scripts/analyze.py` and
`references/categories.md` — it had been sending 18 correctly-shaped `{path, text}`
entries all along and every one was refused. So neither capability nor elicitation was
the bottleneck.

## Changes

- `parse_install_files` accepts `text` (UTF-8) alongside `bytes_base64`/`bytes`; `text`
  wins when both are present. Binary payloads unaffected.
- the schema advertises `files` (`path` + `text`/`bytes_base64`) and tells the model why
  to use it: put a reusable computation in a script rather than prose, and have `SKILL.md`
  name the files it relies on.
- the direct-install arm no longer rejects an author-supplied `files` array.
  `source`/`source_url` stay excluded there — those record provenance and are set by the
  URL path, so an agent must not be able to forge them.
- prose-only installs are untouched: no `files` key still parses to an empty vec.

## Reaching the files at run time

Whether prose injection or a readable path is the better delivery mechanism was measured
separately, and the readable path wins overall — see nearai/benchmarks#327 for the numbers
(ceiling 90.4% vs claude-code 91.5%, paired −0.8pp; self-creation 90.1% vs 93.5%, paired
−2.7pp, 14/20 tasks tied).

A narrower variant — advertise a real path **only** when the skill ships resources — was
built and **measured worse**: −25.7pp against that baseline on self-creation, −40.6pp
against claude-code. It stratifies well on curated skills (+13.2pp where a skill ships
files, −6.9pp where it does not), which is why it looked right, but an agent-authored
skill is usually `SKILL.md`-only: the gate suppresses its path while the selector has
already dropped it for scoring 0, and it reaches the model by neither route. Recorded here
because the stratified argument for it is genuinely persuasive and someone will propose it
again.

## Two things an implementer should not miss

**`SkillBundleDescriptor` cannot advertise bundle resources.** It exposes exactly one
path — `skill_md_path`, hardcoded to `SkillFilePath::skill_md()`. So after an agent
installs `scripts/extract.py`, nothing tells a later run it exists unless `SKILL.md` names
it. That is why the authoring guidance requires `SKILL.md` to list its own files. If a
future discovery design can carry a file list, that removes the workaround.

**Trust.** `FilesystemSkillBundleRoot::user` marks bundles `SkillTrust::Trusted`, so an
agent that can install an executable script is writing into a trusted root that later
runs. Small in code, significant in consequence; agent-authored bundles likely warrant a
distinct trust level. This is the real open design question, not the plumbing.

## Why activation still matters alongside this

The selector scores a candidate only from `activation.keywords`/`tags`/`patterns` and keeps it
`if score > 0`; name and description contribute nothing. Measured on this subset, **0 of 30
agent-authored skills contained an `activation` block**, so none of them ever auto-activates.

That is *not* the same as being invisible. Listing membership is decided by **visibility, not
selection**, so under `SkillInjectionMode::Listing` (reborn's default) an agent-authored skill
**is** shown to the model, together with a header telling it to call `builtin.skill_activate`.
It simply does not: measured at **3 of 30 runs** calling the tool, and **0 of 30** reading a
body.

So the reachability problem is elicitation, not filtering. A floor-score strategy
(`always_available`) was implemented to remove the `score > 0` gate and then removed again — it
bought no reach for exactly this reason, and it demoted chain-loaded companions. `Full`
injection mode is the one place the gate genuinely hides a skill, since only activated bundles
render there.
