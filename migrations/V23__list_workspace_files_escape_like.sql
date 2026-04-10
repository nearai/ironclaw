-- Fix LIKE-metacharacter injection in list_workspace_files().
--
-- The original V1 function concatenated user-supplied directory names and
-- path-derived child names directly into LIKE patterns:
--
--     d.path LIKE p_directory || '%'
--     d2.path LIKE (p_directory || f.child_name) || '/%'
--
-- That meant `_` and `%` inside the input strings were interpreted as
-- LIKE wildcards. Two consequences:
--
--   1. The outer `WHERE … LIKE p_directory || '%'` over-fetches sibling
--      directories (e.g. listing `foo_bar/` also pulls in `fooXbar/...`).
--      Downstream callers don't see this because the function later
--      filters via `child_name`, but it wastes I/O.
--
--   2. The inner EXISTS subqueries that compute `is_directory` use
--      `LIKE (… || child_name) || '/%'` against the same column. If a
--      sibling document path matches the wildcarded form, the function
--      reports `is_directory = true` for what is actually just a file.
--      That is a real correctness bug.
--
-- This migration:
--   * Adds an immutable SQL helper `ironclaw_escape_like(s TEXT)` that
--     prefixes `\`, `%`, and `_` with `\`. Backslash is escaped first so
--     the escapes we add don't get re-escaped.
--   * Replaces `list_workspace_files()` with a version that escapes both
--     `p_directory` and `f.child_name` before they hit a LIKE, and adds
--     `ESCAPE '\'` to every LIKE clause so PostgreSQL knows to interpret
--     `\` as the escape prefix.
--
-- Backwards compatible: same input/output signature. Behaviour only
-- differs for inputs that contain LIKE metacharacters, where the new
-- version returns the *correct* result.

CREATE OR REPLACE FUNCTION ironclaw_escape_like(s TEXT) RETURNS TEXT AS $$
    SELECT replace(replace(replace(s, '\', '\\'), '%', '\%'), '_', '\_');
$$ LANGUAGE SQL IMMUTABLE;

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
DECLARE
    p_directory_esc TEXT;
BEGIN
    -- Normalize directory path (ensure trailing slash for non-root).
    IF p_directory != '' AND NOT p_directory LIKE '%/' THEN
        p_directory := p_directory || '/';
    END IF;

    -- Escaped form for LIKE clauses. Plain p_directory is still used for
    -- `=`, length(), and string concatenation, where wildcards are
    -- harmless.
    p_directory_esc := ironclaw_escape_like(p_directory);

    RETURN QUERY
    WITH files AS (
        SELECT
            d.path,
            d.updated_at,
            LEFT(d.content, 200) as content_preview,
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
          AND (p_directory = '' OR d.path LIKE p_directory_esc || '%' ESCAPE '\')
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
                CASE
                    WHEN p_directory = '' THEN ironclaw_escape_like(f.child_name)
                    ELSE p_directory_esc || ironclaw_escape_like(f.child_name)
                END
                || '/%' ESCAPE '\'
        ) as is_directory,
        MAX(f.updated_at) as updated_at,
        CASE
            WHEN EXISTS (
                SELECT 1 FROM memory_documents d2
                WHERE d2.user_id = p_user_id
                  AND d2.agent_id IS NOT DISTINCT FROM p_agent_id
                  AND d2.path LIKE
                    CASE
                        WHEN p_directory = '' THEN ironclaw_escape_like(f.child_name)
                        ELSE p_directory_esc || ironclaw_escape_like(f.child_name)
                    END
                    || '/%' ESCAPE '\'
            ) THEN NULL
            ELSE MAX(f.content_preview)
        END as content_preview
    FROM files f
    WHERE f.child_name != '' AND f.child_name IS NOT NULL
    GROUP BY f.child_name
    ORDER BY f.child_name, is_directory DESC;
END;
$$ LANGUAGE plpgsql;
