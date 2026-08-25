#!/usr/bin/env python3
"""Contracts for the single Rust toolchain setup path.

One question, one module: *what toolchain, linker, and build flags does a
Rust job get?* The answer is `.github/actions/setup-rust` and nothing else,
and these six validators are what make that true rather than aspirational.

Split out of `ws12_workflow_contracts.py`, which had grown past 2,100 lines
covering unrelated lanes (stress suites, crate scope filters, WebUI sites).
Wiring stays in that module's `validate_workflow_texts`; only the
toolchain-shaped rules live here.

Note which direction each check runs, because the mix is what makes the
suite trustworthy. Three assert an ABSENCE — no dtolnay action, no raw
bootstrap, no shadowing RUSTFLAGS key. Absence-only guards cannot notice that
the thing they protect has been deleted: a job that installs nothing at all
satisfies every one of them. That is exactly how a release workflow with no
Rust install passed this suite.

The other four assert a PRESENCE or an equality and are therefore
deletion-safe for their own subject: the composite has the steps it must have,
its default equals rust-toolchain.toml's channel, the release lane and its
cargo-dist fragment both reach the composite, and every job that runs cargo
reaches it. Before adding an eighth, work out which direction it runs and what
deletion it would miss.
"""

from __future__ import annotations

import re
from pathlib import Path

from workflow_text import JOB_HEADING, job_blocks, job_body, step_body

ROOT = Path(__file__).resolve().parents[3]

SETUP_RUST_ACTION = ".github/actions/setup-rust/action.yml"
RUSTUP_TOOLCHAIN_PIN_STEP = "Pin the resolved toolchain for the rest of this job"
MOLD_INSTALL_STEP = "Install mold and clang"
MOLD_VERIFY_STEP = "Verify mold linker is active"
MOLD_EXPORT_STEP = "Export mold RUSTFLAGS"
# The one canonical mold invocation; a job's own env may append extra flags
# after it (the nightly lanes add -Zcrate-attr=...), but this prefix is the
# only place it may be written out — everywhere else must go through here.
MOLD_RUSTFLAGS = "-C linker=clang -C link-arg=--ld-path=/usr/bin/mold"


def validate_setup_rust_action(text: str | None) -> list[str]:
    """The setup-rust composite must actually pin RUSTUP_TOOLCHAIN and mold.

    Contract: every job that installs Rust through this composite gets a
    RUSTUP_TOOLCHAIN export naming exactly the toolchain dtolnay/rust-toolchain
    just installed (steps.install.outputs.name — not a second, possibly wrong,
    guess at the resolved version), and mold's install/verify/RUSTFLAGS steps
    are gated on Linux so `mold: true` is safe to pass on any runner OS.
    """
    if text is None:
        return [f"{SETUP_RUST_ACTION}: could not read the composite action file"]
    errors: list[str] = []
    pin_step = step_body(text, RUSTUP_TOOLCHAIN_PIN_STEP)
    if pin_step is None:
        errors.append(
            f"{SETUP_RUST_ACTION}: missing the {RUSTUP_TOOLCHAIN_PIN_STEP!r} step"
        )
    elif "RUSTUP_TOOLCHAIN=${{ steps.install.outputs.name }}" not in pin_step:
        errors.append(
            f"{SETUP_RUST_ACTION}: {RUSTUP_TOOLCHAIN_PIN_STEP!r} must export "
            "RUSTUP_TOOLCHAIN from steps.install.outputs.name, or a job's "
            "cargo invocations can drift from what this step actually installed"
        )
    for step_name in (MOLD_INSTALL_STEP, MOLD_VERIFY_STEP, MOLD_EXPORT_STEP):
        body = step_body(text, step_name)
        if body is None:
            errors.append(f"{SETUP_RUST_ACTION}: missing the {step_name!r} step")
            continue
        if "runner.os == 'Linux'" not in body:
            errors.append(
                f"{SETUP_RUST_ACTION}: {step_name!r} must gate on "
                "runner.os == 'Linux' so mold: true is safe on any runner"
            )
    export_step = step_body(text, MOLD_EXPORT_STEP)
    if export_step is not None and MOLD_RUSTFLAGS not in export_step:
        errors.append(
            f"{SETUP_RUST_ACTION}: {MOLD_EXPORT_STEP!r} must export the "
            f"canonical mold RUSTFLAGS prefix '{MOLD_RUSTFLAGS}'"
        )
    return errors


