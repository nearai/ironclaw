//! E2E tests for attachment processing in the LLM pipeline.
//!
//! Verifies that attachments on incoming messages are augmented into the user
//! text and (for images) passed as multimodal content parts to the LLM.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod attachment_tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    use ironclaw::channels::{AttachmentKind, IncomingAttachment, IncomingMessage};
    use ironclaw::llm::ContentPart;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/spot"
    );
    const TIMEOUT: Duration = Duration::from_secs(15);

    fn engine_v2_project_root() -> PathBuf {
        let base_dir = ironclaw::bootstrap::ironclaw_base_dir();
        base_dir.parent().map(PathBuf::from).unwrap_or(base_dir)
    }

    fn make_attachment(kind: AttachmentKind) -> IncomingAttachment {
        IncomingAttachment {
            id: "att-1".to_string(),
            kind,
            mime_type: "application/octet-stream".to_string(),
            filename: None,
            size_bytes: None,
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            data: vec![],
            duration_secs: None,
        }
    }

    /// Audio attachment with transcript reaches the LLM as augmented text.
    #[tokio::test]
    async fn attachment_audio_transcript_reaches_llm() {
        let trace =
            LlmTrace::from_file(format!("{FIXTURES}/attachment_audio_transcript.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        // Build a message with an audio attachment containing a transcript
        let mut att = make_attachment(AttachmentKind::Audio);
        att.filename = Some("voice.ogg".to_string());
        att.mime_type = "audio/ogg".to_string();
        att.extracted_text = Some("Hello, can you help me with my project?".to_string());
        att.duration_secs = Some(5);

        let mut msg = IncomingMessage::new("test", "test-user", "Check this voice note");
        msg.attachments.push(att);

        rig.send_incoming(msg).await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Verify the response was received
        assert!(
            !responses.is_empty(),
            "should receive at least one response"
        );

        // Verify the augmented content reached the LLM
        let requests = rig.captured_llm_requests();
        assert!(!requests.is_empty(), "LLM should have been called");

        let last_request = &requests[requests.len() - 1];
        let last_user_msg = last_request
            .iter()
            .rev()
            .find(|m| matches!(m.role, ironclaw::llm::Role::User))
            .expect("should have a user message");

        // The augmented text should contain the attachment tags and transcript
        assert!(
            last_user_msg.content.contains("<attachments>"),
            "user message should contain <attachments> block, got: {}",
            last_user_msg.content.chars().take(200).collect::<String>()
        );
        assert!(
            last_user_msg
                .content
                .contains("Hello, can you help me with my project?"),
            "user message should contain the transcript"
        );
        assert!(
            last_user_msg.content.contains("duration=\"5s\""),
            "user message should contain duration"
        );

        // Audio attachments should NOT produce image content parts
        assert!(
            last_user_msg.content_parts.is_empty(),
            "audio attachments should not produce image content parts"
        );

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    /// Image attachment with data reaches the LLM with multimodal content parts.
    #[tokio::test]
    async fn attachment_image_produces_content_parts() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/attachment_image.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        // Build a message with an image attachment that has raw data
        let mut att = make_attachment(AttachmentKind::Image);
        att.filename = Some("screenshot.png".to_string());
        att.mime_type = "image/png".to_string();
        att.size_bytes = Some(1024);
        att.data = vec![0x89, 0x50, 0x4E, 0x47]; // PNG magic bytes (fake)

        let mut msg =
            IncomingMessage::new("test", "test-user", "What do you see in this screenshot?");
        msg.attachments.push(att);

        rig.send_incoming(msg).await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(
            !responses.is_empty(),
            "should receive at least one response"
        );

        // Verify multimodal content parts reached the LLM
        let requests = rig.captured_llm_requests();
        assert!(!requests.is_empty(), "LLM should have been called");

        let last_request = &requests[requests.len() - 1];
        let last_user_msg = last_request
            .iter()
            .rev()
            .find(|m| matches!(m.role, ironclaw::llm::Role::User))
            .expect("should have a user message");

        // Should have image content parts
        assert_eq!(
            last_user_msg.content_parts.len(),
            1,
            "should have exactly one image content part"
        );

        // Verify the content part is an ImageUrl with a data: URI
        match &last_user_msg.content_parts[0] {
            ContentPart::ImageUrl { image_url } => {
                assert!(
                    image_url.url.starts_with("data:image/png;base64,"),
                    "image URL should be a base64 data URI, got: {}",
                    &image_url.url[..image_url.url.len().min(40)]
                );
            }
            other => panic!("expected ImageUrl content part, got: {:?}", other),
        }

        // The text should note the image is sent as visual content
        assert!(
            last_user_msg.content.contains(
                "[Image attached — you can already see this image directly in the conversation."
            ),
            "augmented text should note image sent as visual content"
        );

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    /// Message without attachments should have no content_parts and no augmentation.
    #[tokio::test]
    async fn no_attachments_no_augmentation() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_greeting.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Hello! Introduce yourself briefly.").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        let requests = rig.captured_llm_requests();
        let last_request = &requests[requests.len() - 1];
        let last_user_msg = last_request
            .iter()
            .rev()
            .find(|m| matches!(m.role, ironclaw::llm::Role::User))
            .expect("should have a user message");

        // No attachments → no augmentation tags, no content parts
        assert!(
            !last_user_msg.content.contains("<attachments>"),
            "plain message should NOT contain <attachments>"
        );
        assert!(
            last_user_msg.content_parts.is_empty(),
            "plain message should have no content parts"
        );

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn engine_v2_channel_attachments_persist_for_telegram_and_whatsapp() {
        let project_root = engine_v2_project_root();

        for channel in ["telegram", "whatsapp"] {
            let rig = TestRigBuilder::new().with_engine_v2().build().await;

            let attachment_bytes = format!("Attachment from {channel}").into_bytes();
            let mut msg = IncomingMessage::new(channel, "cross-channel-user", "check this file");
            msg.attachments.push(IncomingAttachment {
                id: format!("{channel}-att-1"),
                kind: AttachmentKind::Document,
                mime_type: "text/plain".to_string(),
                filename: Some(format!("{channel}-notes.txt")),
                size_bytes: Some(attachment_bytes.len() as u64),
                source_url: None,
                storage_key: None,
                local_path: None,
                extracted_text: Some(format!("Attachment from {channel}")),
                data: attachment_bytes.clone(),
                duration_secs: None,
            });

            rig.send_incoming(msg).await;
            let deadline = tokio::time::Instant::now() + TIMEOUT;
            let requests = loop {
                let requests = rig.captured_llm_requests();
                if !requests.is_empty() {
                    break requests;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "should capture an LLM request for {channel}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            let last_request = requests.last().expect("captured LLM request");
            let last_user_msg = last_request
                .iter()
                .rev()
                .find(|m| matches!(m.role, ironclaw::llm::Role::User))
                .expect("user message");

            let expected_suffix = format!("{channel}-notes.txt");
            let project_path = last_user_msg
                .content
                .split("project_path=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .expect("project_path attribute");
            assert!(
                project_path.contains(".ironclaw/attachments/"),
                "missing persisted attachment path for {channel}: {}",
                last_user_msg.content
            );
            assert!(
                project_path.ends_with(&expected_suffix),
                "unexpected persisted path for {channel}: {project_path}"
            );

            let saved_path = project_root.join(project_path);
            assert!(
                saved_path.exists(),
                "saved attachment missing: {}",
                saved_path.display()
            );
            assert_eq!(
                std::fs::read(saved_path).expect("read saved attachment"),
                attachment_bytes
            );

            rig.shutdown();
        }
    }
}
