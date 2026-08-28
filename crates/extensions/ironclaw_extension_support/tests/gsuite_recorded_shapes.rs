mod support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_auth::{
    GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE,
    GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
};
use ironclaw_extension_support::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
    CALENDAR_GET_EVENT_CAPABILITY_ID, CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
    CALENDAR_LIST_EVENTS_CAPABILITY_ID, CALENDAR_SET_REMINDER_CAPABILITY_ID,
    CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
};
use ironclaw_host_api::{
    action::NetworkMethod,
    dispatch::RuntimeDispatchErrorKind,
    http::{RuntimeHttpEgressRequest, RuntimeHttpEgressResponse},
};
use serde_json::{Value, json};
use support::*;

struct GsuiteShapeCase {
    capability: &'static str,
    input: Value,
    provider_scopes: Vec<&'static str>,
    responses: Vec<RuntimeHttpEgressResponse>,
}

impl GsuiteShapeCase {
    fn new(
        capability: &'static str,
        input: Value,
        provider_scopes: &[&'static str],
        responses: Vec<RuntimeHttpEgressResponse>,
    ) -> Self {
        Self {
            capability,
            input,
            provider_scopes: provider_scopes.to_vec(),
            responses,
        }
    }

    async fn dispatch(self) -> (Value, Vec<RuntimeHttpEgressRequest>) {
        let scope = scope();
        let auth = auth_with_google_account(
            &scope,
            self.provider_scopes
                .iter()
                .map(|scope| provider_scope(scope))
                .collect(),
        )
        .await;
        let egress = Arc::new(RecordingEgress::with_responses(self.responses));
        let output = dispatch_ok(auth, scope, self.capability, self.input, egress.clone()).await;
        (output, egress.requests())
    }
}

fn json_response(area: &str, name: &str) -> RuntimeHttpEgressResponse {
    RecordingEgress::json_status(200, fixture(area, name))
}

fn request_body(request: &RuntimeHttpEgressRequest, label: &str) -> Value {
    serde_json::from_slice::<Value>(&request.body).expect(label)
}

#[tokio::test]
async fn calendar_read_handlers_use_recorded_google_api_shapes() {
    let (calendars, requests) = GsuiteShapeCase::new(
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        json!({}),
        &[GOOGLE_CALENDAR_READONLY_SCOPE],
        vec![json_response("calendar", "calendar_list.json")],
    )
    .dispatch()
    .await;
    assert_eq!(calendars["body"]["items"][0]["id"], "primary");
    assert_eq!(calendars["body"]["items"][1]["id"], "team@example.com");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.ends_with("/users/me/calendarList"));

    let (events, requests) = GsuiteShapeCase::new(
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "time_min": "2026-05-21T00:00:00Z",
            "time_max": "2026-05-22T00:00:00Z",
            "max_results": 50
        }),
        &[GOOGLE_CALENDAR_READONLY_SCOPE],
        vec![json_response("calendar", "events_list.json")],
    )
    .dispatch()
    .await;
    assert_eq!(events["body"]["nextPageToken"], "CiAKGjBpNDd2Nm");
    assert_eq!(
        events["body"]["items"]
            .as_array()
            .expect("Calendar events response items is an array")
            .len(),
        2
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.contains("/calendars/primary/events"));
    assert!(requests[0].url.contains("timeMin=2026-05-21T00%3A00%3A00Z"));
    assert!(requests[0].url.contains("maxResults=50"));

    let (event, requests) = GsuiteShapeCase::new(
        CALENDAR_GET_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event_id": "evt-standup-001" }),
        &[GOOGLE_CALENDAR_READONLY_SCOPE],
        vec![json_response("calendar", "event_get.json")],
    )
    .dispatch()
    .await;
    assert_eq!(event["body"]["id"], "evt-standup-001");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/events/evt-standup-001"));

    let (free_busy, requests) = GsuiteShapeCase::new(
        CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
        json!({
            "timeMin": "2026-05-21T09:00:00Z",
            "timeMax": "2026-05-21T17:00:00Z",
            "items": [{ "id": "primary" }]
        }),
        &[GOOGLE_CALENDAR_READONLY_SCOPE],
        vec![json_response("calendar", "free_busy.json")],
    )
    .dispatch()
    .await;
    assert_eq!(
        free_busy["body"]["calendars"]["primary"]["busy"][0]["start"],
        "2026-05-21T10:00:00Z"
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert!(requests[0].url.ends_with("/freeBusy"));
}

