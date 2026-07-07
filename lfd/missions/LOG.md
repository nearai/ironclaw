# LFD Run Log — missions

RUN START 2026-01-01T00:00:00Z
<!-- Sentinel timestamp: replace this line with the real ISO-8601 UTC run start before cycle 1. status_core reads the first RUN START line. -->

## Package Notes

- Eval size: 30 dev cases and 12 holdout cases. This is below the 200-case enumerability threshold, so positive dev scores are weak until the probe gap is stable and holdout passes.
- Answers: dev answers are sealed in `harness/answers.dev.json`; holdout answers live outside the repo under `/Volumes/NVME/ironclaw-lfd/holdout/missions/`.
- First optimizer action: run Stage 0 tests, then `harness/score.sh`, `harness/probe.sh`, and `harness/status.sh` before logging a cycle hypothesis.

## Cycles

No optimization cycles have run yet. Before each cycle, append an entry with hypothesis, expected failure mode, diagnostic, change, result, and reflection.

## Final Report

Not started.
