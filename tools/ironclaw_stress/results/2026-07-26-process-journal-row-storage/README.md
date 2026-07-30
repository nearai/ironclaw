# Process journal row-storage rerun

This repeats the 2026-07-26 process-journal contention matrix after replacing
the growing `state.json` CAS document with append-only command rows at
`/processes/journal/records`.

Each matrix uses 8 owners, 16 threads per owner, 20 operations per worker, and
concurrency 8, 32, 64, and 100. `turn_thread_busy` is expected exclusive-thread
admission; there were no process-journal unavailable, CAS-exhaustion, backend
busy, or invalid-request failures in the final runs.

## Journal-isolated results

| Backend | c | succeeded / attempted | expected busy | journal p99 |
| --- | ---: | ---: | ---: | ---: |
| memory | 8 | 160 / 160 | 0 | 1.2 ms |
| memory | 32 | 320 / 640 | 320 | 4.6 ms |
| memory | 64 | 320 / 1280 | 960 | 9.1 ms |
| memory | 100 | 320 / 2000 | 1680 | 12.3 ms |
| LibSQL | 8 | 160 / 160 | 0 | 19.4 ms |
| LibSQL | 32 | 508 / 640 | 132 | 20.7 ms |
| LibSQL | 64 | 753 / 1280 | 527 | 22.3 ms |
| LibSQL | 100 | 814 / 2000 | 1186 | 29.2 ms |

## Chat-turn results

| Backend | c | succeeded / attempted | expected busy | journal p99 |
| --- | ---: | ---: | ---: | ---: |
| memory | 8 | 160 / 160 | 0 | 9.8 ms |
| memory | 32 | 489 / 640 | 151 | 0.3 ms |
| memory | 64 | 823 / 1280 | 457 | 0.5 ms |
| memory | 100 | 960 / 2000 | 1040 | 0.6 ms |
| LibSQL | 8 | 160 / 160 | 0 | 8.5 ms |
| LibSQL | 32 | 483 / 640 | 157 | 28.0 ms |
| LibSQL | 64 | 763 / 1280 | 517 | 34.0 ms |
| LibSQL | 100 | 942 / 2000 | 1058 | 82.2 ms |

## Result

The previous global CAS failure mode is gone through c100. libSQL physically
stores each command in its own `root_filesystem_events` row, while sequence
order preserves cross-handle exclusivity and transition invariants.

The remaining storage work is compaction/projection checkpointing for restart
cost and bounded terminal-history retention. It is no longer required for
write correctness or contention isolation.

Artifacts:

- `chatturn-memory-journal.jsonl`
- `chatturn-filesystem-journal.jsonl`
- `lifecycle-memory-journal.jsonl`
- `lifecycle-filesystem-journal.jsonl`
