"""
Persistent workspace memory — mirrors src/workspace/.

Provides hybrid search (full-text + semantic) over stored documents.
The default backend is an in-memory store; swap for a PostgreSQL +
pgvector backend for production.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

logger = logging.getLogger(__name__)


@dataclass
class WorkspaceDocument:
    """A stored document in the workspace."""

    path: str
    content: str
    created_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    metadata: dict[str, Any] = field(default_factory=dict)


class Workspace:
    """
    Persistent memory with hybrid search.

    Mirrors the Rust ``Workspace`` struct.  In production, back this with
    PostgreSQL + pgvector for proper vector search.  The in-memory
    implementation supports all four tool operations: search, write, read,
    and tree listing.
    """

    def __init__(self) -> None:
        self._docs: dict[str, WorkspaceDocument] = {}

    # ------------------------------------------------------------------
    # CRUD
    # ------------------------------------------------------------------

    async def write(self, path: str, content: str, metadata: dict | None = None) -> None:
        """Write or overwrite a document at path."""
        doc = self._docs.get(path)
        if doc:
            doc.content = content
            doc.updated_at = datetime.now(timezone.utc)
            if metadata:
                doc.metadata.update(metadata)
        else:
            self._docs[path] = WorkspaceDocument(
                path=path,
                content=content,
                metadata=metadata or {},
            )
        logger.debug("Workspace write: %s (%d bytes)", path, len(content))

    async def read(self, path: str) -> str | None:
        """Read a document. Returns None if not found."""
        doc = self._docs.get(path)
        return doc.content if doc else None

    async def delete(self, path: str) -> bool:
        """Delete a document. Returns True if it existed."""
        if path in self._docs:
            del self._docs[path]
            return True
        return False

    async def append(self, path: str, content: str) -> None:
        """
        Append content to an existing document, or create it if absent.

        Used by the compaction system to write to daily log files
        (e.g. ``daily/2026-03-16.md``).  Mirrors the Rust ``Workspace::append``.
        """
        doc = self._docs.get(path)
        if doc:
            doc.content += content
            doc.updated_at = datetime.now(timezone.utc)
        else:
            self._docs[path] = WorkspaceDocument(
                path=path,
                content=content,
            )
        logger.debug("Workspace append: %s (+%d bytes)", path, len(content))

    # ------------------------------------------------------------------
    # Search
    # ------------------------------------------------------------------

    async def search(
        self,
        query: str,
        limit: int = 10,
    ) -> list[tuple[str, str, float]]:
        """
        Full-text search over workspace documents.

        Returns list of (path, snippet, score) tuples, highest score first.
        Production implementation should use RRF over FTS + pgvector.
        """
        query_lower = query.lower()
        results: list[tuple[str, str, float]] = []

        for path, doc in self._docs.items():
            content_lower = doc.content.lower()
            # Simple TF-style scoring: count query term occurrences
            score = content_lower.count(query_lower) + (1.0 if query_lower in path.lower() else 0)
            if score > 0:
                snippet = doc.content[:300]
                results.append((path, snippet, float(score)))

        results.sort(key=lambda x: x[2], reverse=True)
        return results[:limit]

    # ------------------------------------------------------------------
    # Tree listing
    # ------------------------------------------------------------------

    async def tree(self, prefix: str = "") -> list[str]:
        """List all document paths, optionally filtered by prefix."""
        paths = [p for p in self._docs if p.startswith(prefix)]
        return sorted(paths)

    # ------------------------------------------------------------------
    # Identity files (injected into system prompt)
    # ------------------------------------------------------------------

    async def load_identity_context(self) -> str:
        """
        Build the identity/context block from well-known identity files.

        Mirrors the Rust ``Workspace::identity_context()`` — reads
        AGENTS.md, SOUL.md, USER.md, IDENTITY.md, MEMORY.md and
        concatenates them into a string for the system prompt.
        """
        identity_paths = ["AGENTS.md", "SOUL.md", "USER.md", "IDENTITY.md", "MEMORY.md"]
        parts: list[str] = []

        for path in identity_paths:
            content = await self.read(path)
            if content:
                parts.append(f"## {path}\n\n{content}")

        return "\n\n".join(parts)
