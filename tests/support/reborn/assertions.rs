use ironclaw_threads::{MessageKind, MessageStatus, ThreadMessageRecord};
use ironclaw_turns::{TurnEventKind, TurnLifecycleEvent, TurnStatus};

pub fn assert_event_order(events: &[TurnLifecycleEvent], expected: &[TurnEventKind]) {
    let mut search_from = 0;
    for expected_kind in expected {
        let offset = events[search_from..]
            .iter()
            .position(|event| event.kind == *expected_kind)
            .unwrap_or_else(|| {
                panic!(
                    "expected event kind {expected_kind:?} after index {search_from}, got {:?}",
                    events.iter().map(|event| &event.kind).collect::<Vec<_>>()
                )
            });
        search_from += offset + 1;
    }
}

pub fn assert_completed_lifecycle(events: &[TurnLifecycleEvent]) {
    let kinds = events.iter().map(|event| &event.kind).collect::<Vec<_>>();
    assert!(
        kinds.contains(&&TurnEventKind::Submitted),
        "submitted event missing: {kinds:?}"
    );
    assert!(
        kinds.contains(&&TurnEventKind::RunnerClaimed),
        "runner-claimed event missing: {kinds:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == TurnEventKind::Completed
                && event.status == TurnStatus::Completed),
        "completed event missing: {events:?}"
    );
}

pub fn assert_history_contains_user(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.status == MessageStatus::Submitted
                && message.content.as_deref() == Some(text)),
        "thread history should contain submitted user message {text:?}"
    );
}

pub fn assert_history_contains_assistant(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(text)),
        "thread history should contain finalized assistant reply {text:?}"
    );
}

pub fn assert_history_excludes(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .all(|message| message.content.as_deref() != Some(text)),
        "thread history should exclude message from another turn: {text:?}"
    );
}
