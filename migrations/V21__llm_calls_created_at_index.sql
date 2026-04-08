-- Add index on llm_calls.created_at to speed up time-range aggregations
-- used by the admin usage summary endpoint.
CREATE INDEX IF NOT EXISTS idx_llm_calls_created_at ON llm_calls(created_at);
