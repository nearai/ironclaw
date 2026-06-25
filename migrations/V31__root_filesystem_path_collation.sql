-- Use bytewise ordering for virtual filesystem paths.
--
-- The Postgres backend uses half-open path ranges (`path >= prefix AND
-- path < next_prefix`) for prefix scans. Those ranges are only stable across
-- locales when the stored path column uses the C collation.
--
-- Rollback plan:
-- 1. Stop writers that depend on bytewise path range semantics.
-- 2. Revert both path columns to the database default collation:
--      ALTER TABLE root_filesystem_entries
--          ALTER COLUMN path TYPE TEXT COLLATE "default";
--      ALTER TABLE root_filesystem_events
--          ALTER COLUMN path TYPE TEXT COLLATE "default";
-- 3. Recreate any dependent path indexes if PostgreSQL reports they were
--    rebuilt or invalidated during the ALTER COLUMN TYPE operation.

DO $$
BEGIN
    IF to_regclass('root_filesystem_entries') IS NOT NULL
        AND EXISTS (
            SELECT 1
            FROM pg_attribute
            WHERE attrelid = to_regclass('root_filesystem_entries')
                AND attname = 'path'
                AND NOT attisdropped
                AND attcollation <> 'pg_catalog."C"'::regcollation
        )
    THEN
        ALTER TABLE root_filesystem_entries
            ALTER COLUMN path TYPE TEXT COLLATE "C";
    END IF;

    IF to_regclass('root_filesystem_events') IS NOT NULL
        AND EXISTS (
            SELECT 1
            FROM pg_attribute
            WHERE attrelid = to_regclass('root_filesystem_events')
                AND attname = 'path'
                AND NOT attisdropped
                AND attcollation <> 'pg_catalog."C"'::regcollation
        )
    THEN
        ALTER TABLE root_filesystem_events
            ALTER COLUMN path TYPE TEXT COLLATE "C";
    END IF;
END $$;
