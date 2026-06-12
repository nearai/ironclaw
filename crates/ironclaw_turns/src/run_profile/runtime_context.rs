use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use crate::TurnRunOrigin;

/// Model-visible runtime context for one loop execution.
///
/// First slice carries only time. The #4149 plan adds capability posture,
/// scoped-path semantics, and subagent narrowing as additional fields
/// rendered into the same prompt section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRuntimeContext {
    /// Instant this loop execution started. Rendered at minute precision.
    pub loop_started_at_utc: DateTime<Utc>,
    /// Validated IANA timezone for the user (e.g. `chrono_tz::America::Los_Angeles`)
    /// when known. `None` means unknown; never a guessed host timezone.
    ///
    /// Invalid IANA names are rejected at the producer boundary — the type system
    /// guarantees that any `Some` value is a well-formed, parseable timezone.
    pub user_timezone: Option<Tz>,
    /// Channel, delivery, and run-origin state for this loop execution.
    /// `None` means this slice is not yet populated (behaves identically to #4795).
    pub communication: Option<CommunicationRuntimeContext>,
}

/// Connected channels known to the system for this user at loop start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectedChannelsState {
    Unknown,
    Known(Vec<ConnectedChannelSummary>),
}

/// Summary of a single connected channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedChannelSummary {
    pub name: String,
    pub authenticated: bool,
    pub active: bool,
}

/// Outbound delivery target configured for this user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryTargetState {
    Unknown,
    NoneSet,
    Set(DeliveryTargetSummary),
}

/// Summary of the configured delivery target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryTargetSummary {
    pub display_name: String,
    pub channel: String,
}

/// Communication runtime context: channels, delivery target, and run origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunicationRuntimeContext {
    pub connected_channels: ConnectedChannelsState,
    pub delivery_target: DeliveryTargetState,
    /// Whether outbound delivery tool names should appear in model guidance.
    pub delivery_tools_visible: bool,
    pub run_origin: Option<TurnRunOrigin>,
}

