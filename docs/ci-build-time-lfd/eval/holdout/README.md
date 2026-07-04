# Holdout workflow set

The holdout set is intentionally not encoded as fixed answer data in this directory. Use fresh run IDs from GitHub Actions when evaluating a candidate branch.

Recommended holdout workflows:

- `live-canary.yml`
- `coverage.yml`
- `reborn-playwright.yml`

For live canary, pass explicit run IDs to the scorer because scheduled runs can be product-red while still carrying useful build-time timing.

Example:

```bash
BASE_RUN_IDS=28694683262 CANDIDATE_RUN_IDS=<candidate-live-canary-run> \
  docs/ci-build-time-lfd/harness/score.sh --repo nearai/ironclaw
```
