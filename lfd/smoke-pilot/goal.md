# Goal: prove the shared LFD runner and scorer work end to end

This is an infrastructure pilot, not a roadmap lane. It uses the existing
`smoke_builtin_tools` runner profile to verify that:

- visible eval inputs can be executed by `reborn_lfd_runner`;
- runner outcomes can be scored by `lfd/_shared/scorer/score_core.py`;
- lint, probe, status, pins, and sealed dev answers work together.

Acceptance for this pilot: dev score is `1.0000`, lint prints `OK`, probe
emits a perturbed case set, status renders, and no wrapper scans build output.