impl LoopRuntimeContext {
    pub fn render_model_content(&self) -> String {
        let utc = self.loop_started_at_utc.format("%Y-%m-%dT%H:%MZ");
        let local = self.user_timezone.map(|tz| {
            let local = self.loop_started_at_utc.with_timezone(&tz);
            format!("{} ({}, {})", utc, local.format("%H:%M %a"), tz.name())
        });
        let time_line = match local {
            Some(stamped) => format!(
                "Current date/time at loop start: {stamped}. This was captured when \
                 this loop started; for the precise current time use the time \
                 capability if it is visible."
            ),
            None => format!(
                "Current date/time at loop start: {utc}. The user's timezone is \
                 unknown - if local time matters, ask the user or use the time \
                 capability if it is visible."
            ),
        };

        let Some(comm) = &self.communication else {
            return time_line;
        };

        let mut parts = vec![time_line];

        // Connected channels line.
        let channels_line = match &comm.connected_channels {
            ConnectedChannelsState::Unknown => "Connected channels: unknown.".to_string(),
            ConnectedChannelsState::Known(channels) if channels.is_empty() => {
                "Connected channels: none.".to_string()
            }
            ConnectedChannelsState::Known(channels) => {
                let joined = channels
                    .iter()
                    .map(|ch| {
                        let auth = if ch.authenticated {
                            "authenticated"
                        } else {
                            "unauthenticated"
                        };
                        let active = if ch.active { "active" } else { "inactive" };
                        format!("{} ({auth}, {active})", ch.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Connected channels: {joined}.")
            }
        };
        parts.push(channels_line);

        // Outbound delivery target line.
        let delivery_line = match &comm.delivery_target {
            DeliveryTargetState::Unknown => "Outbound delivery target: unknown.".to_string(),
            DeliveryTargetState::NoneSet if comm.delivery_tools_visible => {
                "Outbound delivery target: none set. To deliver routine or trigger results \
                 to a channel, call builtin__outbound_delivery_targets_list, then \
                 builtin__outbound_delivery_target_set, before creating the routine or trigger."
                    .to_string()
            }
            DeliveryTargetState::NoneSet => "Outbound delivery target: none set.".to_string(),
            DeliveryTargetState::Set(summary) => format!(
                "Outbound delivery target: {} ({}) \u{2014} applies to all routine and \
                 trigger results for this user (single preference, not per-trigger).",
                summary.display_name, summary.channel
            ),
        };
        parts.push(delivery_line);

        // Run origin line (and optional ScheduledTrigger+NoneSet warning).
        if let Some(origin) = &comm.run_origin {
            let origin_line = match origin {
                TurnRunOrigin::WebUiChat => {
                    "Run origin: WebUI chat; replies render in this chat.".to_string()
                }
                TurnRunOrigin::ProductInbound { adapter } => format!(
                    "Run origin: inbound message via {adapter}; replies post back to that conversation."
                ),
                TurnRunOrigin::ScheduledTrigger => {
                    "Run origin: scheduled trigger fire.".to_string()
                }
            };
            parts.push(origin_line);

            if matches!(origin, TurnRunOrigin::ScheduledTrigger)
                && matches!(comm.delivery_target, DeliveryTargetState::NoneSet)
                && comm.delivery_tools_visible
            {
                parts.push(
                    "Warning: no delivery target is set \u{2014} this run's result will not be \
                     delivered. Set one with builtin__outbound_delivery_target_set."
                        .to_string(),
                );
            }
        }

        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn stamp() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 6, 11, 21, 32, 47)
            .unwrap()
    }

    fn time_only_ctx() -> LoopRuntimeContext {
        LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: None,
        }
    }

    #[test]
    fn renders_utc_and_local_when_timezone_known() {
        let tz: Tz = "America/Los_Angeles".parse().unwrap();
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: Some(tz),
            communication: None,
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("2026-06-11T21:32Z"),
            "minute-truncated UTC: {text}"
        );
        assert!(text.contains("14:32 Thu"), "local time + weekday: {text}");
        assert!(text.contains("America/Los_Angeles"), "{text}");
        assert!(text.contains("time capability"), "{text}");
        assert!(!text.contains(":47"), "seconds must be truncated: {text}");
    }

    #[test]
    fn renders_unknown_timezone_fallback() {
        let ctx = time_only_ctx();
        let text = ctx.render_model_content();
        assert!(text.contains("2026-06-11T21:32Z"), "{text}");
        assert!(text.contains("timezone is unknown"), "{text}");
        assert!(text.contains("ask the user"), "{text}");
    }

    // Note: the previous `invalid_timezone_falls_back_to_unknown` test is no longer
    // applicable. `user_timezone` is now `Option<chrono_tz::Tz>` — invalid IANA names
    // are rejected at the producer boundary at parse time, by construction. There is no
    // runtime fallback to exercise; misuse is a compile error.

