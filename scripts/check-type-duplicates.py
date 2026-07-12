#!/usr/bin/env python3
"""Detect cross-crate SEMANTIC duplicate-type candidates by field/variant signature.

Name matching misses the real failure mode (same DTO job, different names), so
this compares (field-name, field-type) sets for structs and variant sets for
enums across crates and reports pairs above a similarity threshold.

Output is CANDIDATES, not verdicts: a match is either a true duplicate (unify
into its owner), a justified mirror (independent wire/domain evolution), or a
coincidental shape. Judge each pair by reading the definitions before acting.
See .claude/rules/type-placement.md; judged backlog (2026-07):
docs/plans/2026-07-02-type-dedup-backlog.md.

Usage: python3 scripts/check-type-duplicates.py [--jaccard 0.6] [--min-items 3]
"""
import argparse
import itertools
import re
from pathlib import Path

DEF_RE = re.compile(r'^pub (struct|enum) ([A-Za-z0-9_]+)(?:<[^>]*>)?\s*\{', re.M)
FIELD_RE = re.compile(r'(?:pub(?:\([^)]*\))?\s+)?([a-z_][a-z0-9_]*)\s*:\s*([^,\n]+)')
VARIANT_RE = re.compile(r'^\s{4}([A-Z][A-Za-z0-9_]*)', re.M)
KEYWORDS = {'where', 'if', 'let', 'match'}


def body_of(text, start):
    i = text.index('{', start)
    depth, j = 0, i
    while j < len(text):
        if text[j] == '{':
            depth += 1
        elif text[j] == '}':
            depth -= 1
            if depth == 0:
                break
        j += 1
    return text[i + 1:j]


def collect(min_items):
    types = []
    for src in Path('crates').glob('*/src'):
        crate = src.parent.name.removeprefix('ironclaw_')
        for f in src.rglob('*.rs'):
            try:
                text = f.read_text(errors='ignore')
            except OSError:
                continue
            for m in DEF_RE.finditer(text):
                kind, name = m.group(1), m.group(2)
                body = body_of(text, m.start())
                if kind == 'struct':
                    items = frozenset(
                        (fm.group(1), re.sub(r'\s+', '', fm.group(2)).rstrip(','))
                        for fm in FIELD_RE.finditer(body)
                        if fm.group(1) not in KEYWORDS)
                else:
                    items = frozenset((vm.group(1), '') for vm in VARIANT_RE.finditer(body))
                if len(items) >= min_items:
                    types.append((crate, kind, name, items, f))
    return types


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--jaccard', type=float, default=0.6)
    ap.add_argument('--min-items', type=int, default=3)
    args = ap.parse_args()

    types = collect(args.min_items)
    found = 0
    for a, b in itertools.combinations(types, 2):
        if a[0] == b[0] or a[1] != b[1]:
            continue
        i1, i2 = a[3], b[3]
        union = len(i1 | i2)
        jac = len(i1 & i2) / union if union else 0
        n1 = frozenset(x[0] for x in i1)
        n2 = frozenset(x[0] for x in i2)
        njac = len(n1 & n2) / len(n1 | n2) if (n1 | n2) else 0
        if jac >= args.jaccard or (njac >= 0.75 and min(len(n1), len(n2)) >= 4):
            found += 1
            print(f"jac={jac:.2f} njac={njac:.2f} {a[1]:6} "
                  f"{a[0]}::{a[2]}  <->  {b[0]}::{b[2]}")
    print(f"\n{found} candidate pair(s) from {len(types)} types "
          f"(>= {args.min_items} items). Judge before acting.")


if __name__ == '__main__':
    main()
