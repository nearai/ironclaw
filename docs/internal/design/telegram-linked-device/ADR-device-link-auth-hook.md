# ADR — A vendor auth hook for device-link, and what it costs

**Status:** Proposed · **Date:** 2026-08-07 · **Ships with:** PR 2
**Required by:** `crates/domains/ironclaw_auth/AGENTS.md` — *"a vendor difference
belongs in recipe data or (as a last resort, **with an ADR**) a narrow declared
quirk hook."*
**Context:** [PROPOSAL.md](PROPOSAL.md) §3.2, §3.4, §11.1

This ADR exists because the design revokes a stated security invariant. It is
written to be the document a future reviewer finds when they ask "who decided
this, and did they understand it?"

---

## Decision

Add `VendorAuthRecipe::DeviceLink`, whose mechanics come from a
`DeviceLinkAdapter` implemented by the extension package rather than from recipe
data. The host keeps the state machine, TTLs, revisions, UI projection, and
credential lifecycle; the package supplies five methods and the protocol.

## Why data cannot express it

Every existing auth method is an HTTP conversation the host executes from a
descriptor: endpoints, JSON pointers, a token response shape. MTProto login is a
protocol handshake — `auth.ExportLoginToken` polled on a live session, datacenter
migration via `ImportLoginToken` on a *different* connection, SRP-based 2FA
whose parameters are short-lived server state. No descriptor vocabulary
expresses it, and inventing one would mean building an MTProto interpreter in
the auth engine.

The extension-runtime spec anticipated this exact case. Its "deliberately not
built" table lists *"per-vendor auth adapters, manual-validator trait"* with the
revisit trigger **"a vendor defeats the descriptor (add a narrow hook)."**
Telegram is that vendor; this is that hook.

## Why not bypass the auth engine entirely

The credential-account machinery is what makes everything downstream work:
blocked-run requirement satisfaction, the `AuthRequired` park-and-resume gate,
the derived connect affordance, unlink cleanup with quarantine on failed vendor
revocation. Rebuilding that beside auth would be strictly worse and would
duplicate a security-relevant state machine.

## What this revokes

`docs/reborn/extension-runtime/overview.md` justifies the no-adapter rule as
closing *"a whole class of attack surface — parameter override, state tampering,
token exfiltration."* This hook reopens that class for Telegram. Concretely:

**Package code sees the complete credential set.** The 2FA cloud password, the
login code, and the resulting session key all pass through the adapter, because
grammers must hold them to speak the protocol.

**IronClaw cannot verify the login it is displaying.** `DeviceLinkStep::Display`
returns whatever the adapter produces. Nothing binds that payload to a token
exported by a session IronClaw controls — IronClaw does not speak MTProto and is
**structurally incapable** of checking. A compromised adapter, or a compromised
`grammers`, can export a token on an attacker-controlled session, return it as
the QR, and the user scans it in good faith. The phone path needs no
substitution at all: the adapter already receives phone, code, and password, and
can drive a parallel `sign_in` on its own session.

## The honest compensation set

Two items. Neither is a host-side technical control.

1. **First-party package, reviewed in-tree.** The adapter is our code, under
   normal review.
2. **Trust in `grammers`** — which is unreviewed *by decision* (§11.1) and is
   the code that constructs the request and owns the socket.

An earlier draft listed a third — "the hook cannot reach flow storage or mint a
credential." That is true and it is aimed at the wrong asset. The asset is the
live handshake with the human, not the flow record.

**No host-side control can detect a substituted or parallel login.** State that
plainly rather than implying otherwise.

## The one control that is possible: detection

After `Completed`, show the user the resolved `vendor_user_ref` and ask them to
confirm in Telegram's *Settings → Devices* that exactly one new IronClaw device
exists, created just now.

**Be precise about what this catches, because it is less than it looks.** It
detects the crude **parallel-login** variant, where the attacker's session shows
up as a second device. It does **not** catch:

- **Key exfiltration** — a compromised adapter completes one legitimate login,
  stores the real blob, and copies the auth key out over its own socket. The
  Devices list shows exactly one new IronClaw device. The key *is* the device;
  a stolen one creates no second entry.
- **Named substitution** — the attacker-controlled session that exported the
  substituted token can register under any device name, including "IronClaw", so
  a count-based check reads as passing.

So it is worth shipping — it catches the crude variant and real-world phishing of
the linking ceremony — but it must not be described as making compromise
observable in general. Against the in-process attacker this ADR is actually
about, it is close to no control at all. `vendor_user_ref` answers a different
and narrower question ("did the right *account* get linked?"), which is a
wrong-account control, not an attacker control.

## The larger trade this sits inside

The adapter runs **in-process**, so the exposure is not bounded by the auth flow.
A malicious dependency release can read the process heap — every other user's
decrypted session key, the secrets master key, provider credentials — open its
own sockets, and never consult our validation seam. Every compensating control
in §3.4 is a source-level convention *inside the same address space as the code
it constrains*.

The alternative is an out-of-process sidecar, rejected on cost (roughly a
quarter of engineering). **That rejection is the security decision of this
feature**, and it belongs here rather than in a failure-mode table.

Because of it, the supply-chain controls in §11.1 — exact version pin,
`default-features = false` with an allowlist, a lockfile gate, vendoring or a git
rev, and named human review on every bump — **ship in the PR that adds the
dependency**, not in a later hardening pass. A dependency with full process
authority and a caret version range is not a controlled dependency.

## Consequences

- `ironclaw_extension_host` gains a binding slot and must retire
  `auth_never_binds_is_not_a_binding_field`, which encodes the invariant this
  ADR revokes. That retirement cites this document.
- Three package/family charter sentences need dated amendments. **They are not
  yet enumerated anywhere** — PROPOSAL §3.4 and §8.14 gesture at them without
  naming file and line. Enumerate them in the PR that lands the carve-out;
  candidates include the telegram package `AGENTS.md` "no network egress" line,
  the extensions-family "never holds a raw storable secret" line, and the
  extension-runtime "auth has no adapter trait" statement.
- The hook is **narrow by construction**: five methods (`begin`, `poll`,
  `submit_input`, `cancel`, `revoke`), no access to flow storage, cannot mint a
  credential. Its flow-time session port is pre-scoped; note that the *tool-side*
  factory's containment depends on the host-issued grant landing (PROPOSAL §5.1)
  — without it, that surface is adapter discipline, not structure.
- `ironclaw_auth` remains vendor-blind. If a second vendor ever needs this,
  it implements the same trait — no branch in the engine.

## Revisit

- **If a non-first-party extension ever wants this hook**, this ADR does not
  cover it. The entire compensation set is "our code, our review"; that argument
  does not survive third-party adapters.
- **If the sidecar is built** for another reason (Signal's licence forces one),
  move Telegram behind it and retire the in-process trade.
- **At N = 2 linked accounts**, revisit §11.1 — fleet-wide correlation changes
  the character of the risk, and that is the trigger, not general availability.
