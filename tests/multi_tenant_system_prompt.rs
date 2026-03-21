//! Tests proving that multi-tenant system prompts are broken.
//!
//! Bug: In multi-tenant mode, the agent loop uses `self.workspace()` which
//! returns a single shared workspace (user_id="default"). Identity files
//! (IDENTITY.md, SOUL.md, USER.md) seeded under per-user IDs ("alice",
//! "bob") are invisible to this workspace, so the system prompt is
//! empty/wrong.
//!
//! These tests:
//! 1. Seed identity files for two users (alice, bob) in the database
//! 2. Send messages as each user
//! 3. Verify the system prompt in captured LLM requests contains the
//!    correct user's identity
//! 4. Verify user A's identity doesn't leak into user B's prompt
//!
//! All tests are expected to FAIL until the bug is fixed.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironclaw::channels::IncomingMessage;
    use ironclaw::llm::Role;
    use ironclaw::workspace::Workspace;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    const TIMEOUT: Duration = Duration::from_secs(15);

    const ALICE_USER_ID: &str = "alice";
    const BOB_USER_ID: &str = "bob";

    const ALICE_IDENTITY: &str = "You are Alice's personal assistant. \
        Alice is a software engineer who lives in Seattle.";
    const BOB_IDENTITY: &str = "You are Bob's personal assistant. \
        Bob is a marine biologist who lives in Miami.";

    /// Create a simple trace that returns a canned text response.
    /// We need one step per message we plan to send.
    fn simple_trace(num_steps: usize) -> LlmTrace {
        let steps: Vec<TraceStep> = (0..num_steps)
            .map(|i| TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: format!("Response {}", i),
                    input_tokens: 100,
                    output_tokens: 10,
                },
                expected_tool_results: Vec::new(),
            })
            .collect();

        // Create separate turns for each step so the trace replays correctly.
        let turns: Vec<crate::support::trace_llm::TraceTurn> = steps
            .into_iter()
            .enumerate()
            .map(|(i, step)| crate::support::trace_llm::TraceTurn {
                user_input: format!("message {}", i),
                steps: vec![step],
                expects: Default::default(),
            })
            .collect();

        LlmTrace::new("test-model", turns)
    }

    /// Seed identity files for a user by creating a workspace scoped to that
    /// user and writing IDENTITY.md.
    async fn seed_identity(db: &Arc<dyn ironclaw::db::Database>, user_id: &str, content: &str) {
        let ws = Workspace::new_with_db(user_id, db.clone());
        ws.write("IDENTITY.md", content)
            .await
            .unwrap_or_else(|e| panic!("Failed to seed IDENTITY.md for {user_id}: {e}"));
    }

    /// Extract the system prompt from captured LLM requests.
    ///
    /// The system prompt is the first message with role=System in the first
    /// LLM request for a given turn.
    fn extract_system_prompt(requests: &[Vec<ironclaw::llm::ChatMessage>]) -> Option<String> {
        requests.last().and_then(|msgs| {
            msgs.iter()
                .find(|m| matches!(m.role, Role::System))
                .map(|m| m.content.clone())
        })
    }

    // -----------------------------------------------------------------------
    // Test 1: Alice's identity should appear in system prompt when messaging
    // as Alice.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn alice_system_prompt_contains_alice_identity() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .build()
            .await;

        // Seed alice's identity into the database
        let db = rig.database();
        seed_identity(db, ALICE_USER_ID, ALICE_IDENTITY).await;

        // Send a message AS alice (using her user_id)
        let msg = IncomingMessage::new("test", ALICE_USER_ID, "Hello, who am I?");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // The system prompt sent to the LLM should contain Alice's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests)
            .expect("Expected a system prompt in the LLM request");

        assert!(
            system_prompt.contains("Alice is a software engineer"),
            "System prompt should contain Alice's identity when messaging as Alice.\n\
             Actual system prompt:\n{system_prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: Bob's identity should appear in system prompt when messaging
    // as Bob.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bob_system_prompt_contains_bob_identity() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .build()
            .await;

        // Seed bob's identity into the database
        let db = rig.database();
        seed_identity(db, BOB_USER_ID, BOB_IDENTITY).await;

        // Send a message AS bob
        let msg = IncomingMessage::new("test", BOB_USER_ID, "Hello, who am I?");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // The system prompt should contain Bob's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests)
            .expect("Expected a system prompt in the LLM request");

        assert!(
            system_prompt.contains("Bob is a marine biologist"),
            "System prompt should contain Bob's identity when messaging as Bob.\n\
             Actual system prompt:\n{system_prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 3: Alice's identity must NOT appear in Bob's system prompt.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn alice_identity_does_not_leak_into_bob_prompt() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .build()
            .await;

        // Seed BOTH users' identities
        let db = rig.database();
        seed_identity(db, ALICE_USER_ID, ALICE_IDENTITY).await;
        seed_identity(db, BOB_USER_ID, BOB_IDENTITY).await;

        // Send a message AS bob
        let msg = IncomingMessage::new("test", BOB_USER_ID, "Tell me about myself");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Bob's prompt must NOT contain Alice's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests);

        if let Some(ref prompt) = system_prompt {
            assert!(
                !prompt.contains("Alice is a software engineer"),
                "Alice's identity LEAKED into Bob's system prompt!\n\
                 System prompt:\n{prompt}"
            );
        }
        // Also verify Bob's identity IS present (compound check)
        let prompt = system_prompt
            .expect("Expected a system prompt in the LLM request");
        assert!(
            prompt.contains("Bob is a marine biologist"),
            "Bob's own identity should be in his system prompt.\n\
             Actual system prompt:\n{prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: Bob's identity must NOT appear in Alice's system prompt.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bob_identity_does_not_leak_into_alice_prompt() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .build()
            .await;

        // Seed BOTH users' identities
        let db = rig.database();
        seed_identity(db, ALICE_USER_ID, ALICE_IDENTITY).await;
        seed_identity(db, BOB_USER_ID, BOB_IDENTITY).await;

        // Send a message AS alice
        let msg = IncomingMessage::new("test", ALICE_USER_ID, "Tell me about myself");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Alice's prompt must NOT contain Bob's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests);

        if let Some(ref prompt) = system_prompt {
            assert!(
                !prompt.contains("Bob is a marine biologist"),
                "Bob's identity LEAKED into Alice's system prompt!\n\
                 System prompt:\n{prompt}"
            );
        }
        // Also verify Alice's identity IS present
        let prompt = system_prompt
            .expect("Expected a system prompt in the LLM request");
        assert!(
            prompt.contains("Alice is a software engineer"),
            "Alice's own identity should be in her system prompt.\n\
             Actual system prompt:\n{prompt}"
        );

        rig.shutdown();
    }
}
