#!/usr/bin/env python3
"""probe_core -- deterministic perturbation of LFD dev case inputs.

Generates a value-isomorphism over the visible dev cases of a feature:
  (a) entity renames  -- user/channel/team ids, email locals, names found in
      setup/inbound, mapped via a stable seeded map (seed = feature name),
  (b) date shifts     -- +13 days on ISO dates (YYYY-MM-DD, incl. inside
      ISO-8601 timestamps),
  (c) numeric id shuffles -- digit runs >= 6 remapped to same-length numbers.

The SAME map is applied consistently across setup / llm_script / inbound /
state_queries[].params. Answers are NOT touched here; `score_core --probe`
applies the map to the sealed contracts internally so probes stay blinded.

Everything is deterministic: the RNG is seeded from the feature name only.
Stdlib only. See lfd/_shared/SCHEMA.md.
"""

import argparse
import datetime
import hashlib
import json
import random
import re
import sys
from pathlib import Path

DATE_SHIFT_DAYS = 13

DATE_RE = re.compile(r"\b(\d{4})-(\d{2})-(\d{2})\b")
# Slack-style entity ids: U/C/T/W/D/G prefix + >=3 uppercase alnum incl a digit.
ENTITY_ID_RE = re.compile(r"\b([UCTWDG][0-9A-Z]{3,12})\b")
EMAIL_RE = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
NUMERIC_ID_RE = re.compile(r"\b\d{6,}\b")

NAME_KEYS = frozenset(
    [
        "name",
        "first_name",
        "last_name",
        "display_name",
        "real_name",
        "username",
        "user_name",
        "full_name",
    ]
)
NAME_POOL = [
    "Vexley",
    "Ordano",
    "Miravel",
    "Tessang",
    "Quorrin",
    "Halbrek",
    "Nystrom",
    "Fenwick",
    "Ilmara",
    "Sorvad",
    "Betrik",
    "Ozmun",
]
ID_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTUVWXYZ"

# Contract keys that are structural references, never entity values.
CONTRACT_SKIP_KEYS = frozenset(["id", "type", "query", "weight"])


# ---------------------------------------------------------------------------
# Map application (shared with score_core --probe; MUST stay the single
# implementation so cases and contracts are transformed identically).
# ---------------------------------------------------------------------------


def _shift_date_match(m, days):
    try:
        d = datetime.date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    except ValueError:
        return m.group(0)
    return (d + datetime.timedelta(days=days)).isoformat()


def shift_iso_dates(s, days):
    if not days:
        return s
    return DATE_RE.sub(lambda m: _shift_date_match(m, days), s)


def compile_rename_regex(renames):
    if not renames:
        return None
    keys = sorted(renames, key=len, reverse=True)
    alt = "|".join(re.escape(k) for k in keys)
    return re.compile(r"(?<![A-Za-z0-9])(?:" + alt + r")(?![A-Za-z0-9])")


def apply_map_to_string(s, rename_re, renames, days):
    s = shift_iso_dates(s, days)
    if rename_re is not None:
        s = rename_re.sub(lambda m: renames[m.group(0)], s)
    return s


def apply_map_to_value(value, rename_re, renames, days):
    """Recursively apply the map to a JSON value (strings + int ids)."""
    if isinstance(value, str):
        return apply_map_to_string(value, rename_re, renames, days)
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        s = str(value)
        if s in renames:
            try:
                return int(renames[s])
            except ValueError:
                return renames[s]
        return value
    if isinstance(value, list):
        return [apply_map_to_value(v, rename_re, renames, days) for v in value]
    if isinstance(value, dict):
        return {
            k: apply_map_to_value(v, rename_re, renames, days)
            for k, v in value.items()
        }
    return value


