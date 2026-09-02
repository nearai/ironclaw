// The one comparator for opaque decimal run-completion sequences (u64 on the
// wire, so string length orders before lexical order). Dependency-free so the
// eager badge cache and the lazily imported protocol module share it without
// pulling either into the other's bundle graph.
export function compareSequences(a: string, b: string): number {
  if (a.length !== b.length) return a.length - b.length;
  return a < b ? -1 : a > b ? 1 : 0;
}
