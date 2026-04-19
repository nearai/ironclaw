natively doesnt work. few ways to add functionality tho

for current iteration of Codex worker, options are:

  1. Pass secrets manually at container startup with Docker/Compose env, for
     example OPENAI_API_KEY, then Codex sees it.
  2. Use IronClaw’s built-in worker mode if you need per-job credential grants
     from create_job.
  3. Add an adapter mode to the Codex worker that speaks IronClaw’s worker
     protocol: read IRONCLAW_JOB_ID, IRONCLAW_WORKER_TOKEN, call /credentials,
     inject returned env vars, then run codex exec.
