use serde::{Deserialize, Serialize};

macro_rules! profile_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_profile_ref($kind, &value)?;
                Ok(Self(value))
            }

            #[allow(dead_code)]
            pub(crate) fn from_trusted_static(value: &'static str) -> Self {
                debug_assert!(validate_profile_ref($kind, value).is_ok());
                Self(value.to_string())
            }

            #[allow(dead_code)]
            pub(crate) fn from_trusted_string(value: String) -> Self {
                debug_assert!(validate_profile_ref($kind, &value).is_ok());
                Self(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

profile_ref!(RunClassId, "run_class_id");
profile_ref!(LoopDriverId, "loop_driver_id");
profile_ref!(CheckpointSchemaId, "checkpoint_schema_id");
profile_ref!(ModelProfileId, "model_profile_id");
profile_ref!(CapabilitySurfaceProfileId, "capability_surface_profile_id");
profile_ref!(ContextProfileId, "context_profile_id");
profile_ref!(RunnerPoolId, "runner_pool_id");
profile_ref!(SchedulingClass, "scheduling_class");
profile_ref!(ConcurrencyClass, "concurrency_class");
profile_ref!(ResourceBudgetTier, "resource_budget_tier");
profile_ref!(RunProfileFingerprint, "run_profile_fingerprint");
profile_ref!(RunProfileSourceLayer, "run_profile_source_layer");
profile_ref!(RunProfileSourceRef, "run_profile_source_ref");
profile_ref!(ProviderId, "provider_id");
profile_ref!(ModelId, "model_id");

fn validate_profile_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 128 {
        return Err(format!("{kind} must be at most 128 bytes"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' || c == ':')
    {
        return Err(format!(
            "{kind} must contain only lowercase ASCII letters, digits, _, -, or :"
        ));
    }
    Ok(())
}
