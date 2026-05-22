use super::*;

pub(crate) struct TestManager {
    pub(crate) inner: EventStreamManager,
    pub(crate) update_source: Arc<InMemoryProjectionUpdateSource>,
}

impl std::ops::Deref for TestManager {
    type Target = EventStreamManager;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub(crate) fn manager(scope: ProjectionScope) -> TestManager {
    manager_with_source(scope, Arc::new(InMemoryProjectionUpdateSource::new(8)))
}

pub(crate) fn manager_with_source(
    scope: ProjectionScope,
    update_source: Arc<InMemoryProjectionUpdateSource>,
) -> TestManager {
    TestManager {
        inner: EventStreamManager::new(
            Arc::new(FakeProjectionService::new(scope)),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::clone(&update_source),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        ),
        update_source,
    }
}

pub(crate) async fn assert_second_subscription_denied_by_admission(
    limits: ProjectionStreamLimits,
    first: ProjectionScope,
    second: ProjectionScope,
) {
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(first.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(limits)),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _first = manager
        .subscribe(subscribe_request_for_stream_user(first, None))
        .await
        .expect("first subscription admitted");
    let error = manager
        .subscribe(subscribe_request_for_stream_user(second, None))
        .await
        .expect_err("second subscription rejected by targeted admission limit");

    assert!(matches!(error, ProjectionStreamError::AdmissionDenied));
}
