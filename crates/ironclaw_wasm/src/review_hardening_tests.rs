#[cfg(test)]
mod review_hardening_tests {
    use super::*;

    #[test]
    fn runtime_store_data_uses_one_invocation_wide_host_import_deadline() {
        let mut store = RuntimeStoreData::new(1024, Duration::from_millis(50), None, None);

        let first = store.remaining_host_import_timeout().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        let second = store.remaining_host_import_timeout().unwrap();

        assert!(
            second < first,
            "host import timeout budget must not reset per import"
        );
        assert!(!store.host_import_timed_out);
    }

    #[test]
    fn runtime_store_data_fails_subsequent_imports_after_timeout() {
        let mut store = RuntimeStoreData::new(1024, Duration::from_millis(1), None, None);

        std::thread::sleep(Duration::from_millis(5));

        assert_eq!(store.remaining_host_import_timeout(), None);
        assert!(store.host_import_timed_out);
        assert_eq!(store.remaining_host_import_timeout(), None);
    }

    #[test]
    fn epoch_ticker_thread_does_not_own_shutdown_state() {
        let engine = Engine::default();
        let ticker = spawn_epoch_ticker(engine, Duration::from_millis(1))
            .unwrap()
            .unwrap();
        let weak = Arc::downgrade(&ticker._state);

        drop(ticker);
        std::thread::sleep(Duration::from_millis(5));

        assert!(weak.upgrade().is_none());
    }
}
