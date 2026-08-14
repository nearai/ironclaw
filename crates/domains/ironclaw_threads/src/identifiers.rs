use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable canonical transcript message identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadMessageId(Uuid);

impl ThreadMessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    pub(crate) fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for ThreadMessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ThreadMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Stable summary artifact identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SummaryArtifactId(Uuid);

impl SummaryArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SummaryArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SummaryArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
