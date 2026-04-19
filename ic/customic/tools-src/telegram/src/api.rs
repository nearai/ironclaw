//! Telegram MTProto API implementation.
//!
//! Sends encrypted RPC requests directly to Telegram's data centers via
//! HTTP POST to `https://{dc}.web.telegram.org/apiw`. Uses grammers-mtproto
//! (Sans-IO) for message framing and encryption; no TDLib/TDLight needed.

use grammers_mtproto::mtp::Encrypted;
use grammers_tl_types::{self as tl, Deserializable, Serializable};

use crate::session::Session;
use crate::transport;
use crate::types::*;

/// Current TL layer. Must match grammers-tl-types.
const LAYER: i32 = 185;

/// Wrap a request in InvokeWithLayer + InitConnection for the first RPC.
///
/// Telegram requires the first request in a session to be wrapped in
/// initConnection so the server knows our client metadata.
fn wrap_init_connection(session: &Session, inner_bytes: Vec<u8>) -> Vec<u8> {
    let init = tl::functions::InitConnection {
        api_id: session.api_id,
        device_model: "WASM Sandbox".to_string(),
        system_version: "wasip2".to_string(),
        app_version: "0.1.0".to_string(),
        system_lang_code: "en".to_string(),
        lang_pack: String::new(),
        lang_code: "en".to_string(),
        proxy: None,
        params: None,
        query: inner_bytes,
    };

    tl::functions::InvokeWithLayer {
        layer: LAYER,
        query: init.to_bytes(),
    }
    .to_bytes()
}

/// Create an Encrypted MTP instance from session state.
fn make_mtp(session: &Session) -> Result<Encrypted, String> {
    let auth_key = session.auth_key_bytes()?;
    Ok(Encrypted::build()
        .time_offset(session.time_offset)
        .first_salt(session.first_salt)
        .finish(auth_key))
}

