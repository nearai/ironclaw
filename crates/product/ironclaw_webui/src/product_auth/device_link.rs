//! Device-link route handlers.
//!
//! # Why these are their own routes, and why STATUS is not one of them
//!
//! A device link **is** an `AuthFlowRecord`, so the generic flow-status route
//! already serves it — and §8.12's additive frame rides that response, which
//! is what lets a re-rendered card (a refresh, a second tab, a re-opened
//! settings pane) hydrate without disturbing the live link.
//!
//! The four routes here exist because their *operations* differ:
//!
//! * **start** takes a link mode (scan a code vs use a number); OAuth start
//!   builds an authorize URL.
//! * **poll** makes a **vendor call**. This is the one that departs from the
//!   frontend module's original plan, which had the card poll the read-only
//!   status route. It cannot: a device link only advances when the host asks
//!   the vendor whether the code was accepted (PROPOSAL §4.2 — acceptance is
//!   poll-driven re-export), nothing else drives it, and a card polling a pure
//!   read would wait forever on a QR that was already scanned. Making the
//!   shared GET advance a flow would also have hidden a vendor call behind a
//!   route descriptor declared read-shaped. The host's own poll floor keeps
//!   this cheap: a too-early poll is answered without the adapter being called
//!   at all.
//! * **input** carries a typed kind plus the step revision it was typed
//!   against, so a stale card cannot overwrite newer state.
//! * **cancel** must ask the vendor to log the device out — an
//!   accepted-but-abandoned link otherwise leaves an orphan authorization on
//!   the user's account (PROPOSAL §4.3). Nothing existing does that.
//!
//! Every handler is thin by charter: parse, derive scope from the
//! authenticated caller, and delegate. The step machine, the revision
//! compare-and-swap, the TTLs, and the rate limits are all `ironclaw_auth`'s.

use super::*;

/// The driver, or the explicit unavailable answer.
///
/// Mirrors [`ProductAuthRouteState::auth_engine`]: a deployment that composed
/// no device-link driver says so with a 503 rather than 404-ing as if the
/// flow were unknown.
fn driver(
    state: &ProductAuthRouteState,
) -> Result<Arc<ironclaw_auth::DeviceLinkFlowDriver>, ProductAuthRouteFailure> {
    state
        .product_auth
        .device_link_driver()
        .ok_or_else(ProductAuthRouteFailure::backend_unavailable)
}

/// Project a driver outcome into the wire response every device-link route
/// returns. One builder so the four routes cannot drift.
fn response(record: AuthFlowRecord) -> Json<DeviceLinkFlowResponse> {
    Json(DeviceLinkFlowResponse {
        flow_id: record.id,
        // Read from the stored record's own scope, never from the request:
        // this is the value a follow-up call must send back for
        // `scope_matches` to hold, so it has to be the one the flow was
        // actually persisted with.
        invocation_id: record.scope.resource.invocation_id,
        status: record.status,
        device_link: ironclaw_auth::product_prompt::device_link_view_for_flow(&record),
    })
}

pub(super) async fn device_link_start_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<DeviceLinkStartBody>,
) -> Result<Json<DeviceLinkFlowResponse>, ProductAuthRouteFailure> {
    let driver = driver(&state)?;
    let provider = AuthProviderId::new(&request.provider)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let extension_id = ExtensionId::new(&request.extension_name)
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    // Lenient on scope: a first link has no prior invocation to carry.
    let scope = scope_from_authenticated_caller_parts(&caller, &request.scope)?;
    let continuation = manual_token::manual_token_continuation(
        request.run_id.as_deref(),
        request.gate_ref.as_deref(),
    )?;
    // A stale, mismatched, or unparseable resume id falls through to a fresh
    // link rather than failing — the driver re-checks ownership, provider,
    // extension, terminality, and expiry before it reuses anything.
    let resume = request
        .resume_flow_id
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok())
        .map(AuthFlowId::from_uuid);

    let record = run_with_backend_timeout(driver.start(ironclaw_auth::DeviceLinkStartRequest {
        scope,
        provider,
        extension_id,
        continuation,
        mode: request.mode.into(),
        resume,
    }))
    .await?;
    Ok(response(record))
}

pub(super) async fn device_link_poll_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<DeviceLinkFlowRefBody>,
) -> Result<Json<DeviceLinkFlowResponse>, ProductAuthRouteFailure> {
    let driver = driver(&state)?;
    let flow_id = parse_flow_id(&request.flow_id)?;
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let record = run_with_backend_timeout(driver.poll(&scope, flow_id)).await?;
    Ok(response(record))
}

pub(super) async fn device_link_input_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<DeviceLinkInputBody>,
) -> Result<Json<DeviceLinkFlowResponse>, ProductAuthRouteFailure> {
    let driver = driver(&state)?;
    let flow_id = parse_flow_id(&request.flow_id)?;
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    // Bound the paste at the boundary, before anything vendor-shaped sees it.
    // The identifier is not secret-shaped but is still a user's account
    // handle, so it travels through the same bounded wrapper and is exposed
    // once, here, into the typed input the driver validates again.
    let value = request
        .value
        .into_validated()
        .map_err(|_| ProductAuthRouteFailure::invalid_request())?;
    let input = match request.kind {
        DeviceLinkInputKindBody::Identifier => {
            DeviceLinkInput::Identifier(value.expose_secret().to_string())
        }
        DeviceLinkInputKindBody::Code => DeviceLinkInput::Code(value.into_secret()),
        DeviceLinkInputKindBody::Password => DeviceLinkInput::Password(value.into_secret()),
    };
    let record =
        run_with_backend_timeout(driver.submit_input(&scope, flow_id, request.revision, input))
            .await?;
    Ok(response(record))
}

pub(super) async fn device_link_cancel_handler(
    State(state): State<ProductAuthRouteState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Json(request): Json<DeviceLinkFlowRefBody>,
) -> Result<Json<DeviceLinkFlowResponse>, ProductAuthRouteFailure> {
    let driver = driver(&state)?;
    let flow_id = parse_flow_id(&request.flow_id)?;
    let scope =
        scope_from_authenticated_caller_parts_requiring_invocation(&caller, &request.scope)?;
    let record = run_with_backend_timeout(driver.cancel(&scope, flow_id)).await?;
    Ok(response(record))
}

fn parse_flow_id(value: &str) -> Result<AuthFlowId, ProductAuthRouteFailure> {
    Uuid::parse_str(value)
        .map(AuthFlowId::from_uuid)
        .map_err(|_| ProductAuthRouteFailure::malformed_callback())
}
