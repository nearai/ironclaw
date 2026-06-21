pub(crate) const IRONCLAW_LEARNING_ENABLED_ENV: &str = "IRONCLAW_LEARNING_ENABLED";
pub(crate) const LEARNING_FIELD_NAMES: [&str; 5] =
    ["key", "category", "confidence", "created_at", "source"];

pub(crate) fn learning_enabled() -> bool {
    std::env::var(IRONCLAW_LEARNING_ENABLED_ENV)
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true"))
}