/// Send an encrypted RPC, wrapping in initConnection on first call.
fn rpc_call(
    mtp: &mut Encrypted,
    session: &Session,
    request_bytes: Vec<u8>,
    init_wrap: bool,
) -> Result<Vec<u8>, String> {
    let bytes = if init_wrap {
        wrap_init_connection(session, request_bytes)
    } else {
        request_bytes
    };
    transport::post_encrypted(mtp, session.dc_id, &bytes)
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

/// Send auth code to phone number.
pub fn send_code(session: &mut Session) -> Result<String, String> {
    let phone = session
        .phone_number
        .as_ref()
        .ok_or("phone_number not set in session")?
        .clone();

    let mut mtp = make_mtp(session)?;
    let request = tl::functions::auth::SendCode {
        phone_number: phone,
        api_id: session.api_id,
        api_hash: session.api_hash.clone(),
        settings: tl::enums::CodeSettings::Settings(tl::types::CodeSettings {
            allow_flashcall: false,
            current_number: false,
            allow_app_hash: false,
            allow_missed_call: false,
            allow_firebase: false,
            unknown_number: false,
            logout_tokens: None,
            token: None,
            app_sandbox: None,
        }),
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let sent = tl::enums::auth::SentCode::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse SentCode: {e}"))?;

    match sent {
        tl::enums::auth::SentCode::Code(code) => {
            session.phone_code_hash = Some(code.phone_code_hash.clone());
            Ok(serde_json::to_string(&LoginResult {
                status: "code_sent".into(),
                phone_code_hash: Some(code.phone_code_hash),
                message: Some(
                    "Verification code sent. Use submit_auth_code to complete login.".into(),
                ),
            })
            .unwrap_or_default())
        }
        tl::enums::auth::SentCode::Success(_) => {
            session.logged_in = true;
            Ok(serde_json::to_string(&LoginResult {
                status: "logged_in".into(),
                phone_code_hash: None,
                message: Some("Already logged in.".into()),
            })
            .unwrap_or_default())
        }
        tl::enums::auth::SentCode::PaymentRequired(_) => {
            Err("Telegram requires payment to send auth codes to this number.".into())
        }
    }
}

/// Complete login with the verification code.
pub fn sign_in(session: &mut Session, code: &str) -> Result<String, String> {
    let phone = session
        .phone_number
        .as_ref()
        .ok_or("phone_number not set, call login first")?
        .clone();
    let hash = session
        .phone_code_hash
        .as_ref()
        .ok_or("phone_code_hash not set, call login first")?
        .clone();

    let mut mtp = make_mtp(session)?;
    let request = tl::functions::auth::SignIn {
        phone_number: phone,
        phone_code_hash: hash,
        phone_code: Some(code.to_string()),
        email_verification: None,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;

    match tl::enums::auth::Authorization::from_bytes(&resp_bytes) {
        Ok(tl::enums::auth::Authorization::Authorization(auth)) => {
            session.logged_in = true;
            session.phone_code_hash = None;
            Ok(format_user_auth(&auth.user))
        }
        Ok(tl::enums::auth::Authorization::SignUpRequired(_)) => {
            Err("Account not registered. Sign up on a Telegram client first.".into())
        }
        Err(e) => Err(format!(
            "signIn failed (maybe 2FA required): {e}. \
             If you have 2FA enabled, use submit_2fa_password."
        )),
    }
}

/// Submit 2FA password using SRP protocol.
pub fn check_password(session: &mut Session, password: &str) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;

    // Get the current password info (SRP parameters).
    let request = tl::functions::account::GetPassword {}.to_bytes();
    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let pwd = tl::enums::account::Password::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Password: {e}"))?;

    let tl::enums::account::Password::Password(pwd) = pwd;

    let current_algo = pwd
        .current_algo
        .ok_or("no current_algo, 2FA might not be enabled")?;

    let srp_b = pwd.srp_b.ok_or("no srp_B in password response")?;
    let srp_id = pwd.srp_id.ok_or("no srp_id in password response")?;

    match current_algo {
        tl::enums::PasswordKdfAlgo::Sha256Sha256Pbkdf2Hmacsha512iter100000Sha256ModPow(algo) => {
            let mut a_bytes = vec![0u8; 256];
            getrandom::fill(&mut a_bytes).map_err(|e| format!("getrandom failed: {e}"))?;

            let (m1, g_a) = grammers_crypto::two_factor_auth::calculate_2fa(
                &algo.salt1,
                &algo.salt2,
                &algo.p,
                &algo.g,
                srp_b,
                a_bytes,
                password.as_bytes(),
            );

            let check_req = tl::functions::auth::CheckPassword {
                password: tl::enums::InputCheckPasswordSrp::Srp(tl::types::InputCheckPasswordSrp {
                    srp_id,
                    a: g_a.to_vec(),
                    m1: m1.to_vec(),
                }),
            }
            .to_bytes();

            let resp_bytes = rpc_call(&mut mtp, session, check_req, false)?;
            match tl::enums::auth::Authorization::from_bytes(&resp_bytes) {
                Ok(tl::enums::auth::Authorization::Authorization(auth)) => {
                    session.logged_in = true;
                    session.phone_code_hash = None;
                    Ok(format_user_auth(&auth.user))
                }
                Ok(tl::enums::auth::Authorization::SignUpRequired(_)) => {
                    Err("Unexpected sign-up required after 2FA".into())
                }
                Err(e) => Err(format!("2FA check failed: {e}")),
            }
        }
        tl::enums::PasswordKdfAlgo::Unknown => {
            Err("server returned unknown password KDF algorithm; client may be outdated".into())
        }
    }
}

// ---------------------------------------------------------------------------
// Read-only API methods
// ---------------------------------------------------------------------------

pub fn get_me(session: &Session) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let request = tl::functions::users::GetFullUser {
        id: tl::enums::InputUser::UserSelf,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let full = tl::enums::users::UserFull::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse UserFull: {e}"))?;

    let tl::enums::users::UserFull::Full(full) = full;

    for user_enum in &full.users {
        if let tl::enums::User::User(u) = user_enum {
            return Ok(serde_json::to_string(&UserInfo {
                id: u.id,
                first_name: u.first_name.clone().unwrap_or_default(),
                last_name: u.last_name.clone(),
                username: u.username.clone(),
                phone_number: u.phone.clone(),
            })
            .unwrap_or_default());
        }
    }
    Err("no user in response".into())
}

pub fn get_contacts(session: &Session) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let request = tl::functions::contacts::GetContacts { hash: 0 }.to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let contacts = tl::enums::contacts::Contacts::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Contacts: {e}"))?;

    match contacts {
        tl::enums::contacts::Contacts::Contacts(c) => {
            let users: Vec<UserInfo> = c
                .users
                .iter()
                .filter_map(|u| match u {
                    tl::enums::User::User(u) => Some(UserInfo {
                        id: u.id,
                        first_name: u.first_name.clone().unwrap_or_default(),
                        last_name: u.last_name.clone(),
                        username: u.username.clone(),
                        phone_number: u.phone.clone(),
                    }),
                    _ => None,
                })
                .collect();
            Ok(serde_json::to_string(&users).unwrap_or_default())
        }
        tl::enums::contacts::Contacts::NotModified => Ok("[]".into()),
    }
}

pub fn get_chats(session: &Session, limit: i32) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let request = tl::functions::messages::GetDialogs {
        exclude_pinned: false,
        folder_id: None,
        offset_date: 0,
        offset_id: 0,
        offset_peer: tl::enums::InputPeer::Empty,
        limit,
        hash: 0,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let dialogs = tl::enums::messages::Dialogs::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Dialogs: {e}"))?;

    let chats = extract_chats_from_dialogs(&dialogs);
    Ok(serde_json::to_string(&chats).unwrap_or_default())
}

pub fn get_messages(
    session: &Session,
    chat_id: i64,
    limit: i32,
    from_message_id: Option<i32>,
) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let peer = resolve_peer(chat_id);

    let request = tl::functions::messages::GetHistory {
        peer,
        offset_id: from_message_id.unwrap_or(0),
        offset_date: 0,
        add_offset: 0,
        limit,
        max_id: 0,
        min_id: 0,
        hash: 0,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let messages = tl::enums::messages::Messages::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Messages: {e}"))?;

    let msgs = extract_messages(&messages);
    Ok(serde_json::to_string(&msgs).unwrap_or_default())
}

pub fn send_message(session: &Session, chat_id: i64, text: &str) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let peer = resolve_peer(chat_id);

    let mut rng_buf = [0u8; 8];
    getrandom::fill(&mut rng_buf).map_err(|e| format!("getrandom: {e}"))?;
    let random_id = i64::from_le_bytes(rng_buf);

    let request = tl::functions::messages::SendMessage {
        no_webpage: false,
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        allow_paid_floodskip: false,
        peer,
        reply_to: None,
        message: text.to_string(),
        random_id,
        reply_markup: None,
        entities: None,
        schedule_date: None,
        send_as: None,
        quick_reply_shortcut: None,
        effect: None,
        allow_paid_stars: None,
        suggested_post: None,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let result =
        tl::enums::Updates::from_bytes(&resp_bytes).map_err(|e| format!("parse Updates: {e}"))?;

    match result {
        tl::enums::Updates::UpdateShortSentMessage(m) => Ok(serde_json::to_string(&SendResult {
            message_id: m.id,
            date: m.date,
        })
        .unwrap_or_default()),
        _ => Ok(serde_json::to_string(&SendResult {
            message_id: 0,
            date: 0,
        })
        .unwrap_or_default()),
    }
}

pub fn forward_message(
    session: &Session,
    from_chat_id: i64,
    to_chat_id: i64,
    message_ids: Vec<i32>,
) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let from_peer = resolve_peer(from_chat_id);
    let to_peer = resolve_peer(to_chat_id);

    let random_ids: Result<Vec<i64>, String> = message_ids
        .iter()
        .map(|_| {
            let mut buf = [0u8; 8];
            getrandom::fill(&mut buf).map_err(|e| format!("getrandom: {e}"))?;
            Ok(i64::from_le_bytes(buf))
        })
        .collect();

    let request = tl::functions::messages::ForwardMessages {
        silent: false,
        background: false,
        with_my_score: false,
        drop_author: false,
        drop_media_captions: false,
        noforwards: false,
        allow_paid_floodskip: false,
        from_peer,
        id: message_ids,
        random_id: random_ids?,
        to_peer,
        top_msg_id: None,
        reply_to: None,
        schedule_date: None,
        send_as: None,
        quick_reply_shortcut: None,
        video_timestamp: None,
        allow_paid_stars: None,
        suggested_post: None,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let _updates =
        tl::enums::Updates::from_bytes(&resp_bytes).map_err(|e| format!("parse Updates: {e}"))?;

    Ok(serde_json::to_string(&ForwardResult { ok: true }).unwrap_or_default())
}

pub fn delete_messages(
    session: &Session,
    message_ids: Vec<i32>,
    revoke: bool,
) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;
    let request = tl::functions::messages::DeleteMessages {
        revoke,
        id: message_ids,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let _affected = tl::enums::messages::AffectedMessages::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse AffectedMessages: {e}"))?;

    Ok(serde_json::to_string(&DeleteResult { ok: true }).unwrap_or_default())
}

pub fn search_messages(
    session: &Session,
    query: &str,
    chat_id: Option<i64>,
    limit: i32,
) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;

    let request = if let Some(cid) = chat_id {
        let peer = resolve_peer(cid);
        tl::functions::messages::Search {
            peer,
            q: query.to_string(),
            from_id: None,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: None,
            filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        }
        .to_bytes()
    } else {
        tl::functions::messages::SearchGlobal {
            broadcasts_only: false,
            groups_only: false,
            users_only: false,
            folder_id: None,
            q: query.to_string(),
            filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
            min_date: 0,
            max_date: 0,
            offset_rate: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            offset_id: 0,
            limit,
        }
        .to_bytes()
    };

    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let messages = tl::enums::messages::Messages::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Messages: {e}"))?;

    let msgs = extract_messages(&messages);
    Ok(serde_json::to_string(&msgs).unwrap_or_default())
}

