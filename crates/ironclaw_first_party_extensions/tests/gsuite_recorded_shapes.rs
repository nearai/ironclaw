mod support;

use std::sync::Arc;

use ironclaw_auth::{
    GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_MODIFY_SCOPE, GOOGLE_GMAIL_READONLY_SCOPE,
    GOOGLE_GMAIL_SEND_SCOPE,
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
use ironclaw_host_api::NetworkMethod;
use serde_json::{Value, json};
use support::*;

#[tokio::test]
async fn calendar_read_handlers_use_recorded_google_api_shapes() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE)])
            .await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(200, fixture("calendar", "calendar_list.json")),
        RecordingEgress::json_status(200, fixture("calendar", "events_list.json")),
        RecordingEgress::json_status(200, fixture("calendar", "event_get.json")),
        RecordingEgress::json_status(200, fixture("calendar", "free_busy.json")),
    ]));

    let calendars = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        json!({}),
        egress.clone(),
    )
    .await;
    let events = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_LIST_EVENTS_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "time_min": "2026-05-21T00:00:00Z",
            "time_max": "2026-05-22T00:00:00Z",
            "max_results": 50
        }),
        egress.clone(),
    )
    .await;
    let event = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_GET_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event_id": "evt-standup-001" }),
        egress.clone(),
    )
    .await;
    let free_busy = dispatch_ok(
        auth,
        scope,
        CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID,
        json!({
            "timeMin": "2026-05-21T09:00:00Z",
            "timeMax": "2026-05-21T17:00:00Z",
            "items": [{ "id": "primary" }]
        }),
        egress.clone(),
    )
    .await;

    assert_eq!(calendars["body"]["items"][0]["id"], "primary");
    assert_eq!(calendars["body"]["items"][1]["id"], "team@example.com");
    assert_eq!(events["body"]["nextPageToken"], "CiAKGjBpNDd2Nm");
    assert_eq!(
        events["body"]["items"]
            .as_array()
            .expect("Calendar events response items is an array")
            .len(),
        2
    );
    assert_eq!(event["body"]["id"], "evt-standup-001");
    assert_eq!(
        free_busy["body"]["calendars"]["primary"]["busy"][0]["start"],
        "2026-05-21T10:00:00Z"
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.ends_with("/users/me/calendarList"));
    assert!(requests[1].url.contains("/calendars/primary/events"));
    assert!(requests[1].url.contains("timeMin=2026-05-21T00%3A00%3A00Z"));
    assert!(requests[1].url.contains("maxResults=50"));
    assert!(requests[2].url.ends_with("/events/evt-standup-001"));
    assert_eq!(requests[3].method, NetworkMethod::Post);
    assert!(requests[3].url.ends_with("/freeBusy"));
}

#[tokio::test]
async fn calendar_write_handlers_use_recorded_google_api_shapes() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![provider_scope(ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE)],
    )
    .await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(200, fixture("calendar", "event_created.json")),
        RecordingEgress::json_status(200, fixture("calendar", "event_created.json")),
        RecordingEgress::empty(204),
        RecordingEgress::json_status(200, fixture("calendar", "event_get.json")),
        RecordingEgress::json_status(200, fixture("calendar", "event_created.json")),
        RecordingEgress::json_status(200, fixture("calendar", "event_created.json")),
    ]));

    let created = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_CREATE_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event": { "summary": "Project review" } }),
        egress.clone(),
    )
    .await;
    let updated = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "event": { "summary": "Updated review" }
        }),
        egress.clone(),
    )
    .await;
    let deleted = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_DELETE_EVENT_CAPABILITY_ID,
        json!({ "calendar_id": "primary", "event_id": "evt-001" }),
        egress.clone(),
    )
    .await;
    let attendees_added = dispatch_ok(
        auth.clone(),
        scope.clone(),
        CALENDAR_ADD_ATTENDEES_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "attendees": [{ "email": "ada@example.com" }]
        }),
        egress.clone(),
    )
    .await;
    let reminders_set = dispatch_ok(
        auth,
        scope,
        CALENDAR_SET_REMINDER_CAPABILITY_ID,
        json!({
            "calendar_id": "primary",
            "event_id": "evt-001",
            "reminders": {
                "useDefault": false,
                "overrides": [{ "method": "popup", "minutes": 10 }]
            }
        }),
        egress.clone(),
    )
    .await;

    assert_eq!(created["body"]["id"], "evt-created-099");
    assert_eq!(updated["body"]["id"], "evt-created-099");
    assert_eq!(deleted["status"], 204);
    assert!(deleted["body"].is_null());
    assert_eq!(attendees_added["body"]["id"], "evt-created-099");
    assert_eq!(reminders_set["body"]["id"], "evt-created-099");
    let requests = egress.requests();
    assert_eq!(requests.len(), 6);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(requests[1].method, NetworkMethod::Patch);
    assert_eq!(requests[2].method, NetworkMethod::Delete);
    assert_eq!(requests[3].method, NetworkMethod::Get);
    assert_eq!(requests[4].method, NetworkMethod::Patch);
    assert_eq!(requests[5].method, NetworkMethod::Patch);
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body)
            .expect("parse Calendar create request body")["summary"],
        "Project review"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[4].body)
            .expect("parse Calendar attendees request body")["attendees"][0]["email"],
        "ada@example.com"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[5].body)
            .expect("parse Calendar reminders request body")["reminders"]["overrides"][0]["minutes"],
        10
    );
}

