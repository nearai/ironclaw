//! Integration tests for cognitive routines module.
//!
//! Tests checkpointing, pre-game instructions, and after-action reviews.

use ironclaw::agent::cognitive::{
    after_action_template, pre_game_instructions, post_compaction_recovery,
    CognitiveConfig, CheckpointTracker,
};

#[test]
fn test_checkpoint_tracker_full_workflow() {
    let mut tracker = CheckpointTracker::default();
    
    // Simulate a conversation
    for i in 0..10 {
        tracker.record_exchange();
        if i == 3 {
            tracker.add_topic("IronClaw memory system");
        }
        if i == 7 {
            tracker.add_decision("Use position-aware chunking for citations");
        }
    }
    
    // Should not need checkpoint yet (10 < 15)
    assert!(!tracker.needs_checkpoint(15));
    
    // Continue conversation
    for _ in 0..5 {
        tracker.record_exchange();
    }
    
    // Now should need checkpoint (15 >= 15)
    assert!(tracker.needs_checkpoint(15));
    
    // Generate checkpoint content
    let content = tracker.generate_checkpoint_content();
    assert!(content.contains("Conversation Checkpoint"));
    assert!(content.contains("IronClaw memory system"));
    assert!(content.contains("position-aware chunking"));
    
    // Reset and verify cleared
    tracker.reset();
    assert_eq!(tracker.exchanges_since_checkpoint, 0);
    assert!(tracker.topics.is_empty());
    assert!(tracker.decisions.is_empty());
    assert!(tracker.last_checkpoint.is_some());
}

#[test]
fn test_checkpoint_tracker_no_duplicates() {
    let mut tracker = CheckpointTracker::default();
    
    // Add same topic multiple times
    tracker.add_topic("memory");
    tracker.add_topic("memory");
    tracker.add_topic("memory");
    
    // Should only have one entry
    assert_eq!(tracker.topics.len(), 1);
}

#[test]
fn test_pre_game_instructions_content() {
    let instructions = pre_game_instructions();
    
    // Should contain all 5 checklist items
    assert!(instructions.contains("Restate the task"));
    assert!(instructions.contains("constraints"));
    assert!(instructions.contains("success criteria"));
    assert!(instructions.contains("memory"));
    assert!(instructions.contains("Preparation"));
    assert!(instructions.contains("Execution"));
}

#[test]
fn test_after_action_template_format() {
    let template = after_action_template("Deploy IronClaw to Production");
    
    // Should have the task name
    assert!(template.contains("Deploy IronClaw to Production"));
    
    // Should have all sections
    assert!(template.contains("What happened"));
    assert!(template.contains("Tools used"));
    assert!(template.contains("What I'd do differently"));
    
    // Should have timestamp
    assert!(template.contains("202")); // Year prefix for UTC timestamp
}

#[test]
fn test_post_compaction_recovery_instructions() {
    let recovery = post_compaction_recovery();
    
    // Should have recovery steps
    assert!(recovery.contains("daily notes"));
    assert!(recovery.contains("BRIEFING.md"));
    assert!(recovery.contains("memory_search"));
    assert!(recovery.contains("honest"));
}

#[test]
fn test_cognitive_config_defaults() {
    let config = CognitiveConfig::default();
    
    assert!(config.pre_game_enabled);
    assert!(config.checkpointing_enabled);
    assert_eq!(config.checkpoint_interval, 15);
    assert!(!config.after_action_enabled); // Disabled by default
}

#[test]
fn test_cognitive_config_serialization() {
    let config = CognitiveConfig {
        pre_game_enabled: false,
        checkpointing_enabled: true,
        checkpoint_interval: 20,
        after_action_enabled: true,
    };
    
    // Round-trip through JSON
    let json = serde_json::to_string(&config).expect("serialize");
    let restored: CognitiveConfig = serde_json::from_str(&json).expect("deserialize");
    
    assert_eq!(restored.pre_game_enabled, false);
    assert_eq!(restored.checkpoint_interval, 20);
    assert_eq!(restored.after_action_enabled, true);
}

#[test]
fn test_checkpoint_content_escaping() {
    let mut tracker = CheckpointTracker::default();
    
    // Add topics/decisions with potentially problematic content
    tracker.add_topic("User asked about <script>alert('xss')</script>");
    tracker.add_decision("Decided to use 'single quotes' and \"double quotes\"");
    tracker.add_decision("Path: ../../../etc/passwd");
    
    let content = tracker.generate_checkpoint_content();
    
    // Content should be generated (no panic)
    assert!(content.contains("Conversation Checkpoint"));
    // The content is included as-is (sanitization is the caller's responsibility for log storage)
    assert!(content.contains("script"));
    assert!(content.contains("quotes"));
}

#[test]
fn test_checkpoint_empty_state() {
    let tracker = CheckpointTracker::default();
    
    // Even with no topics/decisions, should generate valid content
    let content = tracker.generate_checkpoint_content();
    assert!(content.contains("Conversation Checkpoint"));
    // Should not contain topic/decision headers when empty
    assert!(!content.contains("Currently discussing"));
    assert!(!content.contains("Key decisions"));
}
