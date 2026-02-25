//! Config round-trip tests (QA Plan item 1.2).
//!
//! Tests the full config lifecycle: write via bootstrap helpers, read back via
//! dotenvy, and assert values match. Each test uses a tempdir for isolation.
//!
//! The bootstrap `.env` format uses double-quoted values with backslash-escaping
//! for quotes and backslashes. dotenvy strips the quotes on read, giving back
//! the original value.
//!
//! Note: `save_bootstrap_env` and `upsert_bootstrap_var` write to the global
//! `~/.ironclaw/.env` path. These tests exercise the same escaping/formatting
//! logic against tempdir paths so they remain hermetic. The format contract is:
//! `KEY="escaped_value"\n` per line, parseable by dotenvy.

use std::collections::HashMap;
use tempfile::tempdir;

/// Write a set of key-value pairs to a .env file using the same format as
/// `save_bootstrap_env`: double-quoted values with escaped backslashes and
/// double quotes.
fn write_bootstrap_env(
    path: &std::path::Path,
    vars: &[(&str, &str)],
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = String::new();
    for (key, value) in vars {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        content.push_str(&format!("{}=\"{}\"\n", key, escaped));
    }
    std::fs::write(path, &content)
}

/// Simulate `upsert_bootstrap_var` against a specific path: read existing
/// content, replace or append the key, write back.
fn upsert_env_var(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let new_line = format!("{}=\"{}\"", key, escaped);
    let prefix = format!("{}=", key);

    let existing = std::fs::read_to_string(path).unwrap_or_default();

    let mut found = false;
    let mut result = String::new();
    for line in existing.lines() {
        if line.starts_with(&prefix) {
            if !found {
                result.push_str(&new_line);
                result.push('\n');
                found = true;
            }
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    if !found {
        result.push_str(&new_line);
        result.push('\n');
    }

    std::fs::write(path, result)
}

/// Parse a .env file into a HashMap using dotenvy.
fn read_env_map(path: &std::path::Path) -> HashMap<String, String> {
    dotenvy::from_path_iter(path)
        .expect("dotenvy should parse the .env file")
        .filter_map(|r| r.ok())
        .collect()
}

// ── Test 1: LLM_BACKEND round-trips ────────────────────────────────────────

#[test]
fn bootstrap_env_round_trips_llm_backend() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    // Write: same vars the wizard writes when user picks an LLM backend
    write_bootstrap_env(
        &env_path,
        &[
            ("DATABASE_BACKEND", "libsql"),
            ("LLM_BACKEND", "openai"),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    // Read back
    let map = read_env_map(&env_path);

    assert_eq!(
        map.get("LLM_BACKEND").map(String::as_str),
        Some("openai"),
        "LLM_BACKEND must survive .env round-trip"
    );

    // All other backends the wizard supports
    for backend in &["nearai", "anthropic", "ollama", "openai_compatible", "tinfoil"] {
        write_bootstrap_env(&env_path, &[("LLM_BACKEND", backend)]).unwrap();
        let map = read_env_map(&env_path);
        assert_eq!(
            map.get("LLM_BACKEND").map(String::as_str),
            Some(*backend),
            "LLM_BACKEND={backend} must survive round-trip"
        );
    }
}

// ── Test 2: EMBEDDING_ENABLED=false survives even with OPENAI_API_KEY ──────

#[test]
fn bootstrap_env_round_trips_embedding_disabled() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    // Write both EMBEDDING_ENABLED=false and OPENAI_API_KEY (a user who has
    // an OpenAI key but explicitly disabled embeddings)
    write_bootstrap_env(
        &env_path,
        &[
            ("DATABASE_BACKEND", "libsql"),
            ("EMBEDDING_ENABLED", "false"),
            ("OPENAI_API_KEY", "sk-test-key-1234567890"),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    let map = read_env_map(&env_path);

    assert_eq!(
        map.get("EMBEDDING_ENABLED").map(String::as_str),
        Some("false"),
        "EMBEDDING_ENABLED=false must not be lost when OPENAI_API_KEY is also present"
    );
    assert_eq!(
        map.get("OPENAI_API_KEY").map(String::as_str),
        Some("sk-test-key-1234567890"),
        "OPENAI_API_KEY must be preserved alongside EMBEDDING_ENABLED"
    );
}

// ── Test 3: ONBOARD_COMPLETED round-trips and check_onboard_needed logic ───

#[test]
fn bootstrap_env_round_trips_onboard_completed() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    write_bootstrap_env(
        &env_path,
        &[
            ("DATABASE_BACKEND", "libsql"),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    let map = read_env_map(&env_path);

    // Verify the value round-trips
    assert_eq!(
        map.get("ONBOARD_COMPLETED").map(String::as_str),
        Some("true"),
        "ONBOARD_COMPLETED=true must survive .env round-trip"
    );

    // Verify the same logic check_onboard_needed() uses: the string must be
    // exactly "true" (not "1", not "TRUE") for the check to pass.
    let onboard_val = map.get("ONBOARD_COMPLETED").unwrap();
    let onboard_completed = onboard_val == "true";
    assert!(
        onboard_completed,
        "Parsed ONBOARD_COMPLETED must satisfy check_onboard_needed() logic (== \"true\")"
    );

    // Also verify that without ONBOARD_COMPLETED, the flag is absent
    write_bootstrap_env(&env_path, &[("DATABASE_BACKEND", "libsql")]).unwrap();
    let map2 = read_env_map(&env_path);
    assert!(
        !map2.contains_key("ONBOARD_COMPLETED"),
        "ONBOARD_COMPLETED must be absent when not written"
    );
}

// ── Test 4: Session token key name round-trips ─────────────────────────────

#[test]
fn bootstrap_env_round_trips_session_token_key() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    // The session manager writes NEARAI_API_KEY via upsert_bootstrap_var
    // (see src/llm/session.rs:424). Verify the key name is correct and
    // the value (which looks like a session token or API key) round-trips.
    let token = "sess_abc123def456ghi789jkl012mno345pqr678stu901vwx234";
    write_bootstrap_env(
        &env_path,
        &[
            ("DATABASE_BACKEND", "libsql"),
            ("NEARAI_API_KEY", token),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    let map = read_env_map(&env_path);

    assert_eq!(
        map.get("NEARAI_API_KEY").map(String::as_str),
        Some(token),
        "NEARAI_API_KEY (session token) must survive .env round-trip"
    );

    // Also test the NEARAI_SESSION_TOKEN key (used by hosting providers)
    let session_token = "sess_hosting_provider_injected_token_value";
    write_bootstrap_env(
        &env_path,
        &[
            ("NEARAI_SESSION_TOKEN", session_token),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    let map2 = read_env_map(&env_path);
    assert_eq!(
        map2.get("NEARAI_SESSION_TOKEN").map(String::as_str),
        Some(session_token),
        "NEARAI_SESSION_TOKEN must survive .env round-trip"
    );
}

// ── Test 5: Multiple keys are preserved on re-read ─────────────────────────

#[test]
fn bootstrap_env_preserves_existing_values() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    // Write the full set of vars the wizard might produce
    let initial_vars: &[(&str, &str)] = &[
        ("DATABASE_BACKEND", "postgres"),
        ("DATABASE_URL", "postgres://user:pass@localhost:5432/ironclaw"),
        ("LLM_BACKEND", "nearai"),
        ("NEARAI_API_KEY", "key_abc123"),
        ("EMBEDDING_ENABLED", "true"),
        ("ONBOARD_COMPLETED", "true"),
    ];
    write_bootstrap_env(&env_path, initial_vars).unwrap();

    let map = read_env_map(&env_path);

    // All keys must be present with correct values
    assert_eq!(map.len(), initial_vars.len(), "all vars must survive round-trip");
    for (key, value) in initial_vars {
        assert_eq!(
            map.get(*key).map(String::as_str),
            Some(*value),
            "{key} must be preserved"
        );
    }

    // Now upsert a new key and verify nothing is lost
    upsert_env_var(&env_path, "LLM_MODEL", "gpt-4o").unwrap();

    let map2 = read_env_map(&env_path);

    // Original keys must still be there
    for (key, value) in initial_vars {
        assert_eq!(
            map2.get(*key).map(String::as_str),
            Some(*value),
            "{key} must be preserved after upsert"
        );
    }
    // New key must also be present
    assert_eq!(
        map2.get("LLM_MODEL").map(String::as_str),
        Some("gpt-4o"),
        "upserted LLM_MODEL must be present"
    );

    // Upsert an existing key and verify the value is updated, others preserved
    upsert_env_var(&env_path, "LLM_BACKEND", "anthropic").unwrap();

    let map3 = read_env_map(&env_path);

    assert_eq!(
        map3.get("LLM_BACKEND").map(String::as_str),
        Some("anthropic"),
        "LLM_BACKEND must be updated after upsert"
    );
    assert_eq!(
        map3.get("DATABASE_URL").map(String::as_str),
        Some("postgres://user:pass@localhost:5432/ironclaw"),
        "DATABASE_URL must be preserved after upsert of different key"
    );
    assert_eq!(
        map3.get("LLM_MODEL").map(String::as_str),
        Some("gpt-4o"),
        "previously upserted LLM_MODEL must be preserved"
    );
}

// ── Test 6: Special characters in values ───────────────────────────────────

#[test]
fn bootstrap_env_handles_special_characters() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");

    let test_cases: &[(&str, &str)] = &[
        // Spaces in values
        ("AGENT_NAME", "my ironclaw agent"),
        // Equals signs in values (e.g., base64 tokens)
        ("API_TOKEN", "dGVzdA=="),
        // Hash characters (common in URL-encoded passwords, treated as comments without quoting)
        ("DATABASE_URL", "postgres://user:p%23assword@host:5432/db"),
        // Single quotes inside double-quoted values
        ("GREETING", "it's a test"),
        // Double quotes (must be escaped)
        ("QUOTED_VAL", r#"say "hello" world"#),
        // Backslashes (must be escaped)
        ("WIN_PATH", r"C:\Users\ironclaw\data"),
        // Mixed special characters
        ("COMPLEX", r#"key=val with "quotes" & back\slash #hash"#),
        // Empty-ish but non-empty value (single space)
        ("SPACER", " "),
    ];

    write_bootstrap_env(&env_path, test_cases).unwrap();

    let map = read_env_map(&env_path);

    for (key, expected) in test_cases {
        let actual = map.get(*key);
        assert!(
            actual.is_some(),
            "{key} must be present in parsed .env"
        );
        assert_eq!(
            actual.unwrap(),
            expected,
            "{key}: value with special characters must round-trip exactly"
        );
    }
}
