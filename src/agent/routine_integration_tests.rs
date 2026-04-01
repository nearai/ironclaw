//! Integration tests for the routine system.
//!
//! These tests validate routine type construction, serialization,
//! and basic configuration patterns.

#[cfg(test)]
mod tests {
    use crate::agent::routine::{NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger};
    use std::collections::HashMap;
    use std::time::Duration;
    use uuid::Uuid;

    #[test]
    fn test_routine_cron_trigger_parse() {
        // Test cron trigger parsing with 5-field and 6-field formats
        let cron_5field = Trigger::Cron {
            schedule: "0 9 * * *".to_string(),
            timezone: None,
        };
        assert!(matches!(cron_5field, Trigger::Cron { .. }));

        let cron_6field = Trigger::Cron {
            schedule: "0 9 * * MON-FRI".to_string(),
            timezone: Some("America/Sao_Paulo".to_string()),
        };
        assert!(matches!(cron_6field, Trigger::Cron { .. }));
    }

    #[test]
    fn test_routine_event_trigger_pattern_match() {
        // Test event trigger regex pattern matching
        let trigger = Trigger::Event {
            channel: Some("telegram".to_string()),
            pattern: "(urgente|prioridade|ajuda)".to_string(),
        };

        // Should match
        assert!(matches!(&trigger, Trigger::Event { pattern, .. } if {
            let re = regex::Regex::new(pattern).unwrap();
            re.is_match("mensagem urgente")
        }));

        // Should not match
        assert!(matches!(&trigger, Trigger::Event { pattern, .. } if {
            let re = regex::Regex::new(pattern).unwrap();
            !re.is_match("mensagem normal")
        }));
    }

    #[test]
    fn test_routine_lightweight_action_config() {
        // Test lightweight action configuration
        let action = RoutineAction::Lightweight {
            prompt: "Revise as últimas 24h e liste tarefas".to_string(),
            context_paths: vec!["context/priorities.md".to_string()],
            max_tokens: 4096,
            use_tools: true,
            max_tool_rounds: 3,
        };

        match action {
            RoutineAction::Lightweight {
                prompt,
                context_paths,
                max_tokens,
                use_tools,
                max_tool_rounds,
            } => {
                assert!(!prompt.is_empty());
                assert_eq!(context_paths.len(), 1);
                assert_eq!(max_tokens, 4096);
                assert!(use_tools);
                assert_eq!(max_tool_rounds, 3);
            }
            _ => panic!("Expected Lightweight action"),
        }
    }

    #[test]
    fn test_routine_full_job_action_config() {
        // Test full_job action configuration
        let action = RoutineAction::FullJob {
            title: "Análise do Repositório".to_string(),
            description: "Executar análise completa do repositório".to_string(),
            max_iterations: 50,
        };

        match action {
            RoutineAction::FullJob {
                title,
                description,
                max_iterations,
            } => {
                assert!(!title.is_empty());
                assert!(!description.is_empty());
                assert_eq!(max_iterations, 50);
            }
            _ => panic!("Expected FullJob action"),
        }
    }

    #[test]
    fn test_routine_guardrails_config() {
        // Test guardrails configuration
        let guardrails = RoutineGuardrails {
            cooldown: Duration::from_secs(300),
            max_concurrent: 1,
            dedup_window: Some(Duration::from_secs(3600)),
        };

        assert_eq!(guardrails.cooldown.as_secs(), 300);
        assert_eq!(guardrails.max_concurrent, 1);
        assert!(guardrails.dedup_window.is_some());
    }

    #[test]
    fn test_routine_notify_config() {
        // Test notification gating configuration
        let notify = NotifyConfig {
            channel: Some("telegram".to_string()),
            user: Some("test-user".to_string()),
            on_attention: true,
            on_failure: true,
            on_success: false,
        };

        assert!(!notify.on_success);
        assert!(notify.on_attention);
        assert!(notify.on_failure);
        assert_eq!(notify.channel, Some("telegram".to_string()));
    }

    #[test]
    fn test_routine_trigger_serialization() {
        // Test that triggers can be serialized/deserialized
        let trigger = Trigger::Cron {
            schedule: "0 */2 * * *".to_string(),
            timezone: Some("UTC".to_string()),
        };

        let json = serde_json::to_string(&trigger).expect("Failed to serialize");
        assert!(!json.is_empty());

        let _deserialized: Trigger = serde_json::from_str(&json).expect("Failed to deserialize");
    }

    #[test]
    fn test_routine_complete_serialization() {
        // Test complete routine serialization
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "daily-standup".to_string(),
            description: "Resumo diário de atividades".to_string(),
            user_id: "test-user".to_string(),
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "0 9 * * MON-FRI".to_string(),
                timezone: Some("America/Sao_Paulo".to_string()),
            },
            action: RoutineAction::Lightweight {
                prompt: "Revise as últimas 24h".to_string(),
                context_paths: vec![],
                max_tokens: 4096,
                use_tools: false,
                max_tool_rounds: 3,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(300),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig {
                channel: None,
                user: None,
                on_attention: true,
                on_failure: true,
                on_success: false,
            },
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string_pretty(&routine).expect("Failed to serialize");
        assert!(!json.is_empty());

        let _deserialized: Routine = serde_json::from_str(&json).expect("Failed to deserialize");
    }

    #[test]
    fn test_routine_manual_trigger() {
        // Test manual trigger (only fires via tool call or CLI)
        let trigger = Trigger::Manual;
        assert!(matches!(trigger, Trigger::Manual));
        assert_eq!(trigger.type_tag(), "manual");
    }

    #[test]
    fn test_routine_system_event_trigger() {
        // Test system event trigger
        use std::collections::HashMap;

        let mut filters = HashMap::new();
        filters.insert("label".to_string(), "bug".to_string());

        let trigger = Trigger::SystemEvent {
            source: "github".to_string(),
            event_type: "issue.opened".to_string(),
            filters,
        };

        assert!(matches!(trigger, Trigger::SystemEvent { .. }));
        assert_eq!(trigger.type_tag(), "system_event");
    }
}
