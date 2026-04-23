use ironclaw_engine::{ActionDef, ActionInventory};

#[derive(Debug, Clone)]
pub(crate) struct InventoryCandidate {
    pub(crate) action: ActionDef,
}

impl InventoryCandidate {
    pub(crate) fn builtin(action: ActionDef) -> Self {
        Self { action }
    }

    pub(crate) fn provider_backed(action: ActionDef) -> Self {
        Self { action }
    }

    pub(crate) fn engine_native(action: ActionDef) -> Self {
        Self { action }
    }
}

pub(crate) struct ActionInventoryPlanner;

impl ActionInventoryPlanner {
    pub(crate) fn plan(candidates: Vec<InventoryCandidate>) -> ActionInventory {
        let mut inline = candidates
            .into_iter()
            .map(|candidate| candidate.action)
            .collect::<Vec<_>>();
        inline.sort_by(|left, right| left.name.cmp(&right.name));
        ActionInventory { inline }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_engine::ActionDef;

    use super::{ActionInventoryPlanner, InventoryCandidate};

    fn action(name: &str, schema: serde_json::Value) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: format!("Action {name}"),
            parameters_schema: schema,
            effects: Vec::new(),
            requires_approval: false,
            discovery: None,
        }
    }

    #[test]
    fn provider_backed_actions_stay_inline_callable() {
        let inventory = ActionInventoryPlanner::plan(vec![
            InventoryCandidate::provider_backed(action(
                "gmail",
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": true
                }),
            )),
            InventoryCandidate::builtin(action(
                "shell",
                serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}}),
            )),
        ]);

        assert_eq!(
            inventory
                .inline
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gmail", "shell"]
        );
    }

    #[test]
    fn engine_native_actions_stay_callable() {
        let inventory =
            ActionInventoryPlanner::plan(vec![InventoryCandidate::engine_native(action(
                "mission_create",
                serde_json::json!({
                    "type": "object",
                    "properties": {"name": {"type": "string"}}
                }),
            ))]);

        assert_eq!(
            inventory
                .inline
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>(),
            vec!["mission_create"]
        );
    }

    #[test]
    fn builtins_stay_inline_callable() {
        let inventory = ActionInventoryPlanner::plan(vec![InventoryCandidate::builtin(action(
            "echo",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"}
                },
                "required": ["text"]
            }),
        ))]);

        assert_eq!(inventory.inline[0].name, "echo");
    }

    #[test]
    fn essential_builtins_stay_inline() {
        let inventory = ActionInventoryPlanner::plan(vec![InventoryCandidate::builtin(action(
            "tool_info",
            serde_json::json!({
                "type": "object",
                "properties": {"name": {"type": "string"}}
            }),
        ))]);

        assert_eq!(inventory.inline[0].name, "tool_info");
    }

    #[test]
    fn critical_setup_and_inventory_builtins_stay_inline() {
        let names = [
            "skill_install",
            "skill_list",
            "tool_activate",
            "tool_install",
            "tool_list",
        ];

        let inventory = ActionInventoryPlanner::plan(
            names
                .iter()
                .map(|name| {
                    InventoryCandidate::builtin(action(
                        name,
                        serde_json::json!({
                            "type": "object",
                            "properties": {"name": {"type": "string"}}
                        }),
                    ))
                })
                .collect(),
        );

        assert_eq!(
            inventory
                .inline
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>(),
            names
        );
    }
}
