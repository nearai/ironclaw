ALTER TABLE memory_documents ADD COLUMN summary_l0 TEXT;
ALTER TABLE memory_documents ADD COLUMN summary_l1 TEXT;

CREATE OR REPLACE FUNCTION list_workspace_files(
    p_user_id TEXT,
    p_agent_id UUID,
    p_directory TEXT DEFAULT ''
)
RETURNS TABLE (
    path TEXT,
    is_directory BOOLEAN,
    updated_at TIMESTAMPTZ,
    content_preview TEXT
) AS $$
BEGIN
    -- Normalize directory path (ensure trailing slash for non-root)
    IF p_directory != '' AND NOT p_directory LIKE '%/' THEN
        p_directory := p_directory || '/';
    END IF;

    RETURN QUERY
    WITH files AS (
        SELECT
            d.path,
            d.updated_at,
            COALESCE(d.summary_l0, LEFT(d.content, 120)) as content_preview,
            -- Extract the immediate child name
            CASE
                WHEN p_directory = '' THEN
                    CASE
                        WHEN position('/' in d.path) > 0
                        THEN substring(d.path from 1 for position('/' in d.path) - 1)
                        ELSE d.path
                    END
                ELSE
                    CASE
                        WHEN position('/' in substring(d.path from length(p_directory) + 1)) > 0
                        THEN substring(
                            substring(d.path from length(p_directory) + 1)
                            from 1
                            for position('/' in substring(d.path from length(p_directory) + 1)) - 1
                        )
                        ELSE substring(d.path from length(p_directory) + 1)
                    END
            END as child_name
        FROM memory_documents d
        WHERE d.user_id = p_user_id
          AND d.agent_id IS NOT DISTINCT FROM p_agent_id
          AND (p_directory = '' OR d.path LIKE p_directory || '%')
    )
    SELECT DISTINCT ON (f.child_name)
        CASE
            WHEN p_directory = '' THEN f.child_name
            ELSE p_directory || f.child_name
        END as path,
        EXISTS (
            SELECT 1 FROM memory_documents d2
            WHERE d2.user_id = p_user_id
              AND d2.agent_id IS NOT DISTINCT FROM p_agent_id
              AND d2.path LIKE
                CASE WHEN p_directory = '' THEN f.child_name ELSE p_directory || f.child_name END
                || '/%'
        ) as is_directory,
        MAX(f.updated_at) as updated_at,
        CASE
            WHEN EXISTS (
                SELECT 1 FROM memory_documents d2
                WHERE d2.user_id = p_user_id
                  AND d2.agent_id IS NOT DISTINCT FROM p_agent_id
                  AND d2.path LIKE
                    CASE WHEN p_directory = '' THEN f.child_name ELSE p_directory || f.child_name END
                    || '/%'
            ) THEN NULL
            ELSE MAX(f.content_preview)
        END as content_preview
    FROM files f
    WHERE f.child_name != '' AND f.child_name IS NOT NULL
    GROUP BY f.child_name
    ORDER BY f.child_name, is_directory DESC;
END;
$$ LANGUAGE plpgsql;
