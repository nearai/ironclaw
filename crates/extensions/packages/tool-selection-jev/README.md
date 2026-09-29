# tool-selection-jev — Jev as the turn-start tool classifier

Turn-start tool selection decides which tools a conversation advertises in
its request's `tools` array: before the first model call, and again at a
turn boundary once the provider's prompt cache can no longer be warm (an
idle gap past the cache lifetime, a model change) or a selected tool was
revoked. The choice
goes through the loop-tier `ToolSelectionClassifier` port
(`ironclaw_loop_contracts::tool_selection`). The loop host bundles a local
classifier (a tool ranker plus score thresholds); this package is the other
one: Jev, TypeSafe's hosted classification model, reached through a decisions
API: TypeSafe's own by default (<https://docs.typesafe.ai/api>), or any
provider serving the same decisions API. A deployment uses exactly one of the
two classifiers.

- **Code:** crate `ironclaw_tool_selection_jev` (this directory). No
  `manifest.toml`: not an installable extension, no capability surface. It is
  a provider behind a loop-host port.
- **Layer:** `substrates`, like the other provider packages.
- **Depends on:** `ironclaw_loop_contracts` (the port), `ironclaw_host_api`
  (network-policy vocabulary), `ironclaw_network` (policy egress; every
  request goes through it).
- **Bound by:** the `ironclaw` binary when `[tool_selection] classifier =
  "jev"` (or `REBORN_TOOL_PREFETCH_CLASSIFIER=jev`) and turn-start selection
  is on. The API key is read host-side from the environment variable named by
  `[tool_selection.jev] api_key_env` (default `TYPESAFE_API_KEY`); an unset
  or empty key refuses startup. Vendor-specific: an upstream change can leave
  this package out and keep the port.

## Provider

Three settings pick the provider; each has an environment override, which
wins:

| `[tool_selection.jev]` | Environment | Default |
| --- | --- | --- |
| `endpoint` | `REBORN_TOOL_PREFETCH_JEV_ENDPOINT` | `https://api.typesafe.ai/v1/systemone` |
| `model` | `REBORN_TOOL_PREFETCH_JEV_MODEL` | `jev-latest` |
| `api_key_env` | `REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV` | `TYPESAFE_API_KEY` |

To use another provider serving the same decisions API, set its full
endpoint URL, the model id it serves and the name of the variable holding its
key; the request and response are the same. TypeSafe keys are invite-only for
now.

The endpoint must be a full `https` URL with a host name and no userinfo,
query or fragment; anything else refuses startup, with an error that names
the rule broken but never repeats the URL. `JevEndpoint` checks the same
rules again and derives the egress policy from the URL: HTTPS to exactly its
host and port (443 unless the URL names one), private address ranges denied.
No wildcard host is accepted, and no other host is reachable.

## Request

`POST <endpoint>`, `Authorization: Bearer <key>`, model `jev-latest` by
default, TypeSafe's flagship alias. The alias moves between Jev releases, so
each success log line carries `served_model`, the model the response's
`model` field names (only a short, plain model id; anything else is
dropped). `model` pins a version (for example `jev-1.13.0`) when selections
must be reproducible.

- `state`: `{conversation: [<user messages>], tools: {<name>: {description,
  parameters: [<parameter names>]}}}`, sent as a JSON object. The decisions
  API accepts `state` as a string, an object or an array, so it is not
  serialized to a string; the questions refer to its fields by
  path (`tools.<name>`, `conversation`). At a conversation's first turn,
  `conversation` is its first user message, cut to 16 KiB. At a
  re-selection it is a window of the conversation's user messages, oldest
  first: the `[tool_selection] context_messages` most recent ones (default
  16, each cut to 16 KiB, at most 32 KiB in all), plus the first message if
  it still fits. Jev scores the whole conversation at once, so it gets whole
  messages, not the local classifier's segments; only when the list would
  pass half of a request's token budget (never at the default budget) are
  the oldest messages dropped, and the newest is always sent. Tools are keyed
  by provider tool name. The tools the conversation has already called are
  pinned by the host (asked about, never taking a slot).
