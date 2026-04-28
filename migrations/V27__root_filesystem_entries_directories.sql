-- Add explicit directory entries for the Reborn RootFilesystem DB backend.

ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS is_dir BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE root_filesystem_entries
    ALTER COLUMN contents SET DEFAULT ''::bytea;
