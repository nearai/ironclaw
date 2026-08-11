# Complement live-tier plan

Governing Lighthouse G0C status: **Revisions Required**.

Publication-package status: **contribution-grade; pending exact-head review**.
This is an unexecuted, unfrozen plan and allowlist, not an executable live-tier
result. Review history for the underlying investigation does not approve these
tracked exact-head bytes or establish any live result.

Use Complement as the homeserver lifecycle and federation substrate. Wrap the
stable Complement Crypto workflow boundary behind a versioned IronClaw RPC
adapter; do not bind the neutral harness to its evolving private Client API.

Survey pins observed on 2026-08-11:

- Complement Crypto `f884f7535488352af2eef53335fe6d7835626040`.
- Its declared Complement dependency
  `db8c562a8790cac086c05c4212173fdf2044a9a0` is the proposed baseline for a
  future executable leg; this package does not freeze or execute it.
- Complement `b6dbb972c99e05c1ebc63d21a27a65b5b53ceb06` is a separately named
  compatibility leg and must not silently replace the baseline.

The exact survey pins, normalized inventory receipt hashes, and bounded
recapture commands are recorded in the
[evidence provenance index](evidence-provenance.json#/candidates/complement_live_tier).
Those hashes identify the retained survey artifacts; they are not a substitute
for the raw bytes and do not claim a live execution. An authorized reviewer
recaptures the survey in an access-controlled disposable checkout at the listed
commits and compares new receipts without overwriting historical evidence.

Required live families are baseline encrypted send/decrypt, two-homeserver
federation, request and response MITM faults, candidate SIGKILL/restart,
disposable-store lifecycle and corruption, crypto recovery, cancellation and
stale-writer rejection, plus separately declared backup and verification.

The pinned Complement Crypto hit list contains 49 E2EE items. Forty-eight are
adopted scenario inputs. QR verification is deferred only when the exact
candidate graph proves the optional QR feature absent; if scheduled but
unsupported it is `failed`, not `not_applicable`. Notification-surface cases
outside the Matrix channel cannot erase their underlying backup, Olm-wedge,
history, and room-event behaviors; those behaviors require neutral substitute
scenarios.

Runtime result vocabulary is closed to `supported`, `failed`, `uncertain`,
`infeasible`, and `not_applicable`. `uncertain` is required when response loss
or a crash crosses a commit/acknowledgement boundary and retained evidence
cannot prove whether the candidate consumed or durably applied the response;
it must not be promoted to `supported` or collapsed into an ordinary failed
assertion. Planning `deferred` is never emitted as a runtime result. Every
adopted scenario must receive a neutral scenario hash before execution.

Any future live runner must record immutable server images/configuration, independent
client provenance, both encryption directions, sanitized MITM receipts, exact
fault barriers, and same-store proof. Complement and Complement Crypto are
test infrastructure, not the candidate identity or a passing oracle by
themselves. Nothing in this plan accepts Gate 0, G0C, or an ADR.