    #[test]
    fn communication_none_renders_identical_to_time_only_baseline() {
        // Verifies that adding communication: None does not change the rendered
        // output compared to the original #4795 time-only behavior.
        let ctx_with_none = time_only_ctx();
        let ctx_pre_4828 = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: None,
        };
        assert_eq!(
            ctx_with_none.render_model_content(),
            ctx_pre_4828.render_model_content(),
            "communication: None must not alter the output"
        );
        let text = ctx_with_none.render_model_content();
        assert!(
            !text.contains("Connected channels"),
            "no channel line when communication is None: {text}"
        );
        assert!(
            !text.contains("Outbound delivery"),
            "no delivery line when communication is None: {text}"
        );
        assert!(
            !text.contains("Run origin"),
            "no origin line when communication is None: {text}"
        );
    }

    #[test]
    fn renders_known_non_empty_channels() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Known(vec![
                    ConnectedChannelSummary {
                        name: "Slack".to_string(),
                        authenticated: true,
                        active: true,
                    },
                    ConnectedChannelSummary {
                        name: "Telegram".to_string(),
                        authenticated: false,
                        active: false,
                    },
                ]),
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Connected channels: Slack (authenticated, active), Telegram (unauthenticated, inactive)."),
            "{text}"
        );
    }

    #[test]
    fn renders_known_empty_channels() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Known(vec![]),
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(text.contains("Connected channels: none."), "{text}");
    }

    #[test]
    fn renders_unknown_channels() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(text.contains("Connected channels: unknown."), "{text}");
    }

    #[test]
    fn renders_delivery_none_set_with_tools_visible() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: true,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Outbound delivery target: none set. To deliver routine"),
            "{text}"
        );
        assert!(
            text.contains("builtin__outbound_delivery_targets_list"),
            "{text}"
        );
        assert!(
            text.contains("builtin__outbound_delivery_target_set"),
            "{text}"
        );
    }

    #[test]
    fn renders_delivery_none_set_without_tools_visible() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Outbound delivery target: none set."),
            "{text}"
        );
        assert!(
            !text.contains("builtin__outbound_delivery_targets_list"),
            "tool name must not appear when not visible: {text}"
        );
    }

    #[test]
    fn renders_delivery_set() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Set(DeliveryTargetSummary {
                    display_name: "#alerts".to_string(),
                    channel: "slack".to_string(),
                }),
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Outbound delivery target: #alerts (slack)"),
            "{text}"
        );
        assert!(
            text.contains("single preference, not per-trigger"),
            "{text}"
        );
    }

    #[test]
    fn renders_delivery_unknown() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: None,
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Outbound delivery target: unknown."),
            "{text}"
        );
    }

    #[test]
    fn renders_origin_web_ui_chat() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: Some(TurnRunOrigin::WebUiChat),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Run origin: WebUI chat; replies render in this chat."),
            "{text}"
        );
    }

    #[test]
    fn renders_origin_product_inbound() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::Unknown,
                delivery_tools_visible: false,
                run_origin: Some(TurnRunOrigin::ProductInbound {
                    adapter: "slack".to_string(),
                }),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains(
                "Run origin: inbound message via slack; replies post back to that conversation."
            ),
            "{text}"
        );
    }

    #[test]
    fn renders_origin_scheduled_trigger() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: false,
                run_origin: Some(TurnRunOrigin::ScheduledTrigger),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Run origin: scheduled trigger fire."),
            "{text}"
        );
    }

    #[test]
    fn scheduled_trigger_with_none_set_delivery_and_tools_visible_renders_warning() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: true,
                run_origin: Some(TurnRunOrigin::ScheduledTrigger),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Run origin: scheduled trigger fire."),
            "{text}"
        );
        assert!(
            text.contains("Warning: no delivery target is set"),
            "{text}"
        );
        assert!(
            text.contains("builtin__outbound_delivery_target_set"),
            "{text}"
        );
    }

    #[test]
    fn scheduled_trigger_with_none_set_delivery_no_tools_visible_omits_warning() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: false,
                run_origin: Some(TurnRunOrigin::ScheduledTrigger),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            text.contains("Run origin: scheduled trigger fire."),
            "{text}"
        );
        assert!(
            !text.contains("Warning: no delivery target is set"),
            "warning must not appear when delivery_tools_visible is false: {text}"
        );
    }

    #[test]
    fn web_ui_chat_with_none_set_delivery_and_tools_visible_does_not_render_warning() {
        // Only ScheduledTrigger triggers the warning, not WebUiChat.
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
            communication: Some(CommunicationRuntimeContext {
                connected_channels: ConnectedChannelsState::Unknown,
                delivery_target: DeliveryTargetState::NoneSet,
                delivery_tools_visible: true,
                run_origin: Some(TurnRunOrigin::WebUiChat),
            }),
        };
        let text = ctx.render_model_content();
        assert!(
            !text.contains("Warning: no delivery target is set"),
            "warning must not fire for WebUiChat: {text}"
        );
    }
}
