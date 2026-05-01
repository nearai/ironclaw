-- X (Twitter) bookmarks pipeline.
--
-- Stores scraped bookmarks per user, runs LLM triage to classify each as
-- build/read/reference/dead, and exposes the queue via the gateway API.
--
-- Status values: 'untriaged' (default), 'build', 'read', 'reference', 'dead'.
--
-- Dedupe is per-user on `tweet_id`: the same tweet can be bookmarked
-- independently by multiple users.

CREATE TABLE x_bookmarks (
    id              UUID PRIMARY KEY,
    user_id         TEXT NOT NULL,
    tweet_id        TEXT NOT NULL,
    author_handle   TEXT,
    author_name     TEXT,
    text            TEXT NOT NULL DEFAULT '',
    url             TEXT,
    media_urls      JSONB NOT NULL DEFAULT '[]'::jsonb,
    quoted_tweet    TEXT,
    thread_id       TEXT,
    posted_at       TIMESTAMPTZ,
    scraped_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status          TEXT NOT NULL DEFAULT 'untriaged',
    rationale       TEXT,
    project_slug    TEXT,
    tags            JSONB NOT NULL DEFAULT '[]'::jsonb,
    triaged_at      TIMESTAMPTZ,
    triage_model    TEXT,
    UNIQUE (user_id, tweet_id)
);

CREATE INDEX idx_x_bookmarks_user_status ON x_bookmarks(user_id, status);
CREATE INDEX idx_x_bookmarks_user_scraped_at ON x_bookmarks(user_id, scraped_at DESC);
CREATE INDEX idx_x_bookmarks_user_posted_at ON x_bookmarks(user_id, posted_at DESC);
