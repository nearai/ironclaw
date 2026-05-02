//! Capability registry — stores capability definitions available to the system.

use std::collections::HashMap;

use crate::types::capability::{ActionDef, Capability};

/// Registry of all known capabilities.
///
/// Capabilities are registered at startup (from extensions, built-in tools,
/// etc.) and queried when granting leases or resolving action names.
#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Capability>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a capability. Overwrites any existing capability with the same name.
    pub fn register(&mut self, capability: Capability) {
        self.capabilities
            .insert(capability.name.clone(), capability);
    }

    /// Look up a capability by name.
    pub fn get(&self, name: &str) -> Option<&Capability> {
        self.capabilities.get(name)
    }

    /// List all registered capabilities.
    pub fn list(&self) -> Vec<&Capability> {
        self.capabilities.values().collect()
    }

    /// Look up a specific action across all capabilities.
    ///
    /// Returns `(capability_name, action_def)` if found.
    pub fn find_action(&self, action_name: &str) -> Option<(&str, &ActionDef)> {
        for cap in self.capabilities.values() {
            if let Some(action) = cap.actions.iter().find(|a| a.name == action_name) {
                return Some((&cap.name, action));
            }
        }
        None
    }

    /// Get an action definition from a specific capability.
    pub fn get_action(&self, capability_name: &str, action_name: &str) -> Option<&ActionDef> {
        self.capabilities
            .get(capability_name)?
            .actions
            .iter()
            .find(|a| a.name == action_name)
    }

    /// Collect all action definitions across all capabilities.
    pub fn all_actions(&self) -> Vec<&ActionDef> {
        self.capabilities
            .values()
            .flat_map(|c| c.actions.iter())
            .collect()
    }

    /// Number of registered capabilities.
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{EffectType, ModelToolSurface};

    fn test_capability() -> Capability {
        Capability {
            name: "github".into(),
            description: "GitHub integration".into(),
            actions: vec![
                ActionDef {
                    name: "create_issue".into(),
                    description: "Create a GitHub issue".into(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: vec![EffectType::WriteExternal, EffectType::CredentialedNetwork],
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::FullSchema,
                    discovery: None,
                },
                ActionDef {
                    name: "list_prs".into(),
                    description: "List pull requests".into(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: vec![EffectType::ReadExternal, EffectType::CredentialedNetwork],
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::FullSchema,
                    discovery: None,
                },
            ],
            knowledge: vec!["When creating issues, always add labels.".into()],
            policies: vec![],
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = CapabilityRegistry::new();
        reg.register(test_capability());
        assert_eq!(reg.len(), 1);
        assert!(reg.get("github").is_some());
        assert!(reg.get("slack").is_none());
    }

    #[test]
    fn find_action_across_capabilities() {
        let mut reg = CapabilityRegistry::new();
        reg.register(test_capability());
        let (cap_name, action) = reg.find_action("create_issue").unwrap();
        assert_eq!(cap_name, "github");
        assert_eq!(action.name, "create_issue");
        assert!(reg.find_action("nonexistent").is_none());
    }

    #[test]
    fn get_action_from_capability() {
        let mut reg = CapabilityRegistry::new();
        reg.register(test_capability());
        assert!(reg.get_action("github", "list_prs").is_some());
        assert!(reg.get_action("github", "delete_repo").is_none());
        assert!(reg.get_action("slack", "list_prs").is_none());
    }

    #[test]
    fn all_actions_collects_across_capabilities() {
        let mut reg = CapabilityRegistry::new();
        reg.register(test_capability());
        reg.register(Capability {
            name: "memory".into(),
            description: "Memory tools".into(),
            actions: vec![ActionDef {
                name: "memory_search".into(),
                description: "Search memory".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
                model_tool_surface: ModelToolSurface::FullSchema,
                discovery: None,
            }],
            knowledge: vec![],
            policies: vec![],
        });
        assert_eq!(reg.all_actions().len(), 3);
    }

    #[test]
    fn overwrite_on_re_register() {
        let mut reg = CapabilityRegistry::new();
        reg.register(test_capability());
        assert_eq!(reg.get("github").unwrap().actions.len(), 2);

        reg.register(Capability {
            name: "github".into(),
            description: "Updated".into(),
            actions: vec![],
            knowledge: vec![],
            policies: vec![],
        });
        assert_eq!(reg.get("github").unwrap().actions.len(), 0);
        assert_eq!(reg.len(), 1);
    }
}