DTOLNAY_ACTION = "dtolnay/rust-toolchain@"


def validate_no_direct_dtolnay_usage(workflows: dict[str, str]) -> list[str]:
    """Every workflow must install Rust through .github/actions/setup-rust.

    A negative substring check, not a per-site window scan: this is what
    T1's earlier per-input-drift design collapsed to once every job routes
    through one composite. Also forbids re-writing out the canonical mold
    RUSTFLAGS prefix by hand anywhere a workflow's own env block might set
    it — the composite is the only place that string may appear.
    """
    errors: list[str] = []
    for path, text in workflows.items():
        if DTOLNAY_ACTION in text:
            errors.append(
                f"{path}: calls {DTOLNAY_ACTION!r} directly — install Rust "
                "through .github/actions/setup-rust instead"
            )
        if MOLD_RUSTFLAGS in text:
            errors.append(
                f"{path}: writes out the canonical mold RUSTFLAGS prefix "
                f"'{MOLD_RUSTFLAGS}' directly — pass mold: true to "
                ".github/actions/setup-rust instead, which prepends it onto "
                "this job's existing RUSTFLAGS"
            )
    return errors


SETUP_RUST_USES = "uses: ./.github/actions/setup-rust"
JOBS_KEY = re.compile(r"^jobs:[ \t]*$", re.MULTILINE)
# One list item that is nothing but an anchor definition (`- &install-rust`).
# Matched per line so the node it opens can be bounded by indentation.
ANCHOR_DEF_LINE = re.compile(r"^\s*-\s*&(?P<name>[\w-]+)\s*$")
# Job-level `env:` (jobs.<job>.env, two-space job + four-space `env:` + this
# key at six spaces) and workflow-level top `env:` (two-space key straight
# under the file's own `env:`) are both re-applied to every step of a job on
# top of $GITHUB_ENV, so both shadow the composite's mold export the same
# way. They need separate patterns, not one merged indentation class: the
# workflow-level form sits in the file's preamble, before any job heading, so
# it is checked once per file rather than by slicing per-job blocks.
# Any RUSTFLAGS key inside a job, at job-env depth (6 spaces) or
# step-env depth (10) — a step-level env shadows the composite's
# $GITHUB_ENV write for that step exactly like a job-level one, so
# pinning the exact job depth left a third of the shapes unguarded.
JOB_ENV_RUSTFLAGS = re.compile(r"^ {6,}RUSTFLAGS:", re.MULTILINE)
WORKFLOW_ENV_RUSTFLAGS = re.compile(r"^ {2}RUSTFLAGS:", re.MULTILINE)