#[tokio::test]
async fn calendar_write_handlers_use_recorded_google_api_shapes() {
    let (created, requests) = GsuiteShapeCase::new(
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event": { "summary": "Project review" } }),
        &[GOOGLE_CALENDAR_EVENTS_SCOPE],
        vec![json_response("calendar", "event_created.json")],
    )
    .dispatch()
    .await;
    assert_eq!(created["body"]["id"], "evt-created-099");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(
        request_body(&requests[0], "parse Calendar create request body")["summary"],
        "Project review"
    );

    let (updated, requests) = GsuiteShapeCase::new(
        CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "event": { "summary": "Updated review" }
        }),
        &[GOOGLE_CALENDAR_EVENTS_SCOPE],
        vec![json_response("calendar", "event_created.json")],
    )
    .dispatch()
    .await;
    assert_eq!(updated["body"]["id"], "evt-created-099");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Patch);
    assert_eq!(
        request_body(&requests[0], "parse Calendar update request body")["summary"],
        "Updated review"
    );

    let (deleted, requests) = GsuiteShapeCase::new(
        CALENDAR_DELETE_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event_id": "evt-001" }),
        &[GOOGLE_CALENDAR_EVENTS_SCOPE],
        vec![RecordingEgress::empty(204)],
    )
    .dispatch()
    .await;
    assert_eq!(deleted["status"], 204);
    assert!(deleted["body"].is_null());
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Delete);

    let (attendees_added, requests) = GsuiteShapeCase::new(
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "attendees": [{ "email": "ada@example.com" }]
        }),
        &[GOOGLE_CALENDAR_EVENTS_SCOPE],
        vec![
            json_response("calendar", "event_get.json"),
            json_response("calendar", "event_created.json"),
        ],
    )
    .dispatch()
    .await;
    assert_eq!(attendees_added["body"]["id"], "evt-created-099");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(requests[1].method, NetworkMethod::Patch);
    assert_eq!(
        request_body(&requests[1], "parse Calendar attendees request body")["attendees"][0]["email"],
        "ada@example.com"
    );

    let (reminders_set, requests) = GsuiteShapeCase::new(
        CALENDAR_SET_REMINDER_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "reminders": {
                "useDefault": false,
                "overrides": [{ "method": "popup", "minutes": 10 }]
            }
        }),
        &[GOOGLE_CALENDAR_EVENTS_SCOPE],
        vec![json_response("calendar", "event_created.json")],
    )
    .dispatch()
    .await;
    assert_eq!(reminders_set["body"]["id"], "evt-created-099");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Patch);
    assert_eq!(
        request_body(&requests[0], "parse Calendar reminders request body")["reminders"]["overrides"]
            [0]["minutes"],
        10
    );
}

#[tokio::test]
async fn calendar_handler_preserves_insufficient_scope_response() {
    let (output, requests) = GsuiteShapeCase::new(
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        json!({}),
        &[GOOGLE_CALENDAR_READONLY_SCOPE],
        vec![RecordingEgress::json_status(
            403,
            fixture("calendar", "insufficient_scope.json"),
        )],
    )
    .dispatch()
    .await;

    assert_eq!(output["status"], 403);
    assert_eq!(output["body"]["error"]["status"], "PERMISSION_DENIED");
    assert_eq!(
        output["body"]["error"]["details"][0]["reason"],
        "insufficient_scope"
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.ends_with("/users/me/calendarList"));
}

