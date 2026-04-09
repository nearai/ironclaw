---
name: channel-docs
description: Write and revise channel documentation in a style consistent with the existing Telegram page.
license: MIT
compatibility: Workspace-local skill for agents that support repository skills.
metadata:
  author: GitHub Copilot
  version: "1.0"
---

# Channel docs skill

Use this skill when creating or editing pages under `channels/`.

## Required context

Before writing:

1. Read `channels/telegram.md`.
2. Read any existing sibling channel pages that are relevant to the requested change.

## Primary rule

Treat `channels/telegram.md` as the structural reference page for channel documentation.

Match its:

- information architecture
- level of operational detail
- instructional tone
- use of Mintlify components

Do not copy Telegram-specific content into other pages. Transfer the structure, not the platform details.

## Organization rule

Use one canonical section per concept.

- If a concept is already covered in the configuration section, do not create a second section that repeats the same content in prose.
- If a concept is already documented in detail elsewhere in the docs site, link to it instead of re-explaining it.
- Prefer cross-references over duplicate explanations.

Allowed repetition:

- repeat a short warning for dangerous actions
- repeat a short prerequisite for critical-path steps where failure is likely
- repeat only the minimum needed to prevent user error

Disallowed repetition:

- restating configuration semantics in multiple sections
- adding a full "locking down" section that duplicates policy options already explained in configuration
- repeating the same setup instruction in setup, troubleshooting, and examples without new context

## Expected structure

Each channel page should follow this order when relevant, unless there is a strong reason not to:

1. Introduction
  Explain what the channel does and how a user interacts with their IronClaw agent through it.

2. Quickstart note
  Near the top of the page, include a note that points users to `quickstart.mdx` if they have not set up their agent yet.

3. Setup steps
  Document the full setup flow in the order a user will perform it.
  This usually includes:
  - channel-side setup
  - IronClaw onboarding or channel installation
  - first-run or pairing test

4. Channel-side settings
  Capture settings controlled in the external service itself, such as privacy, visibility, permissions, webhooks, bot toggles, or app-level behavior.

5. Configuration options
  Document the IronClaw configuration surface for the channel.
  Include:
  - the relevant config file path, if one exists
  - an options overview table when there are multiple options
  - focused subsections for important options or option groups
  - sub-options when they materially affect behavior

6. Configuration examples
  Include realistic examples that map to common use cases.
  Prefer a few targeted examples over many slight variations.

7. Other relevant behavior
  Add a section for channel-specific operational topics that users are likely to need later, such as group participation, polling, mention behavior, delivery modes, or multi-user behavior.

8. Privacy and security considerations
  Call out privacy mode, visibility, ownership restrictions, data exposure, or token handling whenever relevant to the channel.

## Channel page expectations

Unless the product surface makes a section inapplicable, include:

1. A short introduction
2. A quickstart note near the top
3. End-to-end setup steps
4. Channel-side settings
5. IronClaw configuration options
6. Realistic configuration examples
7. Other relevant operational behavior
8. Privacy or security notes

Avoid adding low-value filler sections. A section is required only when it adds user-facing decision value.

Examples of commonly optional sections:

- prerequisites when they are obvious and already implied by setup
- implementation-detail internals that users do not need to act on
- generic "next steps" when the page is already a focused endpoint doc

## Content constraints

- Do not invent unsupported configuration keys or behaviors.
- Separate external platform settings from IronClaw settings.
- Keep procedures chronological.
- Prefer concise explanations over generic summaries.
- Use realistic example values.
- Keep images optional and purposeful.
- Explain webhook/auth security features in plain language before procedural steps.
- Do not add tunnel/provider operational details if those are intentionally centralized in another page.
- Document command references only where they are used by the described flow.

## Writing rules

- Write for a user who is configuring the channel for the first time.
- Use second person and direct language.
- Prefer Mintlify components that match the job: `Steps` for setup, `Accordion` for optional detail, `Note` or `Tip` for guidance, `Warning` for risky actions.
- Keep setup instructions procedural and in chronological order.
- Explain channel-specific terms the first time they appear.
- Use real configuration keys and real behavior only. Do not invent options.
- Include code fences with language tags.
- Use images only when they materially reduce setup friction.
- Keep internal links root-relative or repo-relative according to existing site conventions.

## Review checklist

Before finishing a channel doc change, check that:

- the page reads like `channels/telegram.md` in tone and density
- the setup flow is complete from account creation to first successful message
- channel-side settings and IronClaw-side settings are clearly separated
- every documented option is supported by the product
- examples use realistic values
- privacy or visibility behavior is documented when relevant
- each concept has a single owning section (no parallel duplicated sections)
- any repeated warning or prerequisite is brief, intentional, and safety-critical

## Output quality bar

The finished page should feel like it belongs next to `channels/telegram.md` and should be understandable to a first-time user without extra context.