def validate_no_job_env_rustflags_with_setup_rust(
    workflows: dict[str, str],
) -> list[str]:
    """A job installing Rust via the composite must not set its own RUSTFLAGS.

    GitHub re-applies a job-level `env:` mapping to every step of that job,
    on top of whatever earlier steps wrote to $GITHUB_ENV. So a job-level
    `RUSTFLAGS:` shadows the composite's export for the rest of the job and
    silently drops the mold linker flags — a slower build, never a red
    check, which is exactly the drift class this action exists to remove. A
    workflow-level top `env:` key shadows every job in the file identically,
    so it is checked the same way. Jobs pass their extra flags through the
    composite's `extra_rustflags` input instead, so one place composes the
    final value.
    """

    errors: list[str] = []
    for path, text in workflows.items():
        if SETUP_RUST_USES not in text:
            continue
        anchors = _composite_anchors(text)
        # Whole file, not just the preamble. A top-level mapping key need not
        # precede `jobs:` — a root `env:` block placed after it is valid YAML,
        # applies to every job the same way, and slicing at `jobs:` made it
        # invisible to this check while its two-space indent also dodged the
        # six-space per-job pattern. It fell through both.
        if WORKFLOW_ENV_RUSTFLAGS.search(text):
            errors.append(
                f"{path}: workflow-level env sets a RUSTFLAGS key while a "
                "job in this file installs Rust through "
                ".github/actions/setup-rust — a workflow-level env key "
                "shadows the composite's $GITHUB_ENV write for every job in "
                "the file identically to a job-level one; pass extra flags "
                "as the composite's extra_rustflags input instead"
            )
        for name, block in _job_blocks(text):
            if not _reaches_composite(block, anchors):
                continue
            if JOB_ENV_RUSTFLAGS.search(block):
                errors.append(
                    f"{path}: job {name!r} sets a job- or "
                    "step-level RUSTFLAGS env key while installing Rust through "
                    ".github/actions/setup-rust — job env shadows the "
                    "composite's $GITHUB_ENV write and drops the mold linker "
                    "flags; pass them as the composite's extra_rustflags "
                    "input instead"
                )
    return errors


RUST_BOOTSTRAP_PATTERNS = (
    # Raw rustup bootstraps.
    "sh.rustup.rs",
    "rustup-init",
    "rustup toolchain install",
    # Third-party toolchain actions. The composite is the only sanctioned
    # installer, so a workflow reaching for a different vendor action is the
    # same drift as a curl bootstrap: unpinned toolchain, no mold, no
    # rust-toolchain.toml sync. (`dtolnay/rust-toolchain` has its own check
    # with a more specific message; it is deliberately not repeated here.)
    "actions-rs/toolchain",
    "actions-rust-lang/setup-rust-toolchain",
    "hecrj/setup-rust-action",
    "raftario/setup-rust-action",
)
# Residual risk, named rather than papered over: a job whose `container:`
# image ships Rust preinstalled installs nothing, so no text pattern can see
# it. Such a job would silently build on the image's toolchain instead of the
# pin. Nothing in .github/workflows does this today; if one appears it needs a
# structural check (assert every Rust-building job calls the composite), not a
# wider substring list.
# No workflow may bootstrap Rust outside the composite. cargo-dist
# re-includes .github/dist-build-setup.yml on every regeneration, so the
# release build jobs install Rust through the composite from there — there
# is no lane this contract cannot cover, and so no exemption mechanism here.
# If a genuinely unavoidable bootstrap ever appears, add the escape hatch
# then, with that lane as its first entry and its reason in the comment.


def validate_no_unmanaged_rust_bootstrap(workflows: dict[str, str]) -> list[str]:
    """Every hand-written workflow installs Rust through the composite.

    `validate_no_direct_dtolnay_usage` only sees the vendor action. A raw
    `curl https://sh.rustup.rs | sh` installs Rust just as effectively and
    matches no such string, so it would otherwise pass this gate forever --
    unpinned, without mold, and unchecked against rust-toolchain.toml.
    """

    errors: list[str] = []
    for path, text in workflows.items():
        hits = sum(text.count(pattern) for pattern in RUST_BOOTSTRAP_PATTERNS)
        if not hits:
            continue
        errors.append(
            f"{path}: {hits} raw Rust bootstrap(s) "
            f"({', '.join(RUST_BOOTSTRAP_PATTERNS)}). Install Rust through "
            ".github/actions/setup-rust so the toolchain stays pinned, mold "
            "stays wired, and rust-toolchain.toml stays enforced."
        )
    return errors