#[tokio::test]
async fn gmail_handlers_use_recorded_google_api_shapes() {
    let (messages, requests) = GsuiteShapeCase::new(
        GMAIL_LIST_MESSAGES_CAPABILITY_ID,
        json!({ "query": "is:unread from:ada", "max_results": 25 }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![json_response("gmail", "messages_list.json")],
    )
    .dispatch()
    .await;
    assert_eq!(messages["body"]["messages"][0]["id"], "msg-001");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.contains("/users/me/messages"));
    assert!(requests[0].url.contains("q=is%3Aunread%20from%3Aada"));
    assert!(requests[0].url.contains("maxResults=25"));

    let (message, requests) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-001" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![json_response("gmail", "message_get.json")],
    )
    .dispatch()
    .await;
    assert_eq!(message["body"]["id"], "msg-001");
    assert_eq!(message["body"]["thread_id"], "thr-001");
    assert_eq!(
        message["body"]["headers"],
        json!({
            "from": "Ada Lovelace <ada@example.com>",
            "to": "Bob Hawk <bob@example.com>",
            "cc": "Grace Hopper <grace@example.com>",
            "reply_to": "launch@example.com",
            "subject": "Q2 summary",
            "date": "Wed, 20 May 2026 14:00:00 -0400"
        })
    );
    assert_eq!(message["body"]["body"]["kind"], "text");
    assert_eq!(
        message["body"]["body"]["text"],
        "Quarterly numbers are in.\nSecond line."
    );
    assert_eq!(
        message["body"]["attachments"],
        json!([{
            "attachment_id": "attachment-001",
            "filename": "summary.pdf",
            "mime_type": "application/pdf",
            "size": 4096
        }])
    );
    let serialized = serde_json::to_string(&message).expect("semantic Gmail output serializes");
    assert!(!serialized.contains("X-Internal-Trace-Id"));
    assert!(!serialized.contains("trace-should-not-leak"));
    assert!(!serialized.contains("UXVhcnRlcmx5"));
    assert!(!serialized.contains("Ignore this HTML alternative"));
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .url
            .contains("/users/me/messages/msg-001?format=full")
    );

    let (sent, requests) = GsuiteShapeCase::new(
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        &[GOOGLE_GMAIL_SEND_SCOPE],
        vec![json_response("gmail", "message_sent.json")],
    )
    .dispatch()
    .await;
    assert_eq!(sent["body"]["id"], "msg-sent-700");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
    assert_eq!(
        request_body(&requests[0], "parse Gmail send request body")["raw"],
        "base64url-rfc822"
    );

    let (draft, requests) = GsuiteShapeCase::new(
        GMAIL_CREATE_DRAFT_CAPABILITY_ID,
        json!({ "draft": { "message": { "raw": "base64url-rfc822" } } }),
        &[GOOGLE_GMAIL_MODIFY_SCOPE],
        vec![json_response("gmail", "draft_created.json")],
    )
    .dispatch()
    .await;
    assert_eq!(draft["body"]["id"], "draft-501");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/users/me/drafts"));
    assert_eq!(
        request_body(&requests[0], "parse Gmail draft request body")["message"]["raw"],
        "base64url-rfc822"
    );

    let (reply, requests) = GsuiteShapeCase::new(
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822", "threadId": "thr-001" } }),
        &[GOOGLE_GMAIL_SEND_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE],
        vec![json_response("gmail", "message_sent.json")],
    )
    .dispatch()
    .await;
    assert_eq!(reply["body"]["id"], "msg-sent-700");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/users/me/messages/send"));
    assert_eq!(
        request_body(&requests[0], "parse Gmail reply request body")["threadId"],
        "thr-001"
    );

    let (trashed, requests) = GsuiteShapeCase::new(
        GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-001" }),
        &[GOOGLE_GMAIL_MODIFY_SCOPE],
        vec![json_response("gmail", "message_trashed.json")],
    )
    .dispatch()
    .await;
    assert_eq!(trashed["body"]["labelIds"][0], "TRASH");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .url
            .ends_with("/users/me/messages/msg-001/trash")
    );
}

#[tokio::test]
async fn gmail_get_message_converts_owned_html_to_safe_markdown() {
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-html" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![json_response("gmail", "message_get_html.json")],
    )
    .dispatch()
    .await;

    assert_eq!(message["body"]["body"]["kind"], "markdown");
    let markdown = message["body"]["body"]["text"]
        .as_str()
        .expect("HTML body is exposed as Markdown text");
    assert!(markdown.contains("# Launch notes"), "{markdown}");
    assert!(
        markdown.contains("Readable **decoded** message."),
        "{markdown}"
    );
    assert!(
        markdown.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("- First item") || line.starts_with("*   First item")
        }),
        "{markdown}"
    );
    assert!(
        markdown.contains("[Details](https://example.com)"),
        "{markdown}"
    );
    assert!(markdown.contains("cargo test"), "{markdown}");

    let serialized = serde_json::to_string(&message).expect("semantic Gmail output serializes");
    for excluded in [
        "<h1>",
        "secretScript",
        "secretStyle",
        "secretComment",
        "data:image",
        "ARC-Seal",
        "DKIM-Signature",
        "arc-should-not-leak",
        "dkim-should-not-leak",
    ] {
        assert!(
            !serialized.contains(excluded),
            "leaked {excluded}: {serialized}"
        );
    }
}

