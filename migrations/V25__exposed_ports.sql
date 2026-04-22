-- Add exposed_ports column to agent_jobs for persisting container port mappings.
-- Stored as JSON text: an array of {"container_port": N, "host_port": M} objects.
-- NULL when no ports were exposed (the common case).
ALTER TABLE agent_jobs ADD COLUMN exposed_ports TEXT;