def _composite_anchors(text: str) -> set[str]:
    """Anchor names whose OWN YAML node reaches the composite.

    A job can pick the composite up through an alias (`- *install-rust`)
    rather than a literal `uses:` line, as release-plz.yml does, so both
    contracts below have to resolve aliases before deciding a job misses it.

    Bounded by indentation, and comment-stripped, because the previous span
    ("from this anchor to the next anchor or job heading") was wide enough to
    swallow unrelated siblings: a *comment* elsewhere in that span mentioning
    the composite marked the anchor as installing Rust, and a job that only
    aliased it then passed `validate_rust_jobs_reach_the_composite` while
    installing nothing. That is the same alias-fooled bypass this module's
    docstring warns about, so it gets the same treatment as everything else
    here — the check reads executable steps, never text that merely looks
    like one.
    """
    anchors: set[str] = set()
    lines = text.splitlines()
    for index, line in enumerate(lines):
        match = ANCHOR_DEF_LINE.match(line)
        if not match:
            continue
        indent = len(line) - len(line.rstrip("\n").lstrip())
        for follower in lines[index + 1 :]:
            if not follower.strip():
                continue
            # First non-blank line at or left of the anchor's own marker ends
            # its node — that is a sibling or a parent, not its content.
            if len(follower) - len(follower.lstrip()) <= indent:
                break
            if SETUP_RUST_USES in follower.split("#")[0]:
                anchors.add(match.group("name"))
                break
    return anchors


def _job_blocks(text: str) -> list[tuple[str, str]]:
    """Every job in the file, bounded to the `jobs:` mapping.

    Slicing from the first heading blindly truncated the preamble at the
    first `on:` trigger and made the workflow-level checks dead code on every
    real workflow, so the `jobs:` offset is load-bearing here.
    """
    jobs_key = JOBS_KEY.search(text)
    return job_blocks(text, jobs_key.start() if jobs_key else 0)


def _reaches_composite(block: str, anchors: set[str]) -> bool:
    return SETUP_RUST_USES in block or any(f"*{a}" in block for a in anchors)


# A cargo/rustc/rustup command actually being invoked -- not `Cargo.toml` in a
# `paths:` filter, not `cargo` inside a URL or a comment. Bounded on both sides
# by non-path, non-word characters so `target/cargo-timings` and
# `scripts/cargo-foo.sh` do not count as running the compiler.
CARGO_INVOCATION = re.compile(r"(?<![\w./-])(?:cargo|rustc|rustup)(?![\w./-])")
# The hermetic runners need a toolchain without naming one:
# run-hermetic-test-process.sh probes `rustc --print sysroot` to build the
# child PATH and exits 1 if it cannot resolve one. A lane that invokes them
# therefore needs Rust exactly as much as a literal `cargo` line does, but
# CARGO_INVOCATION cannot see it -- the workflow only mentions the script.
# webui-v2-test-lanes was the one lane in this state: it compiles nothing, so
# nobody noticed it needed rustc, and rustup installed the pinned toolchain
# lazily mid-test, once per shard, racing its own component downloads.
NEEDS_TOOLCHAIN_SCRIPT = re.compile(r"run-hermetic-(?:deterministic-suite|test-process)\.sh")


def validate_rust_jobs_reach_the_composite(workflows: dict[str, str]) -> list[str]:
    """Any job that runs cargo must install Rust through the composite.

    The release-lane check below is the same rule written for one file. It
    was added after a workflow lost its Rust install entirely and every
    absence-only guard called that clean -- but it only ever protected
    `ironclaw-release.yml`. Deleting the composite step from any other
    workflow reproduced the identical failure with the suite still green,
    because "no dtolnay, no bootstrap, no shadowing RUSTFLAGS" is all
    trivially true of a job that installs nothing at all.

    Scoped to jobs that actually invoke the compiler, so a docs or frontend
    job needs no exemption. All 31 cargo-running jobs in the tree satisfy
    this today, which is why it ships with no allowlist: an entry here would
    mean a lane building Rust on an unpinned toolchain.
    """

    errors: list[str] = []
    for path, text in workflows.items():
        anchors = _composite_anchors(text)
        for name, block in _job_blocks(text):
            code = "\n".join(line.split("#")[0] for line in block.splitlines())
            if not (
                CARGO_INVOCATION.search(code) or NEEDS_TOOLCHAIN_SCRIPT.search(code)
            ):
                continue
            if _reaches_composite(block, anchors):
                continue
            errors.append(
                f"{path}: job {name!r} needs a Rust toolchain but never reaches "
                f"`{SETUP_RUST_USES}`. Either it builds on whatever toolchain "
                "the runner image ships (unpinned, no mold), or rustup installs "
                "the pin lazily on first use inside the repo -- mid-lane, once "
                "per shard, racing its own component downloads. Install Rust "
                "through the composite so it happens once, up front."
            )
    return errors


