-- Audit trail for LLM-as-Judge evaluations.
--
-- Every judge verdict (Allow/Deny/Ambiguous) is persisted here so that
-- security reviews can reconstruct what the judge saw and decided, even
-- after the in-process log has rotated. This table is append-only;
-- no rows are ever updated or deleted by the application.
--
-- Columns:
--   tool_name    The tool the agent attempted to invoke.
--   verdict      The judge's final verdict: 'Allow', 'Deny', or 'Ambiguous'.
--   attack_type  Detected attack category (e.g. 'data_exfiltration'), or NULL.
--   confidence   Normalised score 0.0-1.0; 0.0 means judge was unavailable.
--   reasoning    Judge's explanation. 'Judge unavailable — failing open'
--                is a sentinel for fail-open events (confidence=0.0).
--   latency_ms   Wall-clock time of the judge call in milliseconds.
--   created_at   UTC timestamp when the record was inserted.

CREATE TABLE judge_verdicts (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tool_name   TEXT        NOT NULL,
    verdict     TEXT        NOT NULL,
    attack_type TEXT,
    confidence  FLOAT       NOT NULL,
    reasoning   TEXT        NOT NULL,
    latency_ms  BIGINT      NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_judge_verdicts_tool    ON judge_verdicts(tool_name);
CREATE INDEX idx_judge_verdicts_verdict ON judge_verdicts(verdict);
CREATE INDEX idx_judge_verdicts_created ON judge_verdicts(created_at DESC);
