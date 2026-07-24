# Quarantined retired-activation traces

These 15 files are unmodified model traces harvested from live-canary run
`29837220214` at commit `c918d91943a84071726924b4e3e9a47d33d8f695`.
Each trace invokes the retired `builtin.extension_activate` capability.

They are preserved for provenance but are not active model/tool-choice
contracts. Rewriting a recorded tool call or response would fabricate model
evidence. Replace a quarantined case only by recording the scenario against the
current manifest-driven lifecycle, importing it as a review-required candidate,
reviewing its expectations and external-service doubles, and passing
`scripts/ci/check-reborn-qa-fixtures.sh`.

The promoted inventory remains authoritative in the parent
`case-manifest.json`; its `quarantined_model_cases` list must exactly account
for the JSON traces in this directory.
