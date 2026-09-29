//! Which tools one run advertises, as prompt text sees it.
//!
//! A prompt block that names a tool must not render on a run whose `tools`
//! array leaves that tool out (#7836). Before turn-start tool selection every
//! surface was either the whole visible catalog or progressive disclosure's
//! core set plus promotions, and prompt text was written against those
//! surfaces. A frozen turn-start selection advertises far less, so blocks
//! assembled once per runtime (the memory guidance, the protocol appendices,
//! the skill listing, the runtime-context lines) have to be told which tools
//! this run actually advertises.
//!
//! Gating is deliberately limited to selected runs. On an ordinary surface
//! every block renders exactly as it did before selection existed, so prompts
//! with selection off stay byte-identical.

use std::collections::BTreeSet;

use ironclaw_host_api::ids::CapabilityId;
use serde::{Deserialize, Serialize};

use crate::VisibleCapabilitySurface;

/// How a [`VisibleCapabilitySurface`]'s advertised `descriptors` were chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvertisedToolChoice {
    /// The ordinary surface: every visible tool, or progressive disclosure's
    /// core set plus promoted tools. Also what a run gets when turn-start
    /// selection is on but did not apply to it (no accepted user text, a
    /// classifier failure, an unreadable selection history).
    #[default]
    Ordinary,
    /// A conversation's frozen turn-start tool selection: `descriptors` is
    /// exactly what the run advertises, and prompt text may name only those
    /// tools.
    TurnStartSelection,
}

/// The tools one run advertises, for deciding which tool-naming prompt
/// blocks may render.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AdvertisedTools {
    /// The run advertises an ordinary surface ([`AdvertisedToolChoice::Ordinary`]):
    /// every tool-naming block renders as it always has.
    #[default]
    Ordinary,
    /// The run advertises a frozen turn-start selection: exactly these
    /// capabilities, discovery bridges included.
    Selected(BTreeSet<CapabilityId>),
}

impl AdvertisedTools {
    /// The advertised tools of the run that produced `surface`.
    pub fn from_surface(surface: &VisibleCapabilitySurface) -> Self {
        match surface.advertised_choice {
            AdvertisedToolChoice::Ordinary => Self::Ordinary,
            AdvertisedToolChoice::TurnStartSelection => Self::Selected(
                surface
                    .descriptors
                    .iter()
                    .map(|descriptor| descriptor.capability_id.clone())
                    .collect(),
            ),
        }
    }

    /// Whether the run advertises a turn-start selection.
    pub fn is_selected(&self) -> bool {
        matches!(self, Self::Selected(_))
    }

    /// Whether a prompt block naming every capability in `capability_ids`
    /// may render: always on an ordinary surface, and on a selected one only
    /// when the selection advertises all of them.
    pub fn may_name(&self, capability_ids: &[&str]) -> bool {
        match self {
            Self::Ordinary => true,
            Self::Selected(advertised) => capability_ids.iter().all(|wanted| {
                advertised
                    .iter()
                    .any(|capability_id| capability_id.as_str() == *wanted)
            }),
        }
    }
}

#[cfg(test)]
mod tests;
