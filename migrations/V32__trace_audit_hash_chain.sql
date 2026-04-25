ALTER TABLE trace_audit_events
    ADD COLUMN previous_event_hash TEXT;

ALTER TABLE trace_audit_events
    ADD COLUMN event_hash TEXT;
