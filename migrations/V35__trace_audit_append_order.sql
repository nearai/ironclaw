ALTER TABLE trace_audit_events
    ADD COLUMN audit_sequence BIGINT;

WITH ordered AS (
    SELECT
        tenant_id,
        audit_event_id,
        ROW_NUMBER() OVER (
            PARTITION BY tenant_id
            ORDER BY occurred_at ASC, audit_event_id ASC
        )::BIGINT AS audit_sequence
    FROM trace_audit_events
)
UPDATE trace_audit_events AS events
SET audit_sequence = ordered.audit_sequence
FROM ordered
WHERE events.tenant_id = ordered.tenant_id
  AND events.audit_event_id = ordered.audit_event_id;

ALTER TABLE trace_audit_events
    ALTER COLUMN audit_sequence SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_trace_audit_events_tenant_sequence
    ON trace_audit_events (tenant_id, audit_sequence);
