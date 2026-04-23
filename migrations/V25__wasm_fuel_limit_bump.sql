-- The code default for wasm.default_fuel_limit was bumped from 10M to 500M
-- (limits.rs, config/wasm.rs), but databases that persisted the old 10M value
-- in the settings table still read it back at startup (DB-first resolution).
-- Delete the stale row so the code default takes effect.
DELETE FROM settings
WHERE key = 'wasm.default_fuel_limit'
  AND (value#>>'{}')::BIGINT = 10000000;
