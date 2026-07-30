-- Keep ordered-projection path ranges bytewise, matching the authoritative
-- root_filesystem_entries.path column.
--
-- Ordered queries use half-open descendant ranges. The database-default
-- collation can order '/' and '0' differently from bytewise ordering, making
-- an existing projected row invisible on databases initialized with locales
-- such as en_US.utf8.
--
-- Compatibility: this changes only comparison/index ordering. Stored paths and
-- projection rows are unchanged.
--
-- Rollback plan:
-- 1. Stop writers and ordered readers.
-- 2. Revert the column to the database-default collation:
--      ALTER TABLE root_filesystem_ordered_index_rows
--          ALTER COLUMN path TYPE TEXT COLLATE "default";
-- 3. Recreate dependent indexes if PostgreSQL reports they were rebuilt or
--    invalidated by the ALTER COLUMN operation.

DO $$
BEGIN
    IF to_regclass('root_filesystem_ordered_index_rows') IS NOT NULL
        AND EXISTS (
            SELECT 1
            FROM pg_attribute
            WHERE attrelid = to_regclass('root_filesystem_ordered_index_rows')
                AND attname = 'path'
                AND NOT attisdropped
                AND attcollation <> 'pg_catalog."C"'::regcollation
        )
    THEN
        ALTER TABLE root_filesystem_ordered_index_rows
            ALTER COLUMN path TYPE TEXT COLLATE "C";
    END IF;
END $$;
