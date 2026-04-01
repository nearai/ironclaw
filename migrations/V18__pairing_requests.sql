-- Replaces file-based pairing store (~/.ironclaw/{channel}-pairing.json).
-- A pending request has owner_id = NULL until approved.
CREATE TABLE pairing_requests (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    channel     TEXT        NOT NULL,
    external_id TEXT        NOT NULL,
    code        TEXT        NOT NULL UNIQUE,
    owner_id    TEXT        REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    approved_at TIMESTAMPTZ
);

CREATE INDEX idx_pairing_requests_code    ON pairing_requests (code);
CREATE INDEX idx_pairing_requests_channel ON pairing_requests (channel, external_id);
