-- V26: Legal harness — projects, documents, chats, messages.
--
-- Schema for the `legal` skill: a chat-with-legal-documents harness.
-- See docs/legal-harness.md (forthcoming) and SKILL.md at skills/legal/SKILL.md.
--
-- Stream A (this migration) introduces all four tables. Streams B (chat) and
-- C (DOCX export) consume the same shape; do not mutate this migration after
-- merge — add a follow-up Vnn__legal_*.sql instead.
--
-- Schema parity: the libSQL backend mirrors this migration in
-- src/db/libsql_migrations.rs (INCREMENTAL_MIGRATIONS entry version 26,
-- "legal_harness"). Keep the two in sync when extending.

CREATE TABLE legal_projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    deleted_at  BIGINT,
    created_at  BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
    metadata    TEXT
);

CREATE TABLE legal_documents (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES legal_projects(id) ON DELETE CASCADE,
    filename        TEXT NOT NULL,
    content_type    TEXT NOT NULL,
    storage_path    TEXT NOT NULL,
    extracted_text  TEXT,
    page_count      INTEGER,
    bytes           INTEGER NOT NULL,
    sha256          TEXT NOT NULL,
    uploaded_at     BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
);

CREATE INDEX idx_legal_documents_project ON legal_documents(project_id);
CREATE INDEX idx_legal_documents_sha256  ON legal_documents(sha256);

CREATE TABLE legal_chats (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES legal_projects(id) ON DELETE CASCADE,
    title       TEXT,
    created_at  BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
);

CREATE INDEX idx_legal_chats_project ON legal_chats(project_id);

CREATE TABLE legal_chat_messages (
    id              TEXT PRIMARY KEY,
    chat_id         TEXT NOT NULL REFERENCES legal_chats(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    document_refs   TEXT,
    created_at      BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT
);

CREATE INDEX idx_legal_chat_messages_chat ON legal_chat_messages(chat_id);
