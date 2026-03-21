#![cfg(feature = "libsql")]

use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{ConversationStore, Database};

#[tokio::test]
async fn notification_scope_and_consumption_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("routine_notifications.db");
    let backend = LibSqlBackend::new_local(&db_path).await?;
    backend.run_migrations().await?;

    let user_id = "test_user";
    let channel = "gateway";
    let scoped = "thread-123";

    let scoped_id = backend
        .add_conversation_notification(
            user_id,
            channel,
            Some(scoped),
            "routine",
            "routine-1",
            "Scoped notification",
            &serde_json::json!({"routine_name": "Scoped"}),
            None,
        )
        .await?;
    let global_id = backend
        .add_conversation_notification(
            user_id,
            channel,
            None,
            "routine",
            "routine-2",
            "Global notification",
            &serde_json::json!({"routine_name": "Global"}),
            None,
        )
        .await?;

    let scoped_notifications = backend
        .list_unread_conversation_notifications(user_id, channel, Some(scoped), 10)
        .await?;
    assert_eq!(scoped_notifications.len(), 2);
    assert_eq!(scoped_notifications[0].id, scoped_id);
    assert_eq!(scoped_notifications[1].id, global_id);

    let global_notifications = backend
        .list_unread_conversation_notifications(user_id, channel, None, 10)
        .await?;
    assert_eq!(global_notifications.len(), 1);
    assert_eq!(global_notifications[0].id, global_id);

    backend
        .mark_conversation_notifications_consumed(&[scoped_id])
        .await?;

    let scoped_notifications = backend
        .list_unread_conversation_notifications(user_id, channel, Some(scoped), 10)
        .await?;
    assert_eq!(scoped_notifications.len(), 1);
    assert_eq!(scoped_notifications[0].id, global_id);

    Ok(())
}