RELEASE_WORKFLOW = ".github/workflows/ironclaw-release.yml"
DIST_BUILD_SETUP = ".github/dist-build-setup.yml"
# cargo-dist's container build job — the only job in the release workflow
# that installs Rust, and the one the contract below actually guards.
RELEASE_BUILD_JOB = "build-local-artifacts"


# The condition cargo-dist's container build job carries on its Rust install,
# in both the fragment and the generated workflow. Pinned, not merely tolerated:
# the contract used to assert the step's TEXT existed and said nothing about
# when it runs, so `if:` could be narrowed, widened, or dropped silently. The
# value itself is deliberate -- container images ship without cargo, hosted
# runners resolve rust-toolchain.toml themselves -- and it matches the
# pre-composite step exactly, so release behaviour is unchanged by this PR.
RELEASE_STEP_CONDITION = "if: ${{ matrix.container }}"


def _composite_step_condition(text: str) -> str | None:
    """The `if:` guarding the composite step, or None if the step is unguarded.

    Scans the step's WHOLE body, not just the lines above `uses:`. YAML mapping
    keys are unordered, so `uses:` may legally precede `if:`; stopping at the
    `uses:` line reported such a step as unconditional and would have rejected
    a correct release workflow. The step is bounded by its own `- ` marker and
    the next line at or left of that marker.
    """
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if SETUP_RUST_USES not in line.split("#")[0]:
            continue
        start = index
        while start > 0 and not lines[start].lstrip().startswith("- "):
            start -= 1
        marker_indent = len(lines[start]) - len(lines[start].lstrip())
        end = len(lines)
        for offset in range(start + 1, len(lines)):
            follower = lines[offset]
            if not follower.strip():
                continue
            if len(follower) - len(follower.lstrip()) <= marker_indent:
                end = offset
                break
        for candidate in lines[start:end]:
            stripped = candidate.split("#")[0].strip().lstrip("- ").strip()
            if stripped.startswith("if:"):
                return stripped
        return None
    return None


def _has_composite_step(text: str) -> bool:
    """True when some EXECUTABLE line invokes the composite.

    Comment-stripped per line: a substring search over raw text counted a
    commented-out or merely-mentioned `uses:` as an install.
    """
    return any(SETUP_RUST_USES in line.split("#")[0] for line in text.splitlines())