#[tokio::test]
async fn gmail_get_message_excludes_content_disposition_attachments_from_body() {
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-content-disposition" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-content-disposition",
            "threadId": "thr-content-disposition",
            "payload": {
                "mimeType": "multipart/mixed",
                "parts": [
                    {
                        "mimeType": "text/plain",
                        "headers": [{
                            "name": "Content-Disposition",
                            "value": "Attachment; size=14"
                        }],
                        "body": {
                            "size": 14,
                            "data": URL_SAFE_NO_PAD.encode(b"hidden payload")
                        }
                    },
                    {
                        "mimeType": "text/plain",
                        "headers": [{
                            "name": "Content-Disposition",
                            "value": "inline"
                        }],
                        "body": {
                            "data": URL_SAFE_NO_PAD.encode(b"readable body")
                        }
                    }
                ]
            }
        }))],
    )
    .dispatch()
    .await;

    assert_eq!(message["body"]["body"]["kind"], "text");
    assert_eq!(message["body"]["body"]["text"], "readable body");
    assert_eq!(message["body"]["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(message["body"]["attachments"][0]["filename"], "");
    assert_eq!(message["body"]["attachments"][0]["mime_type"], "text/plain");
}

#[tokio::test]
async fn gmail_get_message_uses_plaintext_after_failed_html_alternative() {
    let too_deep_html = format!("{}body{}", "<div>".repeat(65), "</div>".repeat(65));
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-failed-html-alternative" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-failed-html-alternative",
            "threadId": "thr-failed-html-alternative",
            "payload": {
                "mimeType": "multipart/alternative",
                "parts": [
                    {
                        "mimeType": "text/html",
                        "body": { "data": URL_SAFE_NO_PAD.encode(too_deep_html.as_bytes()) }
                    },
                    {
                        "mimeType": "text/plain",
                        "body": { "data": URL_SAFE_NO_PAD.encode(b"plain fallback") }
                    }
                ]
            }
        }))],
    )
    .dispatch()
    .await;

    assert_eq!(message["body"]["body"]["kind"], "text");
    assert_eq!(message["body"]["body"]["text"], "plain fallback");
}

#[tokio::test]
async fn gmail_get_message_filters_data_urls_from_all_html_output_paths() {
    let html = r#"
        <p>Readable message.</p>
        <a href="https://example.com">Details</a>
        <a href="https://example.com/tracking" title="data:image/png;base64,AAAA">Tracking</a>
        <noscript><img src="data:image/png;base64,BBBB" alt="tracking"></noscript>
    "#;
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-data-url-paths" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-data-url-paths",
            "threadId": "thr-data-url-paths",
            "payload": {
                "mimeType": "text/html",
                "body": { "data": URL_SAFE_NO_PAD.encode(html.as_bytes()) }
            }
        }))],
    )
    .dispatch()
    .await;

    let serialized = serde_json::to_string(&message).expect("data URL result serializes");
    assert!(
        !serialized.contains("data:image"),
        "leaked data URL: {serialized}"
    );
    assert!(serialized.contains("https://example.com"), "{serialized}");
}

