use std::sync::OnceLock;

use tokio::sync::Mutex;

#[allow(dead_code)]
pub fn engine_v2_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
