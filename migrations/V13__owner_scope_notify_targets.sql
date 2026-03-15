-- Remove the legacy 'default' sentinel from routine notifications.
-- A NULL notify_user now means "resolve the configured owner's last-seen
-- channel target at send time."

ALTER TABLE routines
    ALTER COLUMN notify_user DROP NOT NULL,
    ALTER COLUMN notify_user DROP DEFAULT;

UPDATE routines
SET notify_user = NULL
WHERE notify_user = 'default';