#[tokio::test]
async fn gmail_get_message_reports_encrypted_content_without_exposing_ciphertext() {
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-encrypted" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![json_response("gmail", "message_get_encrypted.json")],
    )
    .dispatch()
    .await;

    assert_eq!(message["body"]["body"]["kind"], "encrypted");
    assert_eq!(
        message["body"]["body"]["reason"],
        "encrypted content is not supported"
    );
    let serialized = serde_json::to_string(&message).expect("encrypted result serializes");
    assert!(!serialized.contains("VmVyc2lvbjogMQ"));

    let smime_ciphertext = URL_SAFE_NO_PAD.encode(b"smime-ciphertext");
    let (smime, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-smime" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-smime",
            "threadId": "thr-smime",
            "payload": {
                "mimeType": "application/pkcs7-mime; smime-type=enveloped-data",
                "headers": [{ "name": "Subject", "value": "S/MIME notes" }],
                "body": { "size": 16, "data": smime_ciphertext }
            }
        }))],
    )
    .dispatch()
    .await;
    assert_eq!(smime["body"]["body"]["kind"], "encrypted");
    assert!(
        !serde_json::to_string(&smime)
            .expect("S/MIME result serializes")
            .contains(&URL_SAFE_NO_PAD.encode(b"smime-ciphertext"))
    );

    let (mixed, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-encrypted-attachment" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-encrypted-attachment",
            "threadId": "thr-encrypted-attachment",
            "payload": {
                "mimeType": "multipart/mixed",
                "parts": [
                    {
                        "mimeType": "application/pkcs7-mime",
                        "filename": "signed.p7m",
                        "body": { "attachmentId": "attachment-p7m", "size": 16 }
                    },
                    {
                        "mimeType": "text/plain",
                        "body": { "data": URL_SAFE_NO_PAD.encode(b"Readable message body") }
                    }
                ]
            }
        }))],
    )
    .dispatch()
    .await;
    assert_eq!(mixed["body"]["body"]["kind"], "text");
    assert_eq!(mixed["body"]["body"]["text"], "Readable message body");

    let (mixed_inline, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-inline-encrypted" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-inline-encrypted",
            "threadId": "thr-inline-encrypted",
            "payload": {
                "mimeType": "multipart/mixed",
                "parts": [
                    { "mimeType": "application/pkcs7-mime", "body": {} },
                    {
                        "mimeType": "text/plain",
                        "body": { "data": URL_SAFE_NO_PAD.encode(b"Later readable body") }
                    }
                ]
            }
        }))],
    )
    .dispatch()
    .await;
    assert_eq!(mixed_inline["body"]["body"]["kind"], "text");
    assert_eq!(mixed_inline["body"]["body"]["text"], "Later readable body");
}

#[tokio::test]
async fn gmail_get_message_preserves_provider_error_shape() {
    let provider_error = json!({
        "error": { "code": 403, "message": "Gmail API denied this request" }
    });
    let (output, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-denied" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json_status(403, provider_error.clone())],
    )
    .dispatch()
    .await;

    assert_eq!(output["status"], 403);
    assert_eq!(output["body"], provider_error);
}

#[tokio::test]
async fn gmail_get_message_rejects_malformed_gmail_base64url() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)]).await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![json_response(
        "gmail",
        "message_get_malformed.json",
    )]));

    let error = dispatch_error(
        auth,
        scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-malformed" }),
        egress,
    )
    .await;

    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
    assert_eq!(
        error.usage().map(|usage| usage.network_egress_bytes),
        Some(123)
    );
}

#[tokio::test]
async fn gmail_get_message_decodes_declared_text_charset() {
    for (message_id, mime_type, headers, bytes, expected) in [
        (
            "msg-latin-1",
            "text/plain",
            json!([{ "name": "Content-Type", "value": "text/plain; charset=iso-8859-1" }]),
            b"caf\xe9".as_slice(),
            "café",
        ),
        (
            "msg-unknown-charset",
            "text/plain; charset=x-unknown",
            json!([]),
            "fallback ✓".as_bytes(),
            "fallback ✓",
        ),
    ] {
        let (message, _) = GsuiteShapeCase::new(
            GMAIL_GET_MESSAGE_CAPABILITY_ID,
            json!({ "message_id": message_id }),
            &[GOOGLE_GMAIL_READONLY_SCOPE],
            vec![RecordingEgress::json(json!({
                "id": message_id,
                "threadId": format!("thread-{message_id}"),
                "payload": {
                    "mimeType": mime_type,
                    "headers": headers,
                    "body": { "data": URL_SAFE_NO_PAD.encode(bytes) }
                }
            }))],
        )
        .dispatch()
        .await;

        assert_eq!(message["body"]["body"]["kind"], "text");
        assert_eq!(message["body"]["body"]["text"], expected);
    }
}

