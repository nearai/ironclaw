# Successor PR: `invocation_arguments_digest` snapshot pin

> Successor work from PR #3573 — addressing serrrfirat's #3 follow-up
> (the partial fix). Adds an explicit snapshot test pinning the digest
> for a known input so a future change to the hashing path or input-ref
> format is loud.

## Problem

`invocation_arguments_digest` (in `middleware/capability_port.rs`) was
moved from `format!("{:?}", input_ref)` (Debug-unstable) to
`input_ref.as_str()` in PR #3573. That fix removed the immediate
stability risk, but the digest itself is now *implicitly* pinned by L3
milestone snapshots and behavioral tests — not *explicitly* pinned by
a fixture.

Implicit pinning means: a future change to `LoopCapabilityInputRef` or
to the digest's hashing structure (length prefixing order, blake3
version, etc.) can shift the digest silently for repetition-detection
hooks that key on it. The L3 snapshots would catch *some* shifts but
they're not the right tool — they pin the milestone shape, not the
digest value.

## Scope

A single test in `capability_port.rs::tests` (or a snapshot file) that
pins the digest for one canonical `(capability_id, input_ref)` pair:

```rust
#[test]
fn invocation_arguments_digest_is_stable_for_known_inputs() {
    let invocation = CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("snapshot:v1").unwrap(),
        capability_id: CapabilityId::new("cap.snapshot").unwrap(),
        input_ref: CapabilityInputRef::new("input:snapshot:fixed").unwrap(),
    };
    let digest = invocation_arguments_digest(&invocation);
    let hex = hex::encode(digest);
    assert_eq!(
        hex,
        // captured one-time; if you find yourself updating this, ask
        // whether the digest change is intentional, and document it
        // in the digest stability contract.
        "INSERT_CAPTURED_HEX_HERE"
    );
}
```

Plus rustdoc on `invocation_arguments_digest` calling out the
stability contract:

> This digest is part of the **public hook contract**. Repetition-
> detection hooks key on `arguments_digest` across invocations; a
> shifted digest silently breaks them. Changing the hashing structure
> (length-prefix ordering, hasher choice, input field selection)
> requires:
> 1. Updating this contract test with the new fixture.
> 2. Surfacing the change in the cross-crate wire-format contract
>    section of `identity.rs`.
> 3. Bumping the hook framework's contract version if downstream
>    consumers exist.

## What this PR does NOT do

- Switch to a content-addressed *full* arguments digest (i.e., hashing
  the resolved JSON). That's a separate design: the current digest
  hashes only the input *ref*, not the resolved content. Hook authors
  who want content-keyed repetition detection should look at the
  arguments resolver, not this digest.
- Move the digest into the `BeforeCapabilityHookContext` constructor.
  It's already there (`arguments_digest: [u8; 32]`).

## Risk

Tiny. Single test + rustdoc.

## Effort

Small. ~10 minutes including digest capture.
