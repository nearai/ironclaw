ALTER TABLE trace_submissions
    ADD COLUMN IF NOT EXISTS redaction_counts JSONB NOT NULL DEFAULT '{}'::JSONB;

ALTER TABLE trace_derived_records
    ADD COLUMN IF NOT EXISTS summary_model TEXT NOT NULL DEFAULT 'redacted-summary-hash-precheck-v1';

ALTER TABLE trace_derived_records
    ADD COLUMN IF NOT EXISTS tool_sequence JSONB NOT NULL DEFAULT '[]'::JSONB;

ALTER TABLE trace_derived_records
    ADD COLUMN IF NOT EXISTS tool_categories JSONB NOT NULL DEFAULT '[]'::JSONB;

ALTER TABLE trace_derived_records
    ADD COLUMN IF NOT EXISTS coverage_tags JSONB NOT NULL DEFAULT '[]'::JSONB;