#[tokio::test]
async fn gmail_get_message_reports_attachment_backed_encrypted_body() {
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-root-encrypted" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-root-encrypted",
            "threadId": "thread-root-encrypted",
            "payload": {
                "mimeType": "application/pkcs7-mime",
                "filename": "smime.p7m",
                "body": { "attachmentId": "encrypted-payload", "size": 16 }
            }
        }))],
    )
    .dispatch()
    .await;

    assert_eq!(message["body"]["body"]["kind"], "encrypted");
    assert_eq!(
        message["body"]["body"]["reason"],
        "encrypted content is not supported"
    );
    assert!(message["body"]["body"].get("text").is_none());
}

#[tokio::test]
async fn gmail_get_message_bounds_decoded_body_before_model_exposure() {
    let oversized = "x".repeat(600 * 1024);
    let provider_response = json!({
        "id": "msg-large",
        "threadId": "thr-large",
        "payload": {
            "mimeType": "text/plain",
            "headers": [{ "name": "Subject", "value": "Large body" }],
            "body": {
                "size": oversized.len(),
                "data": URL_SAFE_NO_PAD.encode(oversized.as_bytes())
            }
        }
    });
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-large" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(provider_response)],
    )
    .dispatch()
    .await;

    let body = message["body"]["body"]["text"]
        .as_str()
        .expect("large readable body remains text");
    assert_eq!(body.len(), 512 * 1024);
    assert_eq!(message["body"]["body"]["truncated"], true);
    assert!(
        !serde_json::to_string(&message)
            .expect("large semantic result serializes")
            .contains(&URL_SAFE_NO_PAD.encode(oversized.as_bytes()))
    );
}

#[tokio::test]
async fn gmail_get_message_bounds_selected_headers_and_attachment_fields() {
    let oversized_header = "é".repeat(5_000);
    let oversized_filename = "é".repeat(2_500);
    let oversized_mime_type = format!("application/{}", "x".repeat(5_000));
    let oversized_attachment_id = "é".repeat(2_500);
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-large-fields" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-large-fields",
            "threadId": "thr-large-fields",
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [{ "name": "Subject", "value": oversized_header }],
                "body": {},
                "parts": [{
                    "mimeType": oversized_mime_type,
                    "filename": oversized_filename,
                    "body": {
                        "attachmentId": oversized_attachment_id,
                        "size": 1
                    }
                }]
            }
        }))],
    )
    .dispatch()
    .await;

    let subject = message["body"]["headers"]["subject"]
        .as_str()
        .expect("selected Subject remains a string");
    assert_eq!(subject.len(), 4 * 1024);
    assert_eq!(subject.chars().count(), 2 * 1024);

    let attachment = &message["body"]["attachments"][0];
    let filename = attachment["filename"]
        .as_str()
        .expect("attachment filename remains a string");
    assert_eq!(filename.len(), 512);
    assert_eq!(filename.chars().count(), 256);
    assert_eq!(
        attachment["mime_type"]
            .as_str()
            .expect("attachment MIME type remains a string")
            .len(),
        512
    );
    let attachment_id = attachment["attachment_id"]
        .as_str()
        .expect("attachment id remains a string");
    assert_eq!(attachment_id.len(), 512);
    assert_eq!(attachment_id.chars().count(), 256);
}

#[tokio::test]
async fn gmail_get_message_bounds_html_before_markdown_conversion() {
    let html = format!("<p>{}</p>", "x".repeat(600 * 1024));
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-large-html" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-large-html",
            "threadId": "thr-large-html",
            "payload": {
                "mimeType": "text/html",
                "body": {
                    "size": html.len(),
                    "data": URL_SAFE_NO_PAD.encode(html.as_bytes())
                }
            }
        }))],
    )
    .dispatch()
    .await;

    let markdown = message["body"]["body"]["text"]
        .as_str()
        .expect("large HTML remains bounded Markdown");
    assert!(markdown.len() <= 512 * 1024);
    assert_eq!(message["body"]["body"]["kind"], "markdown");
    assert_eq!(message["body"]["body"]["truncated"], true);
}

