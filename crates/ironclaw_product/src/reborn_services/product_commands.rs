//! WebUI command palette: audience-aware `product.commands.list` /
//! `product.commands.execute` facade methods.
//!
//! This is the WebUI door's counterpart to the channel command door
//! (`ProductCommandAdmissionService` + the channel run-delivery observer's
//! `InvalidRequest` -> help-text behavior): it must enforce the same
//! `required_audience`/`CommandAudience` policy so a non-admin caller cannot
//! see or execute an admin-only command through the browser just because the
//! channel-side admission gate does not apply here.

use ironclaw_extension_contracts::state::{InstallationState, LifecyclePublicState};

use crate::{
    LifecycleExtensionSummary, LifecyclePackageKind, LifecycleProductAction,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleReadinessBlocker,
    LifecycleSkillSummary,
};

use super::*;

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    /// Whether `caller` clears the command-admin authorization boundary.
    ///
    /// An env-bearer operator is an implicit admin (parity with
    /// `authorize_admin`). Otherwise the caller's persisted directory record
    /// is read on EVERY call (never cached) — a demoted/suspended admin loses
    /// command-admin access immediately, same contract as `authorize_admin`.
    async fn caller_is_command_admin(
        &self,
        caller: &ProductSurfaceCaller,
    ) -> Result<bool, ProductSurfaceError> {
        if caller.operator_config {
            return Ok(true);
        }
        match self
            .admin_users
            .get_user(&caller.tenant_id, &caller.user_id)
            .await
        {
            Ok(Some(record)) => {
                Ok(record.status == AdminUserStatus::Active && record.role.is_admin())
            }
            Ok(None) => Ok(false),
            // Same sanitized directory-error taxonomy `authorize_admin` uses:
            // `Unavailable` is retryable (503), everything else is not (never
            // silently "not admin" — that would let a demoted-admin race
            // widen access).
            Err(error) => Err(map_admin_user_error(error)),
        }
    }

    /// Registry entries visible to `is_admin`, in registry order (Lifecycle
    /// family first, then `model`/`status` — the order
    /// `product_command_descriptors` yields).
    fn visible_descriptors(is_admin: bool) -> impl Iterator<Item = ProductCommandDescriptor> {
        product_command_descriptors()
            .filter(move |descriptor| is_admin || descriptor.audience == CommandAudience::User)
    }

    /// Audience-filtered "Available commands:" help text — the same shape
    /// `declared_command_help_text` renders for an admission-declared set,
    /// but sourced from the full registry filtered to `is_admin`'s audience
    /// rather than a per-installation declared subset. Never includes an
    /// admin-only (including lifecycle) command name for a non-admin caller.
    fn caller_command_help_text(is_admin: bool) -> String {
        declared_command_help_text(
            Self::visible_descriptors(is_admin).map(|descriptor| descriptor.name),
        )
    }

    /// List the commands `caller`'s audience may see.
    pub(super) async fn list_product_commands(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornProductCommandListResponse, ProductSurfaceError> {
        let is_admin = self.caller_is_command_admin(&caller).await?;
        let commands = Self::visible_descriptors(is_admin)
            .map(|descriptor| RebornProductCommandInfo {
                name: descriptor.name.to_string(),
                title: descriptor.title.to_string(),
                description: descriptor.description.to_string(),
                usage: descriptor.usage.to_string(),
            })
            .collect();
        Ok(RebornProductCommandListResponse { commands })
    }

    /// Resolve (and memoize for this request) whether the caller clears the
    /// admin boundary. Callers pass a per-request slot; the directory is read
    /// at most once and only when a branch genuinely needs the answer.
    async fn resolve_admin_standing(
        &self,
        caller: &ProductSurfaceCaller,
        slot: &mut Option<bool>,
    ) -> Result<bool, ProductSurfaceError> {
        if let Some(known) = slot {
            return Ok(*known);
        }
        let resolved = self.caller_is_command_admin(caller).await?;
        *slot = Some(resolved);
        Ok(resolved)
    }

    /// Parse and execute one slash-command line on behalf of `caller`.
    ///
    /// Every parse-stage failure (ordinary text, an empty/malformed slash
    /// line, or a `ProductCommand::from_payload` argument rejection) becomes a
    /// role-filtered `InvalidRequest` help response — the underlying
    /// rejection's internal reason is never surfaced on the wire (leak rule,
    /// matching the channel observer's `InvalidRequest` -> help-text
    /// behavior), though the sanitized cause is logged server-side at debug
    /// level before it's mapped away. An Admin-audience command is rejected
    /// with a fixed `AccessDenied` message before its handler ever runs when
    /// the caller is not a command admin — this is the ONLY gate on the
    /// Lifecycle family: once an admin clears it, `ProductCommand::Lifecycle`
    /// executes through `lifecycle_service.execute(..)` (the same call the
    /// `product.lifecycle.command` capability op and the channel command door
    /// use) and its response is shaped into a `CommandResultView`.
    /// `ProductCommand::Unknown` keeps the fixed role-filtered help-text
    /// rejection — an unrecognized token is never executable.
    ///
    /// The caller's admin standing is resolved LAZILY and at most once per
    /// request: only branches that actually need it (the audience gate for an
    /// admin-audience command, and role-filtered help text) pay for the
    /// directory lookup, and the first lookup memoizes for the rest of the
    /// call. Never cached across requests.
    ///
    /// The laziness is load-bearing, not an optimization: `caller_is_command_admin`
    /// surfaces a degraded directory as a retryable 503, so resolving it
    /// unconditionally made user-audience commands (`/status`, bare `/model`)
    /// fail wherever the admin directory is unwired or unhealthy — exactly the
    /// `RejectingAdminUserService` default composition. User commands must not
    /// depend on the admin directory being up.
    pub(super) async fn execute_product_command(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornExecuteProductCommandRequest,
    ) -> Result<RebornExecuteProductCommandResponse, ProductSurfaceError> {
        let mut admin_standing: Option<bool> = None;

        let payload =
            match parse_product_slash_command(&request.text, ProductTriggerReason::DirectChat) {
                Ok(Some(payload)) => payload,
                Ok(None) => {
                    return Ok(Self::invalid_request_response(
                        String::new(),
                        Self::caller_command_help_text(
                            self.resolve_admin_standing(&caller, &mut admin_standing)
                                .await?,
                        ),
                    ));
                }
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "product command parse rejected; returning role-filtered help text"
                    );
                    return Ok(Self::invalid_request_response(
                        String::new(),
                        Self::caller_command_help_text(
                            self.resolve_admin_standing(&caller, &mut admin_standing)
                                .await?,
                        ),
                    ));
                }
            };
        let command_name = payload.command.clone();
        let command = match ProductCommand::from_payload(&payload) {
            Ok(command) => command,
            Err(rejection) => {
                // `rejection.reason` is a `RedactedString` — its Display/Debug
                // both always print the fixed placeholder, by design (never
                // even a debug-log leak). `kind`/`disposition` are safe
                // categorical enums that still say *which* validation failed.
                tracing::debug!(
                    command = %command_name,
                    kind = ?rejection.kind,
                    disposition = ?rejection.disposition(),
                    "product command argument rejection; returning role-filtered help text"
                );
                return Ok(Self::invalid_request_response(
                    command_name,
                    Self::caller_command_help_text(
                        self.resolve_admin_standing(&caller, &mut admin_standing)
                            .await?,
                    ),
                ));
            }
        };

        if required_audience(&command) == CommandAudience::Admin
            && !self
                .resolve_admin_standing(&caller, &mut admin_standing)
                .await?
        {
            return Ok(RebornExecuteProductCommandResponse {
                command: command_name,
                result: None,
                rejection: Some(RebornCommandRejection {
                    kind: ProductRejectionKind::AccessDenied,
                    message: "This command requires an admin account.".to_string(),
                }),
            });
        }

        match command {
            ProductCommand::Model { action } => {
                let result = self.execute_product_model_command(caller, action).await?;
                Ok(RebornExecuteProductCommandResponse {
                    command: command_name,
                    result: Some(result),
                    rejection: None,
                })
            }
            ProductCommand::Status => {
                let result = self
                    .execute_product_status_command(
                        caller,
                        ProductStatusCommandInput {
                            thread_id: request.thread_id,
                        },
                    )
                    .await?;
                Ok(RebornExecuteProductCommandResponse {
                    command: command_name,
                    result: Some(result),
                    rejection: None,
                })
            }
            // The audience gate above already fenced this to an admin caller
            // (Lifecycle is `CommandAudience::Admin`). Run the same lifecycle
            // execution the channel command door and the
            // `product.lifecycle.command` capability op use, then shape the
            // result into a `CommandResultView` for the generic renderer.
            ProductCommand::Lifecycle { action } => {
                let result = self
                    .execute_product_lifecycle_command(caller, action)
                    .await?;
                Ok(RebornExecuteProductCommandResponse {
                    command: command_name,
                    result: Some(result),
                    rejection: None,
                })
            }
            // An unrecognized command name is never executable, admin or
            // not — it keeps the fixed role-filtered help-text rejection.
            ProductCommand::Unknown { .. } => Ok(Self::invalid_request_response(
                command_name,
                Self::caller_command_help_text(
                    self.resolve_admin_standing(&caller, &mut admin_standing)
                        .await?,
                ),
            )),
        }
    }

    /// Execute one Lifecycle action and shape its `LifecycleProductResponse`
    /// into the channel-neutral `CommandResultView` the generic renderer
    /// displays. `lifecycle_service.execute` already returns the sanitized
    /// `ProductSurfaceError` boundary taxonomy, so its error propagates
    /// through `?` unchanged — no backend string or internal detail is
    /// mapped or added here.
    async fn execute_product_lifecycle_command(
        &self,
        caller: ProductSurfaceCaller,
        action: LifecycleProductAction,
    ) -> Result<CommandResultView, ProductSurfaceError> {
        let title = lifecycle_command_title(&action);
        let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: caller.tenant_id,
            user_id: caller.user_id,
            agent_id: caller.agent_id,
            project_id: caller.project_id,
        });
        let response = self.lifecycle_service.execute(context, action).await?;
        Ok(lifecycle_command_view(title, &response))
    }

    fn invalid_request_response(
        command: String,
        message: String,
    ) -> RebornExecuteProductCommandResponse {
        RebornExecuteProductCommandResponse {
            command,
            result: None,
            rejection: Some(RebornCommandRejection {
                kind: ProductRejectionKind::InvalidRequest,
                message,
            }),
        }
    }
}