#[tokio::test]
async fn calendar_handler_preserves_insufficient_scope_response() {
    let scope = scope();
    let auth =
        auth_with_google_account(&scope, vec![provider_scope(GOOGLE_CALENDAR_READONLY_SCOPE)])
            .await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(403, fixture("calendar", "insufficient_scope.json")),
    ]));

    let output = dispatch_ok(
        auth,
        scope,
        CALENDAR_LIST_CALENDARS_CAPABILITY_ID,
        json!({}),
        egress.clone(),
    )
    .await;

    assert_eq!(output["status"], 403);
    assert_eq!(output["body"]["error"]["status"], "PERMISSION_DENIED");
    assert_eq!(
        output["body"]["error"]["details"][0]["reason"],
        "insufficient_scope"
    );
    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.ends_with("/users/me/calendarList"));
}

#[tokio::test]
async fn gmail_handlers_use_recorded_google_api_shapes() {
    let scope = scope();
    let auth = auth_with_google_account(
        &scope,
        vec![
            provider_scope(GOOGLE_GMAIL_READONLY_SCOPE),
            provider_scope(GOOGLE_GMAIL_SEND_SCOPE),
            provider_scope(GOOGLE_GMAIL_MODIFY_SCOPE),
        ],
    )
    .await;
    let egress = Arc::new(RecordingEgress::with_responses(vec![
        RecordingEgress::json_status(200, fixture("gmail", "messages_list.json")),
        RecordingEgress::json_status(200, fixture("gmail", "message_get.json")),
        RecordingEgress::json_status(200, fixture("gmail", "message_sent.json")),
        RecordingEgress::json_status(200, fixture("gmail", "draft_created.json")),
        RecordingEgress::json_status(200, fixture("gmail", "message_sent.json")),
        RecordingEgress::json_status(200, fixture("gmail", "message_trashed.json")),
    ]));

    let messages = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_LIST_MESSAGES_CAPABILITY_ID,
        json!({ "query": "is:unread from:ada", "max_results": 25 }),
        egress.clone(),
    )
    .await;
    let message = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_GET_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-001" }),
        egress.clone(),
    )
    .await;
    let sent = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_SEND_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822" } }),
        egress.clone(),
    )
    .await;
    let draft = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_CREATE_DRAFT_CAPABILITY_ID,
        json!({ "draft": { "message": { "raw": "base64url-rfc822" } } }),
        egress.clone(),
    )
    .await;
    let reply = dispatch_ok(
        auth.clone(),
        scope.clone(),
        GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
        json!({ "message": { "raw": "base64url-rfc822", "threadId": "thr-001" } }),
        egress.clone(),
    )
    .await;
    let trashed = dispatch_ok(
        auth,
        scope,
        GMAIL_TRASH_MESSAGE_CAPABILITY_ID,
        json!({ "message_id": "msg-001" }),
        egress.clone(),
    )
    .await;

    assert_eq!(messages["body"]["messages"][0]["id"], "msg-001");
    assert_eq!(message["body"]["id"], "msg-001");
    assert_eq!(sent["body"]["id"], "msg-sent-700");
    assert_eq!(draft["body"]["id"], "draft-501");
    assert_eq!(reply["body"]["id"], "msg-sent-700");
    assert_eq!(trashed["body"]["labelIds"][0], "TRASH");
    let requests = egress.requests();
    assert_eq!(requests.len(), 6);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(requests[0].url.contains("/users/me/messages"));
    assert!(requests[0].url.contains("q=is%3Aunread%20from%3Aada"));
    assert!(requests[0].url.contains("maxResults=25"));
    assert!(
        requests[1]
            .url
            .contains("/users/me/messages/msg-001?format=full")
    );
    assert!(requests[2].url.ends_with("/users/me/messages/send"));
    assert!(requests[3].url.ends_with("/users/me/drafts"));
    assert!(requests[4].url.ends_with("/users/me/messages/send"));
    assert!(
        requests[5]
            .url
            .ends_with("/users/me/messages/msg-001/trash")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[2].body).expect("parse Gmail send request body")
            ["raw"],
        "base64url-rfc822"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[3].body).expect("parse Gmail draft request body")
            ["message"]["raw"],
        "base64url-rfc822"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[4].body).expect("parse Gmail reply request body")
            ["threadId"],
        "thr-001"
    );
}