def transform_contracts(answers, map_data):
    """Apply a probe map to sealed contracts (used by score_core --probe).

    Matcher structural keys (id/type/query/weight) are preserved; every other
    matcher value is transformed with exactly the same function used on case
    inputs. `ordered` lists reference matcher ids and are left untouched.
    """
    renames = dict(map_data.get("renames") or {})
    days = int(map_data.get("date_shift_days") or 0)
    rename_re = compile_rename_regex(renames)
    out = json.loads(json.dumps(answers))
    for contract in out.get("contracts") or []:
        for key in ("required", "forbidden"):
            matchers = contract.get(key) or []
            new_matchers = []
            for m in matchers:
                nm = {}
                for k, v in m.items():
                    if k in CONTRACT_SKIP_KEYS:
                        nm[k] = v
                    else:
                        nm[k] = apply_map_to_value(v, rename_re, renames, days)
                new_matchers.append(nm)
            if matchers:
                contract[key] = new_matchers
    return out


# ---------------------------------------------------------------------------
# Map construction (extraction from setup/inbound)
# ---------------------------------------------------------------------------


def _walk_kv(value, key=None):
    """Yield (key, string_or_int) pairs from a JSON tree, values only."""
    if isinstance(value, dict):
        for k, v in value.items():
            for pair in _walk_kv(v, k):
                yield pair
    elif isinstance(value, list):
        for v in value:
            for pair in _walk_kv(v, key):
                yield pair
    elif isinstance(value, (str, int)) and not isinstance(value, bool):
        yield (key, value)


def _extract_tokens(cases):
    """token -> class ('entity' | 'email' | 'name' | 'numeric' | 'opaque')."""
    tokens = {}

    def note(tok, cls):
        # First classification wins; iteration order is deterministic because
        # cases are traversed in sorted order and dicts preserve insertion.
        if tok and tok not in tokens:
            tokens[tok] = cls

    for case in cases:
        sources = [case.get("setup"), case.get("inbound")]
        for src in sources:
            if src is None:
                continue
            for key, val in _walk_kv(src):
                if isinstance(val, int):
                    if key and str(key).endswith("_id") and len(str(val)) >= 6:
                        note(str(val), "numeric")
                    continue
                if key in NAME_KEYS and len(val) >= 2:
                    note(val, "name")
                elif key and str(key).endswith("_id") and val:
                    if val.isdigit():
                        note(val, "numeric" if len(val) >= 2 else "numeric")
                    elif ENTITY_ID_RE.fullmatch(val):
                        note(val, "entity")
                    else:
                        note(val, "opaque")
                for m in ENTITY_ID_RE.finditer(val):
                    tok = m.group(1)
                    if any(c.isdigit() for c in tok[1:]):
                        note(tok, "entity")
                for m in EMAIL_RE.finditer(val):
                    note(m.group(0), "email")
                for m in NUMERIC_ID_RE.finditer(val):
                    note(m.group(0), "numeric")
    return tokens


def _gen_candidate(tok, cls, rng):
    if cls == "entity":
        body = [rng.choice(ID_ALPHABET) for _ in range(max(3, len(tok) - 1))]
        # force at least one digit so the token keeps its id-like shape
        body[rng.randrange(len(body))] = rng.choice("0123456789")
        return tok[0] + "".join(body)
    if cls == "email":
        local, _, domain = tok.partition("@")
        new_local = "p" + "".join(
            rng.choice("abcdefghijkmnopqrstuvwxyz") for _ in range(7)
        )
        return new_local + "@" + domain
    if cls == "name":
        words = tok.split()
        return " ".join(rng.choice(NAME_POOL) for _ in words)
    if cls == "numeric":
        first = rng.choice("123456789")
        rest = "".join(rng.choice("0123456789") for _ in range(len(tok) - 1))
        return first + rest
    # opaque string id: same length, lowercase hex-ish
    return "x" + "".join(
        rng.choice("0123456789abcdef") for _ in range(max(3, len(tok) - 1))
    )


