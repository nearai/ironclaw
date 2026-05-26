mod support;

use std::sync::Arc;

use ironclaw_auth::{
    GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE,
    GOOGLE_GMAIL_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
};
use ironclaw_first_party_extensions::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
    CALENDAR_GET_EVENT_CAPABILITY_ID, CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
    CALENDAR_LIST_EVENTS_CAPABILITY_ID, CALENDAR_SET_REMINDER_CAPABILITY_ID,
    CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
};
use ironclaw_host_api::{NetworkMethod, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse};
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