def validate_release_workflow_installs_rust(
    workflows: dict[str, str], root: Path = ROOT
) -> list[str]:
    """The release lane must actually REACH the composite, not merely lack a bootstrap.

    Most checks here assert an absence — no dtolnay, no raw bootstrap, no
    shadowing RUSTFLAGS. Absence-only checks called a release workflow with NO
    Rust install whatsoever "clean": removing the old `curl | sh` step without
    adding anything passed the whole suite, and a container build would have
    died on `cargo: command not found`.

    Scoped to `build-local-artifacts` rather than the file, because the file is
    the wrong unit: nothing in ironclaw-release.yml matches a literal `cargo`
    (cargo-dist shells out to `dist build`), so
    `validate_rust_jobs_reach_the_composite` never covers this workflow and
    this is its only guard. A file-wide substring let that job lose its own
    install while an unrelated job — or a comment — kept the contract green.

    ironclaw-release.yml is generated by cargo-dist from the fragment at
    .github/dist-build-setup.yml, so the step has to exist in BOTH: the
    fragment so `dist generate` keeps emitting it, and the checked-in workflow
    because that is the file GitHub actually runs.
    """

    errors: list[str] = []
    release = workflows.get(RELEASE_WORKFLOW)
    if release is not None:
        jobs = dict(_job_blocks(release))
        block = jobs.get(RELEASE_BUILD_JOB)
        if block is None:
            errors.append(
                f"{RELEASE_WORKFLOW}: no {RELEASE_BUILD_JOB!r} job. cargo-dist "
                "names the container build job; if it was renamed, update "
                "RELEASE_BUILD_JOB so this contract keeps guarding it."
            )
        elif _has_composite_step(block) and _composite_step_condition(
            block
        ) != RELEASE_STEP_CONDITION:
            found = _composite_step_condition(block) or "<unconditional>"
            errors.append(
                f"{RELEASE_WORKFLOW}: job {RELEASE_BUILD_JOB!r} guards its "
                f"Rust install with {found!r}, not {RELEASE_STEP_CONDITION!r}. "
                "That condition decides which matrix entries reach the "
                "composite at all; changing it changes which release binaries "
                "are built with the pinned toolchain and mold, so it is a "
                "deliberate edit in both this file and "
                f"{DIST_BUILD_SETUP}, not a silent one."
            )
        elif not _has_composite_step(block):
            errors.append(
                f"{RELEASE_WORKFLOW}: job {RELEASE_BUILD_JOB!r} has no "
                f"`{SETUP_RUST_USES}` step. The container build jobs have no "
                "other Rust install path, so this lane would fail with "
                "'cargo: command not found'. Re-run `dist generate` or "
                f"re-inline the step from {DIST_BUILD_SETUP}."
            )
    try:
        fragment = (root / DIST_BUILD_SETUP).read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"{DIST_BUILD_SETUP}: could not read fragment: {error}")
    else:
        if _has_composite_step(fragment) and _composite_step_condition(
            fragment
        ) != RELEASE_STEP_CONDITION:
            found = _composite_step_condition(fragment) or "<unconditional>"
            errors.append(
                f"{DIST_BUILD_SETUP}: guards its Rust install with {found!r}, "
                f"not {RELEASE_STEP_CONDITION!r} — the fragment and the "
                "generated workflow would disagree the next time cargo-dist "
                "regenerates."
            )
        if not _has_composite_step(fragment):
            errors.append(
                f"{DIST_BUILD_SETUP}: no `{SETUP_RUST_USES}` step. cargo-dist "
                "re-inlines this fragment on regeneration, so dropping it here "
                "silently removes the release lane's Rust install the next time "
                "the workflow is regenerated."
            )
    return errors


# Cargo.toml's `[profile.dev] debug = 0` owns the debug-info policy. Anything
# else ASSIGNING one of these is a second writer of the same value -- harmless
# while the values agree, and a silent divergence the day someone bumps one.
# An assignment only: `run-hermetic-test-process.sh` names the same variables
# in a passthrough allowlist (a `case` pattern, no `=`), which is how a
# developer's `CARGO_PROFILE_DEV_DEBUG=2` override survives the hermetic
# barrier. That entry must keep working, so it must not match here.
# Both syntaxes, because the value was written in both: shell `KEY=value` in
# scripts, YAML `KEY: value` in workflow job envs. Matching only `=` made this
# guard blind to the 14 workflow lines this PR deleted -- the majority of what
# it exists to keep deleted.
# The key may be bare, double-quoted, or single-quoted -- all three are valid
# YAML for the same mapping key, and a quoted one bypassed this guard.
DEBUG_POLICY_ASSIGNMENT = re.compile(
    r"[\"']?CARGO_PROFILE_[A-Z]+_DEBUG[\"']?\s*[:=]"
)
# Where the value could be written. `scripts/` covers the shell/python side;
# `.github/` covers workflow and composite-action env blocks, which is where
# five job envs carried it before this change.
DEBUG_POLICY_SEARCH_ROOTS = ("scripts", ".github")
DEBUG_POLICY_SUFFIXES = (".sh", ".py", ".yml", ".yaml")
DEBUG_POLICY_OWNER = "Cargo.toml"


