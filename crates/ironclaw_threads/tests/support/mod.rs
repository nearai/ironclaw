use ironclaw_host_api::ThreadId;
use ironclaw_threads::{
    EnsureThreadRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    SessionThreadService, ThreadScope,
};

pub async fn seed_listable_threads(
    service: &impl SessionThreadService,
    owned_scope: &ThreadScope,
    other_scope: ThreadScope,
    prefix: &str,
) -> [String; 3] {
    let thread_ids = [
        format!("thread-{prefix}-a"),
        format!("thread-{prefix}-b"),
        format!("thread-{prefix}-c"),
    ];

    for thread_id in &thread_ids {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: owned_scope.clone(),
                thread_id: Some(ThreadId::new(thread_id.clone()).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(thread_id.clone()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }
    service
        .ensure_thread(EnsureThreadRequest {
            scope: other_scope,
            thread_id: Some(ThreadId::new(format!("thread-{prefix}-other")).unwrap()),
            created_by_actor_id: "actor-b".into(),
            title: Some("other".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    thread_ids
}

pub async fn assert_two_page_thread_listing(
    service: &impl SessionThreadService,
    scope: ThreadScope,
    expected_ids: [String; 3],
) {
    let first = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: Some(2),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(first.threads.len(), 2);
    assert!(first.next_cursor.is_some());

    let second = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: Some(2),
            cursor: first.next_cursor.clone(),
        })
        .await
        .unwrap();
    assert_eq!(second.threads.len(), 1);
    assert!(second.next_cursor.is_none());

    let mut actual_ids = listed_thread_ids(&first);
    actual_ids.extend(listed_thread_ids(&second));
    actual_ids.sort();

    let mut expected_ids = expected_ids.to_vec();
    expected_ids.sort();
    assert_eq!(actual_ids, expected_ids);
}

fn listed_thread_ids(response: &ListThreadsForScopeResponse) -> Vec<String> {
    response
        .threads
        .iter()
        .map(|thread| thread.thread_id.as_str().to_string())
        .collect()
}
