use grammers_mtproto::authentication;
use grammers_tl_types::{self as tl, Deserializable};

use crate::session::Session;
use crate::transport;

/// Perform the full DH auth key exchange with a Telegram DC.
///
/// This drives the Sans-IO `grammers_mtproto::authentication` module over
/// HTTP transport. Four round trips:
///
/// 1. step1 -> ReqPqMulti -> server returns ResPq
/// 2. step2 -> ReqDhParams -> server returns ServerDhParams
/// 3. step3 -> SetClientDhParams -> server returns DhGen answer
/// 4. create_key -> produces auth_key, salt, time_offset
pub fn generate_auth_key(session: &mut Session) -> Result<(), String> {
    let dc_id = session.dc_id;

    // Step 1: generate nonce, send ReqPqMulti
    let (request, step1_data) =
        authentication::step1().map_err(|e| format!("auth step1 failed: {e}"))?;

    let response_bytes = transport::post_plain(dc_id, &request)?;
    let res_pq = tl::enums::ResPq::from_bytes(&response_bytes)
        .map_err(|e| format!("failed to parse ResPq: {e}"))?;

    // Step 2: factorize PQ, RSA encrypt, send ReqDhParams
    let (request, step2_data) =
        authentication::step2(step1_data, res_pq).map_err(|e| format!("auth step2 failed: {e}"))?;

    let response_bytes = transport::post_plain(dc_id, &request)?;
    let server_dh = tl::enums::ServerDhParams::from_bytes(&response_bytes)
        .map_err(|e| format!("failed to parse ServerDhParams: {e}"))?;

    // Step 3: compute DH g_b, send SetClientDhParams
    let (request, step3_data) = authentication::step3(step2_data, server_dh)
        .map_err(|e| format!("auth step3 failed: {e}"))?;

    let response_bytes = transport::post_plain(dc_id, &request)?;
    let dh_answer = tl::enums::SetClientDhParamsAnswer::from_bytes(&response_bytes)
        .map_err(|e| format!("failed to parse DhGenAnswer: {e}"))?;

    // Final: derive auth key from shared secret
    let finished = authentication::create_key(step3_data, dh_answer)
        .map_err(|e| format!("auth create_key failed: {e}"))?;

    session.set_auth_key(&finished.auth_key);
    session.first_salt = finished.first_salt;
    session.time_offset = finished.time_offset;
    session.initialized = true;

    Ok(())
}