/// The result-view title for one Lifecycle action: the same `title` the
/// palette's "Available commands" help text and `product.commands.list` use
/// for this command name (one source of truth — see
/// `product_command_descriptors`), so the header a caller sees on execution
/// matches the header they saw when listing. Falls back to the bare command
/// name on the (unreachable in practice) case that the registry has no entry
/// for this action's command name — never panics on a lookup miss.
fn lifecycle_command_title(action: &LifecycleProductAction) -> String {
    product_command_descriptors()
        .find(|descriptor| descriptor.name == action.command_name())
        .map(|descriptor| descriptor.title.to_string())
        .unwrap_or_else(|| action.command_name().to_string())
}

/// Shape one Lifecycle action's `LifecycleProductResponse` into a
/// `CommandResultView`, dispatched on the response's own `payload` shape
/// (not the requested `action`) so a payload/action mismatch from a
/// non-conforming `LifecycleProductService` degrades to the generic
/// confirmation view instead of panicking.
///
/// Two action families render differently, per the PR-2 spec:
/// - List/search (`extension_list`, `extension_search`, `skill_search`)
///   render as a `Count` field plus one readable row per result in `lines` —
///   never a JSON dump of the payload.
/// - Mutations (`install`/`remove`/`activate`/`configure`/`auth`, and the
///   skill equivalents) render a confirmation: which package, whether the
///   mutation itself succeeded, and the resulting public lifecycle state.
///
/// Every variant reports [`LifecyclePublicState`], never the raw internal
/// [`InstallationState`] checkpoint — `ironclaw_extension_contracts::state` documents
/// that a product surface must not expose those checkpoints directly.
fn lifecycle_command_view(title: String, response: &LifecycleProductResponse) -> CommandResultView {
    match &response.payload {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, count }) => {
            lifecycle_rows_view(
                title,
                *count,
                extensions
                    .iter()
                    .map(|entry| extension_row(&entry.summary, entry.installation_phase))
                    .collect(),
                "No extensions matched.",
            )
        }
        Some(LifecycleProductPayload::ExtensionList { extensions, count }) => lifecycle_rows_view(
            title,
            *count,
            extensions
                .iter()
                .map(|entry| extension_row(&entry.summary, Some(entry.phase)))
                .collect(),
            "No extensions installed yet.",
        ),
        Some(LifecycleProductPayload::SkillSearch {
            skills,
            count,
            limit,
            truncated,
        }) => {
            let mut view = lifecycle_rows_view(
                title,
                *count,
                skills.iter().map(skill_row).collect(),
                "No skills matched.",
            );
            if *truncated {
                view.lines.push(format!(
                    "Showing the first {limit} results; refine the search to see more."
                ));
            }
            view
        }
        Some(LifecycleProductPayload::ExtensionInstall {
            installed,
            visible_capability_ids,
            next_step,
        }) => {
            let mut fields = package_ref_fields(response);
            fields.push(command_result_field("Installed", yes_no(*installed)));
            let mut lines = capability_lines(visible_capability_ids);
            if !next_step.is_empty() {
                lines.push(next_step.clone());
            }
            lifecycle_confirmation_view(title, response, fields, lines)
        }
        Some(LifecycleProductPayload::ExtensionActivate {
            activated,
            visible_capability_ids,
            connection_required,
        }) => {
            let mut fields = package_ref_fields(response);
            fields.push(command_result_field("Activated", yes_no(*activated)));
            let mut lines = capability_lines(visible_capability_ids);
            if let Some(requirement) = connection_required {
                lines.push(format!(
                    "{}: {}",
                    requirement.display_name, requirement.instructions
                ));
            }
            lifecycle_confirmation_view(title, response, fields, lines)
        }
        Some(LifecycleProductPayload::ExtensionRemove { removed }) => {
            let mut fields = package_ref_fields(response);
            fields.push(command_result_field("Removed", yes_no(*removed)));
            lifecycle_confirmation_view(title, response, fields, Vec::new())
        }
        Some(LifecycleProductPayload::SkillInstall { installed, name }) => {
            let fields = vec![
                command_result_field("Skill", name.as_str()),
                command_result_field("Installed", yes_no(*installed)),
            ];
            lifecycle_confirmation_view(title, response, fields, Vec::new())
        }
        Some(LifecycleProductPayload::SkillRemove { removed, name }) => {
            let fields = vec![
                command_result_field("Skill", name.as_str()),
                command_result_field("Removed", yes_no(*removed)),
            ];
            lifecycle_confirmation_view(title, response, fields, Vec::new())
        }
        // `extension_auth` / `extension_configure` carry no dedicated payload
        // variant — the envelope (`package_ref` + `phase` + `blockers` +
        // `message`) is the whole story. Any service answering with a bare
        // `LifecycleProductResponse::projection(..)` (the
        // `UnsupportedLifecycleProductService` default, or a
        // partially-implemented backend) also lands here — never panics on
        // an unexpected shape.
        None => {
            let fields = package_ref_fields(response);
            lifecycle_confirmation_view(title, response, fields, Vec::new())
        }
    }
}