pub fn get_updates(session: &Session) -> Result<String, String> {
    let mut mtp = make_mtp(session)?;

    let request = tl::functions::updates::GetState {}.to_bytes();
    let resp_bytes = rpc_call(&mut mtp, session, request, true)?;
    let state = tl::enums::updates::State::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse State: {e}"))?;

    let tl::enums::updates::State::State(s) = state;

    let request = tl::functions::updates::GetDifference {
        pts: s.pts.saturating_sub(10),
        pts_limit: None,
        pts_total_limit: None,
        date: s.date,
        qts: s.qts,
        qts_limit: None,
    }
    .to_bytes();

    let resp_bytes = rpc_call(&mut mtp, session, request, false)?;
    let diff = tl::enums::updates::Difference::from_bytes(&resp_bytes)
        .map_err(|e| format!("parse Difference: {e}"))?;

    let updates = extract_updates_from_diff(&diff);
    Ok(serde_json::to_string(&updates).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a successful auth response with user info.
fn format_user_auth(user: &tl::enums::User) -> String {
    match user {
        tl::enums::User::User(u) => serde_json::to_string(&AuthResult {
            status: "logged_in".into(),
            user: Some(UserInfo {
                id: u.id,
                first_name: u.first_name.clone().unwrap_or_default(),
                last_name: u.last_name.clone(),
                username: u.username.clone(),
                phone_number: u.phone.clone(),
            }),
            message: None,
        })
        .unwrap_or_default(),
        tl::enums::User::Empty(e) => serde_json::to_string(&AuthResult {
            status: "logged_in".into(),
            user: Some(UserInfo {
                id: e.id,
                first_name: "Unknown".into(),
                last_name: None,
                username: None,
                phone_number: None,
            }),
            message: None,
        })
        .unwrap_or_default(),
    }
}

/// Resolve a chat_id to an InputPeer. Negative IDs are channels/supergroups.
fn resolve_peer(chat_id: i64) -> tl::enums::InputPeer {
    if chat_id > 0 {
        tl::enums::InputPeer::User(tl::types::InputPeerUser {
            user_id: chat_id,
            access_hash: 0,
        })
    } else {
        let abs_id = chat_id.unsigned_abs() as i64;
        if abs_id > 1_000_000_000_000 {
            // Channel/supergroup: strip -100 prefix
            let channel_id = abs_id - 1_000_000_000_000;
            tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id,
                access_hash: 0,
            })
        } else {
            tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: abs_id })
        }
    }
}

