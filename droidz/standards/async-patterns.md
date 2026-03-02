# Async and Concurrency Standard (AI Agents)

## Required

- Use async I/O with Tokio patterns already present in the codebase.
- Use `Arc<T>` for shared ownership across tasks.
- Use `RwLock` when concurrent reads with occasional writes are expected.
- Keep async functions non-blocking; move blocking work out of async paths.

## Concurrency hygiene

- Keep lock scope minimal.
- Avoid nested locks where possible.
- Prefer deterministic state transitions over implicit side effects.

## Task execution

- Follow existing scheduler/worker lifecycle patterns.
- Avoid spawning unmanaged background tasks without lifecycle ownership.