/// List/search shaping shared by `extension_search`, `extension_list`, and
/// `skill_search`: a `Count` field plus one row per line, or a single
/// human-readable line when there are no rows (never an empty `lines: []`,
/// which would render as a bare title with no explanation).
fn lifecycle_rows_view(
    title: String,
    count: usize,
    rows: Vec<String>,
    empty_message: &'static str,
) -> CommandResultView {
    let lines = if rows.is_empty() {
        vec![empty_message.to_string()]
    } else {
        rows
    };
    CommandResultView {
        title,
        fields: vec![command_result_field("Count", count.to_string())],
        lines,
    }
}

/// Mutation-confirmation shaping shared by every non-list Lifecycle action:
/// appends the resulting public `State`, then the service-authored
/// `message` (when present) and a plain-language line per readiness
/// blocker — never the blocker's internal `ref_id` diagnostic string.
fn lifecycle_confirmation_view(
    title: String,
    response: &LifecycleProductResponse,
    mut fields: Vec<CommandResultField>,
    mut lines: Vec<String>,
) -> CommandResultView {
    fields.push(command_result_field(
        "State",
        LifecyclePublicState::from_host_checkpoint(response.phase).as_str(),
    ));
    let mut all_lines = match response.message.as_deref() {
        Some(message) if !message.is_empty() => vec![message.to_string()],
        _ => Vec::new(),
    };
    all_lines.append(&mut lines);
    all_lines.extend(response.blockers.iter().map(blocker_line));
    CommandResultView {
        title,
        fields,
        lines: all_lines,
    }
}