def validate_single_debug_policy_owner(root: Path = ROOT) -> list[str]:
    """Only Cargo.toml may set the debug-info profile values.

    The migration deleted these env pairs from five workflow job envs on the
    strength of the profile block owning them, but left the identical `:-0`
    defaults standing in two scripts -- so the change's own claim of a single
    owner was not true of the whole tree. Two audit lanes reported it
    independently.

    The first version of this guard then repeated the mistake in miniature: it
    scanned only `scripts/**` for `KEY=value`, so it could not see the 14
    workflow lines -- the majority of what it was written to keep deleted. Two
    review lanes reported THAT independently. Scope and syntax now cover both
    places the value was actually written.
    """

    errors: list[str] = []
    candidates: list[Path] = []
    for search_root in DEBUG_POLICY_SEARCH_ROOTS:
        candidates.extend(sorted((root / search_root).rglob("*")))
    for path in candidates:
        if not path.is_file() or path.suffix not in DEBUG_POLICY_SUFFIXES:
            continue
        # Test files carry the forbidden string on purpose, as fixtures and
        # as the sabotage input that proves this check fires. Scanning them
        # would make the contract unable to have a regression test at all.
        if path.name.startswith(("test_", "test-")):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            errors.append(f"{path}: could not read: {error}")
            continue
        for number, line in enumerate(text.splitlines(), start=1):
            if DEBUG_POLICY_ASSIGNMENT.search(line.split("#")[0]):
                errors.append(
                    f"{path.relative_to(root)}:{number}: assigns a "
                    "CARGO_PROFILE_*_DEBUG value, which "
                    f"{DEBUG_POLICY_OWNER}'s `[profile.dev]` block owns. Two "
                    "writers of one value diverge the day either is bumped; "
                    "delete this and let the profile decide."
                )
    return errors


def validate_toolchain_pin_sync(root: Path = ROOT) -> list[str]:
    """rust-toolchain.toml and the composite's default must name one version."""
    try:
        file_text = (root / "rust-toolchain.toml").read_text(encoding="utf-8")
    except OSError:
        return ["rust-toolchain.toml: missing (single source of truth for the CI toolchain)"]
    channel_match = re.search(r'^channel = "(\d+\.\d+\.\d+)"$', file_text, re.MULTILINE)
    if channel_match is None:
        return ['rust-toolchain.toml: channel must be an exact stable version ("X.Y.Z")']
    channel = channel_match.group(1)
    try:
        action_text = (root / SETUP_RUST_ACTION).read_text(encoding="utf-8")
    except OSError:
        return [f"{SETUP_RUST_ACTION}: missing"]
    # Scoped to the `toolchain:` input's own entry, not the whole file: a
    # `re.search` over the entire action text would resolve to whichever
    # `default: "..."` happens to appear first, which is only the toolchain
    # input's by accident of every other input's default being empty and
    # sitting after it in the file today.
    # `job_body` bounds a two-space `name:` block, which is exactly the
    # shape of an action.yml `inputs:` entry — no second helper needed.
    toolchain_input = job_body(action_text, "toolchain")
    if toolchain_input is None:
        return [f"{SETUP_RUST_ACTION}: no `toolchain:` input found"]
    default_match = re.search(r'default:\s*"([^"]+)"', toolchain_input)
    if default_match is None or default_match.group(1) != channel:
        found = default_match.group(1) if default_match else "<none>"
        return [
            f"{SETUP_RUST_ACTION}: toolchain input default {found!r} != "
            f"rust-toolchain.toml channel {channel!r}"
        ]
    return []
