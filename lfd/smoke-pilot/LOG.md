# LFD Run Log - smoke-pilot

RUN START 2026-07-07T13:41:50Z

## Cycles

### Cycle 1 - 2026-07-07T13:41:50Z
- hypothesis: The existing `smoke_builtin_tools` profile can run scripted text turns and produce scoreable outcomes.
- expected failure mode: The runner may fail to compile, the profile may not persist replies, or scorer/lint may reject the package.
- diagnostic: Run lint, Rust runner with `LFD_CASES`, score emitted outcomes, probe, and status.
- result: PASS. `lint.sh` printed `OK`; `status.sh` rendered; `probe.sh` emitted 2 perturbed cases; `cargo test --test reborn_lfd_runner -- --nocapture` emitted 2 `status: ran` outcomes; `score.sh --outcomes /tmp/lfd-smoke-out` returned `score: 1.0000`; probe scoring returned `score: 1.0000` and `gap_vs_dev: +0.0000`. First compile cost was high (~4m25s); post-patch probe output writes `map.json` outside `eval/probe/cases/`, and that default path is runner-consumable.

### Pin refresh verification - 2026-07-07T14:44:00Z
- hypothesis: The smoke pilot should still run after pins and `runner_hash` include the shared scorer tree.
- expected failure mode: The runner may produce outcomes whose hash no longer matches the scorer-side pin set, or probe execution may regress after shared probe fallback changes.
- diagnostic: Re-run the dev cases and probe cases through `cargo test --test reborn_lfd_runner -- --nocapture`, then score both output directories.
- result: PASS. Dev runner emitted 2 `status: ran` outcomes and `score.sh` returned `score: 1.0000`; probe runner emitted 2 `status: ran` outcomes and probe scoring returned `score: 1.0000`, `gap_vs_dev: +0.0000`.
