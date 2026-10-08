# tool-selection-jev — Jev as the turn-start tool classifier

Turn-start tool selection decides which deferred tools a conversation
advertises in its request's `tools` array. The loop host owns everything
around that decision; the decision itself goes through the loop-tier
`ToolSelectionClassifier` port (`ironclaw_loop_contracts`). This package
implements that port with Jev, TypeSafe's hosted classification model, reached
through a decisions API: TypeSafe's own by default, or any provider serving
the same API.

- **Code:** crate `ironclaw_tool_selection_jev` (this directory). No
  `manifest.toml`: it is a provider behind a loop-host port, not an
  installable extension, and declares no capability surface.
- **Layer:** `substrates`, like the other provider packages.
- **Depends on:** `ironclaw_loop_contracts` (the port), `ironclaw_host_api`
  (network-policy vocabulary), `ironclaw_network` (policy egress; every
  request goes through it).
- **Bound by:** the `ironclaw` binary only, which reads the API key host-side
  and hands composition the neutral port. Vendor-specific: a build can leave
  this package out and keep the port.

## Building one

`JevToolClassifier::new(endpoint, model, api_key, timeout)` takes a
`JevEndpoint` (`parse(url)` or `default()`), a model name (`DEFAULT_JEV_MODEL`
is `jev-latest`), a `JevApiKey` (zeroed on drop, never printed) and one
deadline for a whole classification.

The endpoint (default `https://api.typesafe.ai/v1/systemone`) must be a full
`https` URL with a host name and no userinfo, query or fragment. An error
names the rule broken and never repeats the URL. The egress policy is derived
from the URL: HTTPS to exactly its host and port, private address ranges
denied, so no other host is reachable. An IP address in place of the host name
is refused, since a private or loopback address could never pass the pin.

`jev-latest` is an alias that moves between Jev releases; name a pinned model
when selections must be reproducible. `classifier.slice_count(&request)` says
how many requests a classification will send.

## Request

`POST <endpoint>` with `Authorization: Bearer <key>` and a JSON body:

- `state`: `{conversation: [<user messages>], tools: {<name>: {parameters:
  [<parameter names>]}}}`. The conversation is the request's user messages,
  oldest first, cut to 16 KiB in all. Each tool sends at most 64 top-level
  parameter names of at most 128 bytes each.
- `questions`: one `noul` per tool, keyed by the tool's name: "How likely is
  it that `tools.<name>` will be used in the following `conversation`, given
  that it is described as: <description>?", the description cut to 1 KiB.
- Every cut falls on a character boundary, and the body is the same, byte for
  byte, on every run for the same request.
- Response: `{answers: {<name>: {type: "noul", noul: <0..1>}}, ...}`. Answers
  under names that were not asked are ignored. At most 1 MiB is read.

## Slicing

The decisions API publishes two limits for one request, and a large catalog
is split so that every request (slice) stays within both:

| Limit | Published | Budget used |
| --- | ---: | ---: |
| `state` plus the single longest question | 32,000 tokens | 30,000 (`DEFAULT_MAX_STATE_AND_QUESTION_TOKENS`) |
| `state` plus every question | 64,000 tokens | 60,000 (`DEFAULT_MAX_REQUEST_TOKENS`) |

Tokens are estimated at 256 fixed per request plus one per three bytes of
JSON, which over-counts: measured requests came to about 215 fixed tokens and
3.8 to 4.3 bytes a token.

Descriptions ride in the questions because `state` is counted against both
limits. In `state`, full 1 KiB descriptions ended a slice at about 75 tools;
in the questions a slice holds about 140, so 1,000 such tools take 7 requests
instead of 18. Against a live endpoint the two layouts chose the same top
three tools, with scores moving by little more than the model's run-to-run
noise; one borderline tool crossed the top-five cut-off.

The estimate was calibrated on English text. Text that tokenizes worse than
one token per three bytes (some non-Latin scripts) can make a full slice
exceed the published limit, which the endpoint refuses (`rejected`).

Slices are runs of consecutive tools in catalog order, each carrying the same
`conversation`, and the split depends only on the request. A tool too large
for a slice of its own still gets one. The slices are sent concurrently and
their probabilities merged into one vector before anything is chosen.

## Selection

Candidates are sorted by probability, highest first, ties in catalog order.
The top `max_tools` are taken, stopping at the first one that would take the
chosen tools' estimated schema tokens past `token_budget`. There is no
probability threshold. The selection's `scorer` is `jev:<model>`. Nothing is
sent when `max_tools` is zero or there are no candidates.

## Failures

Nothing is retried, and any failing slice fails the whole classification: a
partial vector is never ranked. The host decides what a failure means.

| Cause | Error label |
| --- | --- |
| The timeout elapsed (all slices together) | `timeout` |
| Connection or transport failure, policy denial, any other `5xx` | `unavailable` |
| `401`, `403` | `unauthorized` |
| `402` | `payment_required` |
| `429`, `503`, `529` (overload) | `rate_limited` |
| Any other status | `rejected` |
| Malformed JSON, a missing answer, a probability outside `[0, 1]`, a body over 1 MiB | `invalid_output` |

The endpoint must answer directly. A redirect is followed without the
`Authorization` header, so a URL the provider redirects usually ends as
`unauthorized`.

Tool descriptions are part of the question text, and there is no probability
threshold, so a description written to flatter itself can raise its own
tool's rank. The effect is bounded: only tools the caller is already
authorized for are candidates, and the host checks every chosen name.

## Confidentiality

Every classification sends the conversation's user text and every candidate
tool's name, description and parameter names to the configured provider, a
third party; retention is governed by that provider's terms. Nothing here
logs the conversation, the descriptions, the answers or the key. Logs are at
`debug!`, target `ironclaw::reborn::tool_selection`: the model, the chosen
names and probabilities, the slice count, latency and stable error labels.

## Tests

`cargo test -p ironclaw_tool_selection_jev` drives the classifier against a
loopback stub of the endpoint; nothing calls a real provider. The
`test-support` feature exports `with_stub_endpoint`, which points a classifier
at the stub with a policy that allows plain HTTP to loopback. Try it against a
real endpoint with `cargo run -p ironclaw --example jev_select_tools`.