def _fallback_candidate(tok, cls, salt):
    digest = hashlib.sha256(("%s:%s" % (tok, salt)).encode("utf-8")).hexdigest()
    if cls == "entity":
        body = list(digest[: max(3, len(tok) - 1)].upper())
        body[0] = digest[-1]
        return tok[0] + "".join(body)
    if cls == "email":
        _, _, domain = tok.partition("@")
        return "p%s@%s" % (digest[:10], domain)
    if cls == "name":
        words = tok.split() or [tok]
        return " ".join("Probe%s" % digest[i * 4 : i * 4 + 4] for i, _ in enumerate(words))
    if cls == "numeric":
        first = str((int(digest[0], 16) % 9) + 1)
        rest = "".join(str(int(ch, 16) % 10) for ch in digest[1 : len(tok)])
        return first + rest
    return "x" + digest[: max(3, len(tok) - 1)]


def build_map(feature, cases):
    """Build {"renames": {...}, "date_shift_days": 13} deterministically."""
    seed = int.from_bytes(hashlib.sha256(feature.encode("utf-8")).digest()[:8], "big")
    rng = random.Random(seed)
    tokens = _extract_tokens(cases)
    corpus = json.dumps(cases, sort_keys=True)
    renames = {}
    used = set()
    for tok in sorted(tokens):
        cls = tokens[tok]
        cand = None
        for _ in range(1000):
            c = _gen_candidate(tok, cls, rng)
            if c != tok and c not in used and c not in tokens and c not in corpus:
                cand = c
                break
        if cand is None:
            for salt in range(100):
                c = _fallback_candidate(tok, cls, salt)
                if c != tok and c not in used and c not in tokens and c not in corpus:
                    cand = c
                    break
        if cand is None:
            raise RuntimeError("probe_core: could not generate unique rename for %r" % tok)
        renames[tok] = cand
        used.add(cand)
    return {"renames": renames, "date_shift_days": DATE_SHIFT_DAYS}


def perturb_case(case, map_data):
    renames = dict(map_data.get("renames") or {})
    days = int(map_data.get("date_shift_days") or 0)
    rename_re = compile_rename_regex(renames)
    out = json.loads(json.dumps(case))
    for field in ("setup", "llm_script", "inbound"):
        if field in out:
            out[field] = apply_map_to_value(out[field], rename_re, renames, days)
    if "state_queries" in out and isinstance(out["state_queries"], list):
        new_queries = []
        for q in out["state_queries"]:
            if isinstance(q, dict) and "params" in q:
                q = dict(q)
                q["params"] = apply_map_to_value(q["params"], rename_re, renames, days)
            new_queries.append(q)
        out["state_queries"] = new_queries
    return out


# ---------------------------------------------------------------------------
# Case loading (shared helper)
# ---------------------------------------------------------------------------


def load_cases(cases_dir):
    """Load case JSONs from a directory, sorted by filename. Returns
    [(filename, case_dict)]."""
    cases_dir = Path(cases_dir)
    if not cases_dir.is_dir():
        raise FileNotFoundError("cases directory not found: %s" % cases_dir)
    result = []
    for p in sorted(cases_dir.glob("*.json")):
        with open(p, "r", encoding="utf-8") as f:
            result.append((p.name, json.load(f)))
    return result


def main(argv=None):
    ap = argparse.ArgumentParser(description="LFD deterministic case perturbation")
    ap.add_argument("--feature", required=True)
    ap.add_argument("--lfd-root", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)

    cases_dir = Path(args.lfd_root) / args.feature / "eval" / "dev" / "cases"
    loaded = load_cases(cases_dir)
    map_data = build_map(args.feature, [c for _, c in loaded])

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    # Default lane wrappers write cases to eval/probe/cases. Keep that
    # directory case-only so the Rust runner can consume it directly.
    map_path = out_dir.parent / "map.json" if out_dir.name == "cases" else out_dir / "map.json"
    for fname, case in loaded:
        perturbed = perturb_case(case, map_data)
        with open(out_dir / fname, "w", encoding="utf-8") as f:
            json.dump(perturbed, f, indent=2, sort_keys=True)
            f.write("\n")
    with open(map_path, "w", encoding="utf-8") as f:
        json.dump(map_data, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "probe: wrote %d perturbed case(s) to %s and map.json to %s"
        % (len(loaded), out_dir, map_path)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
