# github.get_job_logs

Fetch the plain-text logs for a single GitHub Actions **job** (not a whole run).

Use this to see *why* a CI check failed — the specific compiler error, failing
test, linter message, or action output — before deciding how to fix it. The
`job_id` comes from `github.get_workflow_run_jobs` (each job's `id`).

Returns the raw log text. The GitHub logs endpoint redirects to a short-lived
download URL; the host follows that redirect for you and returns the log body,
so you do not need to fetch any secondary URL yourself.

Input: `owner`, `repo`, `job_id`.
