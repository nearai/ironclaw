//! Regression guard for the webui_v2 i18n locale dictionaries.
//!
//! Parses every `frontend/src/i18n/*.ts` pack and asserts cross-locale
//! consistency against the English source: identical key sets, no empty
//! values, and matching `{placeholder}` tokens. These invariants caught real
//! bugs during the i18n completion work — keys present in only some locales
//! (rendered as raw key strings), a Spanish string blanked to `""`, and
//! dropped interpolation placeholders. This test stops them from regressing.
//!
//! Pure file parsing — does not depend on the crate's `webui-v2-beta` API, so
//! it runs under the default feature set.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

fn i18n_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend/src/i18n")
}

/// Skip whitespace and `//` line comments starting at `i`.
fn skip_ws_comments(b: &[char], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == '/' && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        break;
    }
    i
}

/// Read a `"..."` or `'...'` string literal at `i`, honoring `\` escapes.
/// Returns the unescaped contents and the index just past the closing quote.
fn read_string(b: &[char], mut i: usize) -> Option<(String, usize)> {
    if i >= b.len() {
        return None;
    }
    let quote = b[i];
    if quote != '"' && quote != '\'' {
        return None;
    }
    i += 1;
    let mut out = String::new();
    while i < b.len() {
        let c = b[i];
        if c == '\\' && i + 1 < b.len() {
            let next = b[i + 1];
            out.push(match next {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            i += 2;
            continue;
        }
        if c == quote {
            return Some((out, i + 1));
        }
        out.push(c);
        i += 1;
    }
    None
}

/// Parse a `registerPack("lang", { "key": "value", ... })` module body into a
/// key→value map. Handles double/single-quoted values, multi-line entries,
/// escape sequences, and `//` line comments between entries.
fn parse_pack(src: &str) -> BTreeMap<String, String> {
    // Start after `registerPack` so the `import { registerPack }` brace and any
    // earlier `{` don't confuse the object-body scan.
    let tail = match src.find("registerPack") {
        Some(p) => &src[p..],
        None => src,
    };
    let b: Vec<char> = tail.chars().collect();
    let n = b.len();
    let mut map = BTreeMap::new();

    let mut i = 0;
    while i < n && b[i] != '{' {
        i += 1;
    }
    if i < n {
        i += 1; // skip opening '{'
    }

    loop {
        i = skip_ws_comments(&b, i);
        if i >= n || b[i] == '}' {
            break;
        }
        let Some((key, ni)) = read_string(&b, i) else {
            break;
        };
        i = skip_ws_comments(&b, ni);
        if i >= n || b[i] != ':' {
            break;
        }
        i = skip_ws_comments(&b, i + 1);
        let Some((val, ni)) = read_string(&b, i) else {
            break;
        };
        map.insert(key, val);
        i = skip_ws_comments(&b, ni);
        if i < n && b[i] == ',' {
            i += 1;
        }
    }
    map
}

/// Extract the set of `{name}` placeholder tokens from a translation value.
fn placeholders(s: &str) -> BTreeSet<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            let mut name = String::new();
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                name.push(chars[j]);
                j += 1;
            }
            if j < chars.len() && chars[j] == '}' && !name.is_empty() {
                out.insert(name);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn load_all() -> BTreeMap<String, BTreeMap<String, String>> {
    let dir = i18n_dir();
    let mut packs = BTreeMap::new();
    for entry in fs::read_dir(&dir).expect("read i18n dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let lang = path
            .file_stem()
            .expect("file stem")
            .to_string_lossy()
            .to_string();
        let src = fs::read_to_string(&path).expect("read locale file");
        packs.insert(lang, parse_pack(&src));
    }
    packs
}

fn en_pack(packs: &BTreeMap<String, BTreeMap<String, String>>) -> &BTreeMap<String, String> {
    packs.get("en").expect("en.ts locale present")
}

#[test]
fn en_pack_parses_and_is_substantial() {
    let packs = load_all();
    let en = en_pack(&packs);
    // Sanity check on the parser: en carries the full key set.
    assert!(
        en.len() > 700,
        "parsed only {} keys from en.ts — parser likely broke",
        en.len()
    );
    assert!(
        packs.len() >= 11,
        "expected >= 11 locales, found {}",
        packs.len()
    );
}

#[test]
fn all_locales_share_the_en_key_set() {
    let packs = load_all();
    let en_keys: BTreeSet<String> = en_pack(&packs).keys().cloned().collect();

    let mut problems = Vec::new();
    for (lang, pack) in &packs {
        if lang == "en" {
            continue;
        }
        let keys: BTreeSet<String> = pack.keys().cloned().collect();
        let missing: Vec<&String> = en_keys.difference(&keys).collect();
        let extra: Vec<&String> = keys.difference(&en_keys).collect();
        if !missing.is_empty() || !extra.is_empty() {
            problems.push(format!("{lang}: missing={missing:?} extra={extra:?}"));
        }
    }
    assert!(
        problems.is_empty(),
        "locale key-set drift vs en:\n{}",
        problems.join("\n")
    );
}

#[test]
fn all_locales_include_recent_automations_and_skills_keys() {
    let packs = load_all();
    let required_keys = [
        "automations.detail.currentRun",
        "automations.summary.running",
        "skills.contentLoadFailed",
    ];

    let mut problems = Vec::new();
    for (lang, pack) in &packs {
        for key in required_keys {
            if !pack.contains_key(key) {
                problems.push(format!("{lang}: missing \"{key}\""));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "locale packs are missing keys introduced by automations/skills UI:\n{}",
        problems.join("\n")
    );
}

#[test]
fn no_empty_values_where_en_is_nonempty() {
    let packs = load_all();
    let en = en_pack(&packs);

    let mut problems = Vec::new();
    for (lang, pack) in &packs {
        if lang == "en" {
            continue;
        }
        for (key, en_val) in en {
            if en_val.trim().is_empty() {
                continue;
            }
            if let Some(val) = pack.get(key)
                && val.trim().is_empty()
            {
                problems.push(format!("{lang}: \"{key}\" is empty"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "empty translations (would render blank in the UI):\n{}",
        problems.join("\n")
    );
}

#[test]
fn placeholders_match_en() {
    let packs = load_all();
    let en = en_pack(&packs);

    let mut problems = Vec::new();
    for (lang, pack) in &packs {
        if lang == "en" {
            continue;
        }
        for (key, en_val) in en {
            let want = placeholders(en_val);
            if want.is_empty() {
                continue;
            }
            if let Some(val) = pack.get(key) {
                let got = placeholders(val);
                if want != got {
                    problems.push(format!("{lang}: \"{key}\" want={want:?} got={got:?}"));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "placeholder token drift vs en (broken interpolation):\n{}",
        problems.join("\n")
    );
}