#[tokio::test]
async fn gmail_get_message_reports_unavailable_body() {
    for (message_id, payload) in [
        (
            "msg-empty-text",
            json!({ "mimeType": "text/plain", "body": {} }),
        ),
        (
            "msg-unsupported",
            json!({
                "mimeType": "multipart/mixed",
                "parts": [{ "mimeType": "application/calendar+json", "body": {} }]
            }),
        ),
    ] {
        let (message, _) = GsuiteShapeCase::new(
            GMAIL_GET_MESSAGE_CAPABILITY_ID,
            json!({ "message_id": message_id }),
            &[GOOGLE_GMAIL_READONLY_SCOPE],
            vec![RecordingEgress::json(json!({
                "id": message_id,
                "threadId": format!("thread-{message_id}"),
                "payload": payload
            }))],
        )
        .dispatch()
        .await;

        assert_eq!(message["body"]["body"]["kind"], "unavailable");
        assert_eq!(
            message["body"]["body"]["reason"],
            "no supported readable message body"
        );
        assert!(message["body"]["body"].get("text").is_none());
    }
}

#[tokio::test]
async fn gmail_get_message_enforces_structural_mime_bounds() {
    let mut overdeep = json!({
        "mimeType": "text/plain",
        "body": { "data": URL_SAFE_NO_PAD.encode(b"too deep") }
    });
    for _ in 0..17 {
        overdeep = json!({ "mimeType": "multipart/mixed", "parts": [overdeep] });
    }
    let resource_scope = scope();
    let auth = auth_with_google_account(
        &resource_scope,
        vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)],
    )
    .await;
    let error = dispatch_error(
        auth,
        resource_scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-too-deep" }),
        Arc::new(RecordingEgress::with_responses(vec![
            RecordingEgress::json(json!({
                "id": "msg-too-deep",
                "threadId": "thr-too-deep",
                "payload": overdeep
            })),
        ])),
    )
    .await;
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);

    let too_many_parts = (0..256)
        .map(|_| json!({ "mimeType": "application/octet-stream" }))
        .collect::<Vec<_>>();
    let resource_scope = scope();
    let auth = auth_with_google_account(
        &resource_scope,
        vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)],
    )
    .await;
    let error = dispatch_error(
        auth,
        resource_scope,
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-too-many-parts" }),
        Arc::new(RecordingEgress::with_responses(vec![
            RecordingEgress::json(json!({
                "id": "msg-too-many-parts",
                "threadId": "thr-too-many-parts",
                "payload": { "mimeType": "multipart/mixed", "parts": too_many_parts }
            })),
        ])),
    )
    .await;
    assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);

    let attachments = (0..65)
        .map(|index| {
            json!({
                "mimeType": "application/pdf",
                "filename": format!("attachment-{index}.pdf"),
                "body": { "attachmentId": format!("attachment-{index}"), "size": 1 }
            })
        })
        .collect::<Vec<_>>();
    let (message, _) = GsuiteShapeCase::new(
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-many-attachments" }),
        &[GOOGLE_GMAIL_READONLY_SCOPE],
        vec![RecordingEgress::json(json!({
            "id": "msg-many-attachments",
            "threadId": "thr-many-attachments",
            "payload": { "mimeType": "multipart/mixed", "parts": attachments }
        }))],
    )
    .dispatch()
    .await;
    assert_eq!(
        message["body"]["attachments"]
            .as_array()
            .expect("attachments remain an array")
            .len(),
        64
    );
    assert_eq!(message["body"]["attachments_truncated"], true);
}

#[tokio::test]
async fn gmail_get_message_rejects_html_over_complexity_limits() {
    let deeply_nested = format!("{}body{}", "<div>".repeat(65), "</div>".repeat(65));
    let too_many_siblings = "<span>x</span>".repeat(1_025);

    for (message_id, html) in [
        ("msg-html-too-deep", deeply_nested),
        ("msg-html-too-wide", too_many_siblings),
    ] {
        let resource_scope = scope();
        let auth = auth_with_google_account(
            &resource_scope,
            vec![provider_scope(GOOGLE_GMAIL_READONLY_SCOPE)],
        )
        .await;
        let error = dispatch_error(
            auth,
            resource_scope,
            GMAIL_GET_MESSAGE_CAPABILITY_ID,
            json!({ "message_id": message_id }),
            Arc::new(RecordingEgress::with_responses(vec![
                RecordingEgress::json(json!({
                    "id": message_id,
                    "threadId": format!("thread-{message_id}"),
                    "payload": {
                        "mimeType": "text/html",
                        "body": { "data": URL_SAFE_NO_PAD.encode(html.as_bytes()) }
                    }
                })),
            ])),
        )
        .await;

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OutputDecode);
    }
}