fn extract_chats_from_dialogs(dialogs: &tl::enums::messages::Dialogs) -> Vec<ChatInfo> {
    let chats = match dialogs {
        tl::enums::messages::Dialogs::Dialogs(d) => &d.chats,
        tl::enums::messages::Dialogs::Slice(d) => &d.chats,
        tl::enums::messages::Dialogs::NotModified(_) => return vec![],
    };

    chats.iter().filter_map(chat_to_info).collect()
}

fn chat_to_info(chat: &tl::enums::Chat) -> Option<ChatInfo> {
    match chat {
        tl::enums::Chat::Chat(c) => Some(ChatInfo {
            id: -(c.id),
            chat_type: "group".into(),
            title: Some(c.title.clone()),
            username: None,
        }),
        tl::enums::Chat::Channel(c) => Some(ChatInfo {
            id: -(1_000_000_000_000 + c.id),
            chat_type: if c.megagroup { "supergroup" } else { "channel" }.into(),
            title: Some(c.title.clone()),
            username: c.username.clone(),
        }),
        tl::enums::Chat::Forbidden(c) => Some(ChatInfo {
            id: -(c.id),
            chat_type: "group".into(),
            title: Some(c.title.clone()),
            username: None,
        }),
        tl::enums::Chat::ChannelForbidden(c) => Some(ChatInfo {
            id: -(1_000_000_000_000 + c.id),
            chat_type: "channel".into(),
            title: Some(c.title.clone()),
            username: None,
        }),
        tl::enums::Chat::Empty(_) => None,
    }
}

