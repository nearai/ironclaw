//! WebUI command palette: audience-aware `product.commands.list` /
//! `product.commands.execute` facade methods.
//!
//! This is the WebUI door's counterpart to the channel command door
//! (`ProductCommandAdmissionService` + the channel run-delivery observer's
//! `InvalidRequest` -> help-text behavior): it must enforce the same
//! `required_audience`/`CommandAudience` policy so a non-admin caller cannot
//! see or execute an admin-only command through the browser just because the
//! channel-side admission gate does not apply here.

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
    /// the caller is not a command admin. The Lifecycle family stays
    /// listing-only (non-executable) here even for admins, per the PR-2 spec.
    ///
    /// `is_admin` is resolved once, here, and reused by every branch below
    /// that needs the caller's admin standing — never re-queried per branch.
    /// Still exactly one admin-directory lookup per request (never cached
    /// across requests).
    pub(super) async fn execute_product_command(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornExecuteProductCommandRequest,
    ) -> Result<RebornExecuteProductCommandResponse, ProductSurfaceError> {
        let is_admin = self.caller_is_command_admin(&caller).await?;

        let payload =
            match parse_product_slash_command(&request.text, ProductTriggerReason::DirectChat) {
                Ok(Some(payload)) => payload,
                Ok(None) => {
                    return Ok(Self::invalid_request_response(
                        String::new(),
                        Self::caller_command_help_text(is_admin),
                    ));
                }
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "product command parse rejected; returning role-filtered help text"
                    );
                    return Ok(Self::invalid_request_response(
                        String::new(),
                        Self::caller_command_help_text(is_admin),
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
                    Self::caller_command_help_text(is_admin),
                ));
            }
        };

        if required_audience(&command) == CommandAudience::Admin && !is_admin {
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
            // Lifecycle stays non-executable from the WebUI composer in this
            // PR — listing-only, even for an admin caller that just cleared
            // the audience gate above.
            ProductCommand::Lifecycle { .. } | ProductCommand::Unknown { .. } => {
                Ok(Self::invalid_request_response(
                    command_name,
                    Self::caller_command_help_text(is_admin),
                ))
            }
        }
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
