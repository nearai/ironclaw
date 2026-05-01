-- Legal harness v1 — projects, documents, chats, chat messages.
--
-- Owned by Stream A (foundation). Streams B (chat) and C (DOCX export)
-- include this same migration verbatim in their branches so each PR is
-- testable in isolation; reviewer collapses duplicates at merge time.
--
-- Schema is canonical per the legal-harness v1 spec — do not modify
-- without coordinating with the other streams.

CREATE TABLE legal_projects (
    id TEXT PRIMARY KEY,                             -- ulid
    name TEXT NOT NULL,
    deleted_at INTEGER,                              -- soft delete timestamp, NULL = active
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    metadata TEXT                                    -- optional JSON blob
);

CREATE TABLE legal_documents (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES legal_projects(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,                      -- 'application/pdf' | OOXML mime
    storage_path TEXT NOT NULL,                      -- relative to ironclaw data dir
    extracted_text TEXT,                             -- nullable until extraction completes
    page_count INTEGER,
    bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,                            -- dedupe within a project
    uploaded_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_legal_documents_project ON legal_documents(project_id);
CREATE INDEX idx_legal_documents_sha256 ON legal_documents(sha256);

CREATE TABLE legal_chats (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES legal_projects(id) ON DELETE CASCADE,
    title TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_legal_chats_project ON legal_chats(project_id);

CREATE TABLE legal_chat_messages (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES legal_chats(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user','assistant','system','tool')),
    content TEXT NOT NULL,
    document_refs TEXT,                              -- JSON array of legal_documents.id
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_legal_chat_messages_chat ON legal_chat_messages(chat_id);