/// The package-identity field for a mutation response, when the service
/// echoed one back (`extension_install`/`activate`/`remove`/`auth`/
/// `configure` always target one `LifecyclePackageRef`; the skill actions
/// carry their identity in the payload's own `name` field instead, so this
/// contributes nothing for those and callers add a `"Skill"` field
/// themselves).
fn package_ref_fields(response: &LifecycleProductResponse) -> Vec<CommandResultField> {
    response
        .package_ref
        .as_ref()
        .map(|package_ref| {
            vec![command_result_field(
                package_kind_label(package_ref.kind),
                package_ref.id.as_str(),
            )]
        })
        .unwrap_or_default()
}

fn package_kind_label(kind: LifecyclePackageKind) -> &'static str {
    match kind {
        LifecyclePackageKind::Extension => "Extension",
        LifecyclePackageKind::Skill => "Skill",
        LifecyclePackageKind::Mcp => "MCP",
        LifecyclePackageKind::Wasm => "Wasm",
    }
}

/// A short, plain-language line per readiness blocker. Deliberately omits
/// the blocker's `ref_id` — that is an internal diagnostic/correlation
/// string (e.g. `"extension_lifecycle_store_unwired"`), not user-facing
/// copy, matching this crate's rule against leaking backend detail into a
/// product-surface response.
fn blocker_line(blocker: &LifecycleReadinessBlocker) -> String {
    match blocker {
        LifecycleReadinessBlocker::Setup { .. } => {
            "Needs additional setup before it can activate.".to_string()
        }
        LifecycleReadinessBlocker::Auth { .. } => {
            "Needs the extension's account connected.".to_string()
        }
        LifecycleReadinessBlocker::Pairing { .. } => {
            "Needs channel pairing to finish connecting.".to_string()
        }
        LifecycleReadinessBlocker::Approval { .. } => {
            "Needs approval before it can activate.".to_string()
        }
        LifecycleReadinessBlocker::Policy { .. } => "Blocked by policy.".to_string(),
        LifecycleReadinessBlocker::Credential { .. } => {
            "Needs a credential configured.".to_string()
        }
        LifecycleReadinessBlocker::Runtime { .. } => "Not supported by this runtime.".to_string(),
    }
}

fn capability_lines(visible_capability_ids: &[String]) -> Vec<String> {
    if visible_capability_ids.is_empty() {
        Vec::new()
    } else {
        vec![format!("Tools: {}", visible_capability_ids.join(", "))]
    }
}

/// One readable row for an extension search/list result: id, name, version,
/// and (when the caller supplied a checkpoint) its public lifecycle state —
/// collapsed through [`LifecyclePublicState::from_host_checkpoint`], never
/// the raw [`InstallationState`] variant name.
fn extension_row(summary: &LifecycleExtensionSummary, phase: Option<InstallationState>) -> String {
    let state = phase
        .map(LifecyclePublicState::from_host_checkpoint)
        .map(|state| format!(" [{}]", state.as_str()))
        .unwrap_or_default();
    format!(
        "- {} — {} (v{}){state}",
        summary.package_ref.id.as_str(),
        summary.name,
        summary.version
    )
}

/// One readable row for a skill search result.
fn skill_row(skill: &LifecycleSkillSummary) -> String {
    format!(
        "- {} (v{}) — {}",
        skill.name.as_str(),
        skill.version,
        skill.description
    )
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
