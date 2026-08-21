use super::*;

#[tokio::test]
async fn decorating_factory_applies_decorators_in_declared_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let inner = Arc::new(DecoratorTestPort {
        label: "inner",
        log: Arc::clone(&log),
    });
    let factory =
        DecoratingLoopCapabilityPortFactory::new(Arc::new(DecoratorTestFactory { port: inner }))
            .with_decorator(Arc::new(LoggingDecorator {
                label: "first",
                log: Arc::clone(&log),
            }))
            .with_decorator(Arc::new(LoggingDecorator {
                label: "second",
                log: Arc::clone(&log),
            }));

    let port = factory
        .create_capability_port(&loop_run_context(&execution_context("decorator-order")).await)
        .await
        .expect("decorated port");

    let error = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect_err("test inner port should fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(
        &*log.lock().expect("log lock"),
        &["second", "first", "inner"]
    );
}

#[tokio::test]
async fn decorating_factory_propagates_inner_error() {
    let decorate_calls = Arc::new(AtomicUsize::new(0));
    let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(FailingDecoratorFactory {
        error: AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "inner factory failed"),
    }))
    .with_decorator(Arc::new(NoopDecorator {
        decorate_calls: Arc::clone(&decorate_calls),
    }));

    let error = match factory
        .create_capability_port(&loop_run_context(&execution_context("decorator-error")).await)
        .await
    {
        Ok(_) => panic!("inner factory error should propagate"),
        Err(error) => error,
    };

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(error.safe_summary, "inner factory failed");
    assert_eq!(decorate_calls.load(Ordering::SeqCst), 0);
}
