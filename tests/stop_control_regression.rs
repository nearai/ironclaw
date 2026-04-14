use ironclaw::agent::session::{Thread, ThreadState};
use uuid::Uuid;

#[test]
fn interrupt_cancels_active_turn_and_marks_thread_interrupted() {
    let session_id = Uuid::new_v4();
    let mut thread = Thread::with_id(Uuid::new_v4(), session_id, Some("web"));

    let cancel = {
        let turn = thread.start_turn("please stop me");
        assert_eq!(turn.user_input, "please stop me");
        thread
            .current_turn_cancel()
            .expect("turn should expose a cancellation token")
    };

    assert!(!cancel.is_cancelled(), "token should start active");

    thread.interrupt();

    assert_eq!(thread.state, ThreadState::Interrupted);
    assert!(cancel.is_cancelled(), "interrupt should cancel the token");
    assert!(thread.current_turn_cancel().is_some());
}
