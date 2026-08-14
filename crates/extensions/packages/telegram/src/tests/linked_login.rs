//! Tests for the device-link adapter's pure decision surface: the payload the
//! card renders on every poll, and the identity shown after
//! completion. Error classification is pinned next to its table, in
//! `linked_login_errors.rs`.
//!
//! Everything that needs a socket is deliberately absent — the login handshake
//! is exercised against a live account by the manual QA journey, not here.
//! What *can* be pinned without Telegram is the part where a mistake is
//! invisible in manual testing: whether a terminal failure is reported as
//! restartable.

use super::pending::should_logout_on_abandon;
use super::*;

// ---------------------------------------------------------------------------
// Poll cadence and the rendered payload
// ---------------------------------------------------------------------------

#[test]
fn expiry_is_judged_against_the_servers_clock_not_the_local_one() {
    let local_now = local_unix_seconds();
    // The server is 120 s ahead of us. A token expiring at server-now + 10 must
    // read as 10 s left, not 130.
    let offset = 120;
    let expires = i32::try_from(local_now + offset + 10).expect("epoch fits in i32");
    let remaining = remaining_for(expires, offset);
    assert!(
        remaining <= Duration::from_secs(11) && remaining >= Duration::from_secs(9),
        "expected roughly 10s of server-time headroom, got {remaining:?}"
    );
}

#[test]
fn an_already_expired_token_yields_no_headroom_rather_than_underflowing() {
    let local_now = local_unix_seconds();
    let expires = i32::try_from(local_now - 60).expect("epoch fits in i32");
    assert_eq!(remaining_for(expires, 0), Duration::ZERO);
}

#[test]
fn the_rendered_payload_is_the_url_a_telegram_client_scans() {
    let payload = login_payload(&[0xde, 0xad, 0xbe, 0xef]).expect("payload");
    assert_eq!(payload.expose(), "tg://login?token=3q2-7w");

    // The token itself must be URL-safe and unpadded, or the client rejects
    // the link. `0xde 0xad 0xbe 0xef` is the case that proves it: standard
    // base64 renders it `3q2+7w==`, with both a `+` and padding.
    let token = payload
        .expose()
        .strip_prefix("tg://login?token=")
        .expect("the payload is a tg login link");
    assert!(!token.contains('+'));
    assert!(!token.contains('/'));
    assert!(!token.contains('='));
}

#[test]
fn a_re_export_of_the_same_token_keeps_painting_the_code_it_carries() {
    // Telegram returns the SAME bytes on every poll within a token's window,
    // so this is the ordinary case, not an edge one. It used to answer
    // `AwaitingVendor` — "nothing to show" — which blanked a still-valid QR
    // about one poll after painting it and parked the card on "waiting for the
    // vendor" forever: the code was never displayed again and no one could
    // scan it.
    let mut state = PendingState::default();
    let exported = tl::types::auth::LoginToken {
        expires: (local_unix_seconds() + 30) as i32,
        token: vec![0xde, 0xad, 0xbe, 0xef],
    };

    let first = paint_token(&mut state, exported.clone());
    let second = paint_token(&mut state, exported);

    for (label, step) in [("first", first), ("re-export", second)] {
        match step {
            DeviceLinkStep::Display { payload, .. } => {
                assert_eq!(
                    payload.expose(),
                    "tg://login?token=3q2-7w",
                    "{label} poll must paint the live code"
                );
            }
            other => panic!("{label} poll produced {other:?} instead of the code to scan"),
        }
    }
}

#[test]
fn the_payload_redacts_under_debug_because_it_is_the_login_token() {
    let payload = login_payload(&[1, 2, 3, 4]).expect("payload");
    let rendered = format!("{payload:?}");
    assert!(rendered.contains("redacted"));
    assert!(!rendered.contains("tg://login"));
}

// ---------------------------------------------------------------------------
// What the user is shown after completion
// ---------------------------------------------------------------------------

