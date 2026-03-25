use serde::Deserialize;

/// Sensitivity level for a memory layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerSensitivity {
    #[default]
    Private,
    Shared,
}

/// A named memory layer with read/write permissions and a scope.
///
/// Layers map to synthetic `user_id` values in the workspace tables.
/// The `scope` field is the user_id used for DB queries on this layer.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryLayer {
    pub name: String,
    pub scope: String,
    #[serde(default = "default_true")]
    pub writable: bool,
    #[serde(default)]
    pub sensitivity: LayerSensitivity,
}

fn default_true() -> bool {
    true
}

impl MemoryLayer {
    /// Build the default layer set: a single private layer for the given user_id.
    pub fn default_for_user(user_id: &str) -> Vec<MemoryLayer> {
        vec![MemoryLayer {
            name: "private".to_string(),
            scope: user_id.to_string(),
            writable: true,
            sensitivity: LayerSensitivity::Private,
        }]
    }

    /// Extract read scopes (all layer scope values).
    pub fn read_scopes(layers: &[MemoryLayer]) -> Vec<String> {
        layers.iter().map(|l| l.scope.clone()).collect()
    }

    /// Extract writable scopes only.
    pub fn writable_scopes(layers: &[MemoryLayer]) -> Vec<String> {
        layers
            .iter()
            .filter(|l| l.writable)
            .map(|l| l.scope.clone())
            .collect()
    }

    /// Find a layer by name. Returns None if not found.
    pub fn find<'a>(layers: &'a [MemoryLayer], name: &str) -> Option<&'a MemoryLayer> {
        layers.iter().find(|l| l.name == name)
    }

    /// Find the private layer (first layer with Private sensitivity).
    pub fn private_layer(layers: &[MemoryLayer]) -> Option<&MemoryLayer> {
        layers
            .iter()
            .find(|l| l.sensitivity == LayerSensitivity::Private)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_for_user_creates_single_private_layer() {
        let layers = MemoryLayer::default_for_user("alice");
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].name, "private");
        assert_eq!(layers[0].scope, "alice");
        assert!(layers[0].writable);
        assert_eq!(layers[0].sensitivity, LayerSensitivity::Private);
    }

    #[test]
    fn read_scopes_collects_all() {
        let layers = vec![
            MemoryLayer {
                name: "private".into(),
                scope: "alice".into(),
                writable: true,
                sensitivity: LayerSensitivity::Private,
            },
            MemoryLayer {
                name: "shared".into(),
                scope: "shared".into(),
                writable: true,
                sensitivity: LayerSensitivity::Shared,
            },
            MemoryLayer {
                name: "reports".into(),
                scope: "reports".into(),
                writable: false,
                sensitivity: LayerSensitivity::Shared,
            },
        ];
        let scopes = MemoryLayer::read_scopes(&layers);
        assert_eq!(scopes, vec!["alice", "shared", "reports"]);
    }

    #[test]
    fn writable_scopes_filters_read_only() {
        let layers = vec![
            MemoryLayer {
                name: "private".into(),
                scope: "alice".into(),
                writable: true,
                sensitivity: LayerSensitivity::Private,
            },
            MemoryLayer {
                name: "reports".into(),
                scope: "reports".into(),
                writable: false,
                sensitivity: LayerSensitivity::Shared,
            },
        ];
        let scopes = MemoryLayer::writable_scopes(&layers);
        assert_eq!(scopes, vec!["alice"]);
    }

    #[test]
    fn find_returns_matching_layer() {
        let layers = MemoryLayer::default_for_user("alice");
        assert!(MemoryLayer::find(&layers, "private").is_some());
        assert!(MemoryLayer::find(&layers, "shared").is_none());
    }

    #[test]
    fn deserialize_from_json() {
        let json = serde_json::json!({
            "name": "shared",
            "scope": "shared",
            "writable": true,
            "sensitivity": "shared"
        });
        let layer: MemoryLayer = serde_json::from_value(json).unwrap();
        assert_eq!(layer.name, "shared");
        assert_eq!(layer.sensitivity, LayerSensitivity::Shared);
    }

    #[test]
    fn deserialize_defaults() {
        let json = serde_json::json!({
            "name": "private",
            "scope": "alice"
        });
        let layer: MemoryLayer = serde_json::from_value(json).unwrap();
        assert!(layer.writable); // default true
        assert_eq!(layer.sensitivity, LayerSensitivity::Private); // default
    }
}