- `questions`: one `noul` per tool, keyed by the tool's name:
  "How likely is it that `tools.<name>` will be used in the following
  `conversation`?"
- Response: `{model, answers: {<id>: {type: "noul", noul: <0..1>}},
  usage: {input_tokens, output_tokens}}`, as the decisions API documents it. Answers
  under ids that were not asked are ignored; `model` and `usage` are
  optional.

## Slicing

A large catalog does not fit one request. The limits are TypeSafe's
published figures for `jev-1.13.0`: 64k tokens per request, 32k for the state
plus the longest question. Another provider may differ and publishes its own
limits. The budget is not a config key: `JevToolClassifier::with_max_slice_tokens`
sets it in code, and the binary uses the default. Tokens are estimated
at one per three bytes of JSON, and the catalog is split, in catalog order,
into slices of at most 24,000 estimated tokens each. Every slice carries the
same `conversation`. The slices are sent concurrently and their
probabilities merged into one vector before anything is chosen. At about 70
estimated tokens per tool, 1,000 tools is about three requests.

## Selection

Pinned tools (the host's floor and the operator's extras) are asked about
but never take a slot. The rest are sorted by probability, highest first,
ties in catalog order; the top N are kept, N being `max_tools` minus the
floor and the bridges. If they exceed the token budget, the lowest
probabilities are dropped until they fit. There is no probability threshold.
The host then checks the answer and adds the floor, as for any classifier.

## Failures

Any failure fails the whole classification; a partial vector is never
ranked, since the tools in a missing slice could never be chosen:

| Cause | Error label |
| --- | --- |
| `timeout_ms` elapsed (all slices and retries together) | `timeout` |
| Connection or transport failure, policy denial | `unavailable` |
| `401`, `403` | `unauthorized` |
| `402` (the account cannot pay), not retried | `payment_required` |
| `422` and any other unexpected status | `rejected` |
| An overload status (`429` rate limit, `503` model at capacity, `529` overloaded) that does not clear inside the timeout (retried with backoff, `retry-after` honoured) | `rate_limited` |
| Malformed JSON, a missing answer, a probability outside `[0, 1]` | `invalid_output` |

At a conversation's first selection the loop host then freezes the core tool
set (the authorized `CORE_TOOL_NAMES` plus the floor and extras) and records
it with the label, so the conversation keeps it until its next re-selection.
At a re-selection the loop host instead keeps the selection in force and
records nothing (a revocation still removes the revoked tool). The local
classifier is never used as a fallback.

## Confidentiality

`jev` sends the conversation context (the first user message, and at each
re-selection a window of up to `context_messages` recent user messages) and
every candidate tool's name, description and parameter names to the
configured Jev provider (TypeSafe by default), a third party. There is no
on-host option; retention is governed by that provider's terms. `local` is
the default. Nothing here logs the conversation, the
descriptions sent, the answers received or the key: logs (at `debug!`,
target `ironclaw::reborn::tool_prefetch`) carry the requested and served
model, the chosen names and probabilities, the probability of the first tool
left out, the slice count, latency and reported input tokens. TypeSafe's
published price for `jev-1.13.0` is $0.042 per million input tokens, which the
benchmark uses unless told another; another provider may charge differently
and publishes its own price.

The selection history records the scorer as `jev:<model>` (for example
`jev:jev-latest`), whichever provider served it. Entries written by earlier
builds keep the label they were stored with.

## Tests

`cargo test -p ironclaw_tool_selection_jev` drives the classifier against a
loopback stub of the endpoint (the `test-support` feature points it there):
the request shape and the configured URL's own path, top-N and ties, the
token budget, concurrent slicing and merging, every failure mode, a retried
`429`, `503` and `529`, a `402` that is not retried, and a log capture proving
no conversation text, description, answer or key is logged. Nothing calls a
real provider. Unit tests drive the production endpoint and egress pin
through a recording transport and a fixed resolver (no DNS, no socket): the
default classifier posts to `https://api.typesafe.ai/v1/systemone` with
`jev-latest` and the key as the bearer token; a configured endpoint gets the
request at its own path, and its pin refuses any other host, another port
and plain HTTP; and every malformed endpoint is refused.
