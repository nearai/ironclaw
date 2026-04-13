-- Scope grants: cross-user read/write access control.
--
-- A scope grant allows one user to read (or read+write) another user's
-- workspace data.  The `scope` column is the target user_id whose data
-- becomes accessible.  `writable = true` means the grantee can also
-- write to that scope (via writable memory layers).
--
-- Example: (user_id='andrew', scope='household', writable=true) means
-- Andrew can read and write data stored under the 'household' user scope.

CREATE TABLE IF NOT EXISTS scope_grants (
    user_id    TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope      TEXT        NOT NULL,
    writable   BOOLEAN     NOT NULL DEFAULT FALSE,
    granted_by TEXT        REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, scope)
);

CREATE INDEX IF NOT EXISTS idx_scope_grants_user  ON scope_grants(user_id);
CREATE INDEX IF NOT EXISTS idx_scope_grants_scope ON scope_grants(scope);