fn extract_messages(msgs: &tl::enums::messages::Messages) -> Vec<MessageInfo> {
    let messages = match msgs {
        tl::enums::messages::Messages::Messages(m) => &m.messages,
        tl::enums::messages::Messages::Slice(m) => &m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => &m.messages,
        tl::enums::messages::Messages::NotModified(_) => return vec![],
    };

    messages.iter().filter_map(message_to_info).collect()
}

fn message_to_info(msg: &tl::enums::Message) -> Option<MessageInfo> {
    match msg {
        tl::enums::Message::Message(m) => Some(MessageInfo {
            message_id: m.id,
            date: m.date,
            from_user_id: m.from_id.as_ref().and_then(peer_id),
            text: Some(m.message.clone()),
            chat_id: Some(peer_id_value(&m.peer_id)),
        }),
        tl::enums::Message::Service(m) => Some(MessageInfo {
            message_id: m.id,
            date: m.date,
            from_user_id: m.from_id.as_ref().and_then(peer_id),
            text: Some("[service message]".into()),
            chat_id: Some(peer_id_value(&m.peer_id)),
        }),
        tl::enums::Message::Empty(_) => None,
    }
}

fn peer_id(peer: &tl::enums::Peer) -> Option<i64> {
    Some(peer_id_value(peer))
}

fn peer_id_value(peer: &tl::enums::Peer) -> i64 {
    match peer {
        tl::enums::Peer::User(p) => p.user_id,
        tl::enums::Peer::Chat(p) => -(p.chat_id),
        tl::enums::Peer::Channel(p) => -(1_000_000_000_000 + p.channel_id),
    }
}

fn extract_updates_from_diff(diff: &tl::enums::updates::Difference) -> Vec<UpdateInfo> {
    match diff {
        tl::enums::updates::Difference::Difference(d) => extract_update_list(&d.new_messages),
        tl::enums::updates::Difference::Slice(d) => extract_update_list(&d.new_messages),
        tl::enums::updates::Difference::Empty(_) => vec![],
        tl::enums::updates::Difference::TooLong(_) => {
            vec![UpdateInfo {
                update_type: "too_long".into(),
                message: None,
            }]
        }
    }
}

fn extract_update_list(messages: &[tl::enums::Message]) -> Vec<UpdateInfo> {
    messages
        .iter()
        .filter_map(|m| {
            message_to_info(m).map(|info| UpdateInfo {
                update_type: "new_message".into(),
                message: Some(info),
            })
        })
        .collect()
}