fn raw_user(username: Option<&str>, first: Option<&str>, last: Option<&str>) -> tl::enums::User {
    tl::enums::User::User(tl::types::User {
        is_self: true,
        contact: false,
        mutual_contact: false,
        deleted: false,
        bot: false,
        bot_chat_history: false,
        bot_nochats: false,
        verified: false,
        restricted: false,
        min: false,
        bot_inline_geo: false,
        support: false,
        scam: false,
        apply_min_photo: false,
        fake: false,
        bot_attach_menu: false,
        premium: false,
        attach_menu_enabled: false,
        bot_can_edit: false,
        close_friend: false,
        stories_hidden: false,
        stories_unavailable: false,
        contact_require_premium: false,
        bot_business: false,
        bot_has_main_app: false,
        bot_forum_view: false,
        bot_forum_can_manage_topics: false,
        bot_can_manage_bots: false,
        bot_guestchat: false,
        bot_guard: false,
        id: 4242,
        access_hash: Some(99),
        first_name: first.map(str::to_string),
        last_name: last.map(str::to_string),
        username: username.map(str::to_string),
        phone: None,
        photo: None,
        status: None,
        bot_info_version: None,
        restriction_reason: None,
        bot_inline_placeholder: None,
        lang_code: None,
        emoji_status: None,
        usernames: None,
        stories_max_id: None,
        color: None,
        profile_color: None,
        bot_active_users: None,
        bot_verification_icon: None,
        send_paid_messages_stars: None,
    })
}

#[test]
fn the_completion_label_prefers_a_username_and_the_reference_is_the_vendor_id() {
    let (label, reference) = identity(&raw_user(Some("ada"), Some("Ada"), Some("Lovelace")));
    assert_eq!(label, "@ada");
    assert_eq!(
        reference, "4242",
        "the reference is what makes a substituted login observable, so it must \
         be the vendor's stable identifier, not a display name"
    );
}

#[test]
fn a_user_without_a_username_falls_back_to_a_name_then_to_the_id() {
    let (named, _) = identity(&raw_user(None, Some("Ada"), Some("Lovelace")));
    assert_eq!(named, "Ada Lovelace");

    let (anonymous, reference) = identity(&raw_user(None, None, None));
    assert_eq!(
        anonymous, reference,
        "an empty label would fail DeviceLinkStep::validate, so it must never \
         be produced"
    );
    assert!(!anonymous.is_empty());
}

#[test]
fn the_self_peer_is_marked_as_self_even_when_the_vendor_shape_is_not() {
    let info = self_peer(&raw_user(Some("ada"), None, None));
    assert!(matches!(
        info,
        PeerInfo::User {
            id: 4242,
            is_self: Some(true),
            ..
        }
    ));
}

#[tokio::test]
async fn an_authorized_session_wins_over_a_replayed_signup_required_result() {
    let recovered = raw_user(Some("ada"), Some("Ada"), Some("Lovelace"));
    let resolution = resolve_post_sign_in_failure(PostSignInFailure::SignUpRequired, || async {
        Ok::<_, DeviceLinkError>(Some(recovered))
    })
    .await
    .expect("the authorization probe succeeds");

    assert!(matches!(
        resolution,
        PostSignInResolution::Authorized(user)
            if matches!(user.as_ref(), tl::enums::User::User(user) if user.id == 4242)
    ));
}

#[tokio::test]
async fn signup_required_is_terminal_only_when_the_session_is_not_authorized() {
    let resolution = resolve_post_sign_in_failure(PostSignInFailure::SignUpRequired, || async {
        Ok::<_, DeviceLinkError>(None)
    })
    .await
    .expect("the authorization probe succeeds");

    assert!(matches!(resolution, PostSignInResolution::Unregistered));
}

#[test]
fn a_completed_but_unfinalized_login_is_still_logged_out_when_abandoned() {
    let mut state = PendingState::default();
    state.accepted = true;
    state.phase = PendingPhase::Completed {
        account_label: "@ada".to_string(),
        vendor_user_ref: "4242".to_string(),
    };

    assert!(
        should_logout_on_abandon(&state),
        "custody persistence alone is not host acceptance; cancellation must still revoke the provisional device",
    );
}

// ---------------------------------------------------------------------------
// Attempt bounds
// ---------------------------------------------------------------------------

#[test]
fn a_flow_stops_accepting_input_after_a_bounded_number_of_attempts() {
    let mut state = PendingState::default();
    for _ in 0..MAX_INPUT_ATTEMPTS {
        state.charge_attempt().expect("within the attempt budget");
    }
    let exhausted = state
        .charge_attempt()
        .expect_err("the budget must be enforced");
    assert_eq!(exhausted.code(), DeviceLinkErrorCode::RateLimited);
    assert!(
        exhausted.restartable(),
        "exhausting attempts means start over, not give up forever"
    );
}
