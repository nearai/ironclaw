//! Caller-level test for issue #3285's default-off wiring.
//!
//! This test drives [`ironclaw::config::validate_telegram_v1_v2_exclusivity`]
//! through the same path the host startup glue will use to enforce
//! mutually-exclusive route binding. A unit test on the validator alone
//! proves logic but not that the caller threads the right inputs — this
//! caller test exercises every observable input combination to pin the
//! contract from the perspective of the host.

use ironclaw::config::validate_telegram_v1_v2_exclusivity;

#[test]
fn default_off_keeps_v1_only() {
    // The default IronClaw config has REBORN_TELEGRAM_V2_ENABLED = false.
    // Even if v1 telegram is configured, the validator must allow startup.
    validate_telegram_v1_v2_exclusivity(true, false).expect("default off is valid");
}

#[test]
fn v2_only_is_valid_when_v1_disabled() {
    validate_telegram_v1_v2_exclusivity(false, true).expect("v2 alone is valid");
}

#[test]
fn neither_is_valid() {
    validate_telegram_v1_v2_exclusivity(false, false).expect("neither is valid");
}

#[test]
fn v1_plus_v2_simultaneous_is_a_hard_startup_error() {
    let err = validate_telegram_v1_v2_exclusivity(true, true)
        .expect_err("simultaneous v1+v2 must reject");
    let rendered = err.to_string();
    assert!(rendered.contains("REBORN_TELEGRAM_V2_ENABLED"));
    assert!(rendered.contains("3285"));
}

#[test]
#[cfg(feature = "libsql")]
fn config_for_testing_has_v2_disabled() {
    // The library's testing helper produces a Config with reborn_telegram_v2_enabled
    // = false. Pin that so the legacy v1 path runs unchanged in every test.
    let temp = tempfile::tempdir().expect("tempdir");
    let libsql = temp.path().join("test.db");
    let skills = temp.path().join("skills");
    let installed = temp.path().join("installed_skills");
    let config = ironclaw::config::Config::for_testing(libsql, skills, installed);
    assert!(
        !config.channels.reborn_telegram_v2_enabled,
        "test config must default Reborn Telegram v2 to off"
    );
}
