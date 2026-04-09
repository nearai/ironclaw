-- Per-user quota columns for fail-closed enforcement.
-- NULL = no quota assigned (denied for non-admin users).
ALTER TABLE users ADD COLUMN max_routines INTEGER DEFAULT NULL;
ALTER TABLE users ADD COLUMN max_cost_per_day_cents BIGINT DEFAULT NULL;
