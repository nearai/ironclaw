# Live-canary model traces

These fixtures were harvested from the Reborn WebUI v2 live-QA matrix in
[run 29837220214](https://github.com/nearai/ironclaw/actions/runs/29837220214),
executed against commit `c918d91943a84071726924b4e3e9a47d33d8f695`.

The catalog contains 47 live-QA cases:

- 27 cases reached the model and retain an active case-named trace in this
  directory.
- 15 cases reached the model but their exact harvested traces invoke the
  retired `builtin.extension_activate` lifecycle tool. Their recorded
  user/model/tool steps are preserved unmodified under
  `quarantined_retired_activation/` as historical provenance and are not active
  model/tool-choice contracts.
- Five Slack connect/preflight cases did not invoke the model and therefore
  cannot produce an LLM trace: `qa_3a_slack_connect`, `qa_5a_slack_connect`,
  `qa_7a_slack_product_channel_connect`, `qa_8a_slack_connect`, and
  `qa_9a_slack_connect`.

`case-manifest.json` is the promoted inventory from that run. The Rust
contract derives active/quarantined/no-model coverage from it, while each
active trace declares its required tools in `expects.tools_used`; there is no
second Rust case table.

The committed traces retain user prompts, model responses, tool names and
arguments, and final assistant text. Recorded tool-result payloads are removed
because the serve runtime did not capture the corresponding capability HTTP
exchanges, so these are model/tool-choice contracts rather than full runtime
replay fixtures. Removing the payloads also prevents live provider content from
entering the repository. Remaining emails, local paths, names, and Slack entity
identifiers are normalized before the repository fixture scrub check runs.
Raw per-run traces remain available only inside the live-QA runner and are
explicitly excluded from public Actions artifact uploads.

Quarantine is intentionally lossless. We do not rename the recorded tool,
delete a recorded response step, or invent a replacement model answer. A case
can leave quarantine only after a fresh attended run against the current
manifest-driven lifecycle is imported, reviewed, scrubbed, and promoted using
the repository candidate workflow in the parent README.

The source run had 44 successful cases. The three QA-9 cases still emitted
complete diagnostic model traces but failed their live outcome probes:

- `qa_9b_routine_dm_delivery_exactly_once`: Slack extension activation
  precondition failed after routine creation.
- `qa_9c_slack_digest_names_not_ids`: the reply leaked raw Slack user IDs; its
  trace is also quarantined because it invokes the retired activation tool.
- `qa_9d_routine_per_trigger_delivery_target`: Slack extension activation
  precondition failed after routine creation.

Their fixture contracts assert only the intended model/tool path and do not
bless the failing final outcome.
