# CI build-time LFD log

## 2026-07-04

Observed recent main workflow timings with `gh run list` and `gh run view`.

Key finding: the first OVH nextest archive PR is a negative result for PR CI latency. It made test consumers faster but increased the overall Reborn workflow active time because the archive producer became a serial dependency.

Current working hypothesis:

- OVH should not be the single central builder for all PR tests.
- OVH can still help as a trusted producer for workflows that already duplicate identical setup across shards, especially live canary and binary-based browser tests.
- The highest-signal next move is build-once artifact fan-out for repeated binary/WASM setup, measured by workflow active time, not isolated job duration.

Evidence captured:

- Tests (Reborn) main run `28696684191`: active 597s.
- Reborn E2E main run `28696684177`: active 487s.
- Reborn Coverage main run `28696684175`: active 574s.
- Tests (Legacy) main run `28228462994`: active 1888s.
- Live Canary run `28694683262`: active 1403s, failed, but Reborn WebUI v2 QA shards still expose repeated setup cost.
- OVH archive experiment:
  - baseline `28681653394`: 603s
  - full-feature archive `28684600515`: 711s
  - libsql archive `28685265502`: 793s

Next action for an implementation branch:

Run `docs/ci-build-time-lfd/harness/score.sh` against baseline and candidate runs, then start with live canary or Reborn E2E binary artifact fan-out. Do not merge an optimization unless it beats the primary workflow set and does not regress the probe set.
