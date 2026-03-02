//! Config round-trip tests (QA Plan item 1.2).
//!
//! Tests the full config lifecycle: write via bootstrap helpers, read back via
//! dotenvy, and assert values match. Each test uses a tempdir for isolation.
//!
//! These tests call the real `save_bootstrap_env_to` and `upsert_bootstrap_var_to`
//! functions from `ironclaw::bootstrap`, ensuring test coverage of the actual
//! escaping/formatting logic rather than a reimplementation.

use std::collections::HashMap;
use tempfile::tempdir;

use ironclaw::bootstrap::{save_bootstrap_env_to, upsert_bootstrap_var_to};

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
    save_bootstrap_env_to(
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
    for backend in &[
        "nearai",
        "anthropic",
        "ollama",
        "openai_compatible",
        "tinfoil",
    ] {
        save_bootstrap_env_to(&env_path, &[("LLM_BACKEND", backend)]).unwrap();
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

    save_bootstrap_env_to(
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

    save_bootstrap_env_to(
        &env_path,
        &[
            ("DATABASE_BACKEND", "libsql"),
            ("ONBOARD_COMPLETED", "true"),
        ],
    )
    .unwrap();

    let map = read_env_map(&env_path);

    assert_eq!(
        map.get("ONBOARD_COMPLETED").map(String::as_str),
        Some("true"),
        "ONBOARD_COMPLETED=true must survive .env round-trip"
    );

    let onboard_val = map.get("ONBOARD_COMPLETED").unwrap();
    let onboard_completed = onboard_val == "true";
    assert!(
        onboard_completed,
        "Parsed ONBOARD_COMPLETED must satisfy check_onboard_needed() logic (== \"true\")"
    );

    // Also verify that without ONBOARD_COMPLETED, the flag is absent
    save_bootstrap_env_to(&env_path, &[("DATABASE_BACKEND", "libsql")]).unwrap();
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

    let token = "sess_abc123def456ghi789jkl012mno345pqr678stu901vwx234";
    save_bootstrap_env_to(
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

    let session_token = "sess_hosting_provider_injected_token_value";
    save_bootstrap_env_to(
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

    let initial_vars: &[(&str, &str)] = &[
        ("DATABASE_BACKEND", "postgres"),
        (
            "DATABASE_URL",
            "postgres://user:pass@localhost:5432/ironclaw",
        ),
        ("LLM_BACKEND", "nearai"),
        ("NEARAI_API_KEY", "key_abc123"),
        ("EMBEDDING_ENABLED", "true"),
        ("ONBOARD_COMPLETED", "true"),
    ];
    save_bootstrap_env_to(&env_path, initial_vars).unwrap();

    let map = read_env_map(&env_path);

    assert_eq!(
        map.len(),
        initial_vars.len(),
        "all vars must survive round-trip"
    );
    for (key, value) in initial_vars {
        assert_eq!(
            map.get(*key).map(String::as_str),
            Some(*value),
            "{key} must be preserved"
        );
    }

    // Now upsert a new key and verify nothing is lost
    upsert_bootstrap_var_to(&env_path, "LLM_MODEL", "gpt-4o").unwrap();

    let map2 = read_env_map(&env_path);

    for (key, value) in initial_vars {
        assert_eq!(
            map2.get(*key).map(String::as_str),
            Some(*value),
            "{key} must be preserved after upsert"
        );
    }
    assert_eq!(
        map2.get("LLM_MODEL").map(String::as_str),
        Some("gpt-4o"),
        "upserted LLM_MODEL must be present"
    );

    // Upsert an existing key and verify the value is updated, others preserved
    upsert_bootstrap_var_to(&env_path, "LLM_BACKEND", "anthropic").unwrap();

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

    save_bootstrap_env_to(&env_path, test_cases).unwrap();

    let map = read_env_map(&env_path);

    for (key, expected) in test_cases {
        let actual = map.get(*key);
        assert!(actual.is_some(), "{key} must be present in parsed .env");
        assert_eq!(
            actual.unwrap(),
            expected,
            "{key}: value with special characters must round-trip exactly"
        );
    }
}
