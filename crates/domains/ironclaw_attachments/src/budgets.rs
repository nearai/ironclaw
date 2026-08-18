/// Attachment-count and decoded-byte budgets shared by every product surface.
///
/// Serde-capable so wire DTOs can embed it with `#[serde(flatten)]` instead of
/// re-declaring the same three fields and hand-copying them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentBudgets {
    pub max_count: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
}

/// Current WebUI-compatible attachment budgets.
pub const DEFAULT_ATTACHMENT_BUDGETS: AttachmentBudgets = AttachmentBudgets {
    max_count: 10,
    max_file_bytes: 10 * 1024 * 1024,
    max_total_bytes: 10 * 1024 * 1024,
};

/// Browser-facing inline-attachment contract.
///
/// Carries the `accept` tokens generated from the shared
/// [`ironclaw_common`] format registry (so a file picker can never drift from
/// the server's allowed MIME set) plus the budgets the server-side decode
/// enforces. A surface uses this only for pre-submit hints; the server-side
/// decode remains the sole authority on what is accepted.
///
/// It lives beside [`AttachmentBudgets`] because this crate is the one home for
/// attachment size ceilings (PROPOSAL §6.4.9): a transport that advertises a
/// ceiling and the routine that enforces it must read the same constant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentCapabilities {
    /// HTML file-input `accept` tokens from the shared registry: exact MIME
    /// types plus extensions, e.g. `["image/png", ".png", "application/pdf",
    /// ".pdf"]` — never `image/*` wildcards (which would advertise unsupported
    /// formats, and which break folder navigation in the native macOS picker).
    pub accept: Vec<String>,
    /// The count/byte budgets the decode enforces. Flattened, so the wire shape
    /// is unchanged and a new budget field reaches the browser without an
    /// intermediate edit here.
    #[serde(flatten)]
    pub budgets: AttachmentBudgets,
}

/// The inline-attachment contract advertised to browsers. Generated from the
/// shared format registry and the budgets the decode enforces, so the picker
/// and the server stay in lockstep by construction.
pub fn attachment_capabilities() -> AttachmentCapabilities {
    AttachmentCapabilities {
        accept: ironclaw_common::accept_tokens(),
        budgets: DEFAULT_ATTACHMENT_BUDGETS,
    }
}

/// Ceilings for one voice clip recorded in a product composer and submitted for
/// transcription.
///
/// A voice clip is not an attachment — it is never landed, never persisted, and
/// only its transcript survives the request. It nevertheless shares this
/// module's charter: this crate is the one home for the byte ceilings a
/// transport advertises and a server-side decode enforces, so the transcription
/// route and the decode that guards it read the same constant instead of each
/// picking a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VoiceClipBudget {
    /// Maximum decoded clip size. Sized to the same ~10 MiB decoded payload the
    /// gateway-wide 14 MiB body budget already covers once base64 and JSON
    /// framing are added, so a clip that passes this check could always have
    /// reached the handler.
    pub max_bytes: usize,
    /// Maximum clip length the recorder should allow before stopping itself.
    /// A browser hint, not a server check: the host cannot cheaply measure the
    /// duration of an opaque container, so [`Self::max_bytes`] is the enforced
    /// bound and this only stops the recorder before it produces a clip that
    /// would be rejected.
    ///
    /// The two are **not** independent. Browsers upload transcription clips as
    /// 16 kHz mono 16-bit WAV (the recorded webm/mp4 is re-encoded client-side
    /// because the transcription endpoint decodes neither), which is a fixed
    /// 32,000 bytes per second — so a clip recorded up to this length must
    /// still fit inside [`Self::max_bytes`], or the recorder would happily
    /// produce something the upload then rejects. Pinned by
    /// `voice_duration_ceiling_fits_the_byte_ceiling_as_wav`.
    pub max_duration_secs: u32,
}

/// Current voice-clip ceilings.
pub const DEFAULT_VOICE_CLIP_BUDGET: VoiceClipBudget = VoiceClipBudget {
    max_bytes: 10 * 1024 * 1024,
    max_duration_secs: 300,
};

/// Browser-facing voice-capture contract.
///
/// Carries the audio MIME types the server will accept for transcription —
/// generated from the shared [`ironclaw_common`] format registry, so the
/// recorder can never pick a container the decode then rejects — plus the
/// budgets that decode enforces. Same relationship as
/// [`AttachmentCapabilities`]: a pre-submit hint, with the server-side decode
/// remaining the sole authority.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VoiceCapabilities {
    /// Exact audio MIME types the transcription route accepts, in registry
    /// order — never `audio/*`. The browser picks the first one its
    /// `MediaRecorder` supports, which is how one contract covers Chrome
    /// (`audio/webm`) and Safari (`audio/mp4`) without a per-browser branch.
    pub accept: Vec<String>,
    #[serde(flatten)]
    pub budget: VoiceClipBudget,
}

/// The voice-capture contract advertised to browsers. Generated from the shared
/// format registry and the budget the decode enforces.
pub fn voice_capabilities() -> VoiceCapabilities {
    VoiceCapabilities {
        accept: audio_mime_types(),
        budget: DEFAULT_VOICE_CLIP_BUDGET,
    }
}

/// Every audio MIME type the shared registry recognizes, canonical spellings
/// and aliases alike, in registry order.
///
/// Aliases are included deliberately: a browser reports the type it will
/// actually emit (`audio/x-m4a`, `audio/opus`), and rejecting a spelling the
/// registry already resolves would fail a recording the decode would have
/// accepted.
pub fn audio_mime_types() -> Vec<String> {
    ironclaw_common::attachment_format::all_formats()
        .iter()
        .filter(|format| format.kind == ironclaw_common::AttachmentKind::Audio)
        .flat_map(|format| {
            std::iter::once(format.mime.to_string())
                .chain(format.mime_aliases.iter().map(|alias| alias.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_capabilities_carry_the_enforced_budgets_and_registry_tokens() {
        let advertised = attachment_capabilities();
        assert_eq!(
            advertised.budgets, DEFAULT_ATTACHMENT_BUDGETS,
            "the advertised ceiling must be the enforced ceiling"
        );
        assert_eq!(
            advertised.accept,
            ironclaw_common::accept_tokens(),
            "accept tokens come from the shared registry, never a local list"
        );
        assert!(
            !advertised.accept.iter().any(|token| token.contains('*')),
            "wildcards would advertise unsupported formats: {:?}",
            advertised.accept
        );
    }

    /// The budgets are `#[serde(flatten)]`ed, so the browser sees one flat
    /// object. A nested `budgets` key would silently break every client that
    /// reads `max_file_bytes` at the top level.
    #[test]
    fn advertised_capabilities_serialize_the_budgets_flat() {
        let json = serde_json::to_value(attachment_capabilities()).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(object.contains_key("accept"));
        assert!(object.contains_key("max_count"));
        assert!(object.contains_key("max_file_bytes"));
        assert!(object.contains_key("max_total_bytes"));
        assert!(
            !object.contains_key("budgets"),
            "budgets must stay flattened"
        );
    }

    /// The recorder picks a container from this list, so a MediaRecorder type
    /// the browsers actually emit must be in it — otherwise voice capture is
    /// silently unavailable on that browser.
    #[test]
    fn voice_capabilities_cover_the_browser_recorder_containers() {
        let advertised = voice_capabilities();
        for required in ["audio/webm", "audio/mp4", "audio/ogg"] {
            assert!(
                advertised.accept.iter().any(|mime| mime == required),
                "{required} is a MediaRecorder container and must be advertised: {:?}",
                advertised.accept,
            );
        }
        assert!(
            !advertised.accept.iter().any(|token| token.contains('*')),
            "wildcards would advertise formats the decode rejects: {:?}",
            advertised.accept,
        );
        assert!(
            !advertised.accept.iter().any(|token| token.starts_with('.')),
            "voice capture selects a recorder MIME type, not a file extension: {:?}",
            advertised.accept,
        );
    }

    /// Every advertised type must resolve in the registry, or the pre-submit
    /// hint would promise something the server-side decode refuses.
    #[test]
    fn every_advertised_voice_mime_is_registry_supported() {
        for mime in voice_capabilities().accept {
            assert!(
                ironclaw_common::attachment_format::is_supported_mime(&mime),
                "advertised voice MIME {mime} is not in the shared registry",
            );
            assert_eq!(
                ironclaw_common::attachment_format::kind_for_mime(&mime),
                ironclaw_common::AttachmentKind::Audio,
                "advertised voice MIME {mime} is not audio",
            );
        }
    }

    /// Flattened like [`AttachmentBudgets`] — a nested `budget` key would break
    /// a browser reading `max_bytes` at the top level.
    #[test]
    fn voice_capabilities_serialize_the_budget_flat() {
        let json = serde_json::to_value(voice_capabilities()).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(object.contains_key("accept"));
        assert!(object.contains_key("max_bytes"));
        assert!(object.contains_key("max_duration_secs"));
        assert!(!object.contains_key("budget"), "budget must stay flattened");
    }

    /// The decoded ceiling has to fit inside the gateway body budget once
    /// base64 (4/3) and JSON framing are added, or a clip within the advertised
    /// limit would be rejected by the body-limit layer before any handler runs
    /// — a limit the user cannot see or act on.
    #[test]
    fn voice_clip_ceiling_fits_the_gateway_body_budget() {
        const GATEWAY_BODY_BUDGET_BYTES: usize = 14 * 1024 * 1024;
        let encoded = DEFAULT_VOICE_CLIP_BUDGET.max_bytes.div_ceil(3) * 4;
        assert!(
            encoded < GATEWAY_BODY_BUDGET_BYTES,
            "base64 of the {}-byte ceiling is {encoded} bytes, over the {GATEWAY_BODY_BUDGET_BYTES}-byte body budget",
            DEFAULT_VOICE_CLIP_BUDGET.max_bytes,
        );
    }

    /// Bytes per second of the format the browser actually uploads: 16 kHz,
    /// mono, 16-bit PCM. `voice-encode.ts` is the other half of this contract.
    const WAV_BYTES_PER_SECOND: usize = 16_000 * 2;

    /// The duration hint and the byte ceiling describe the same clip. If a
    /// recording allowed to run the full duration cannot fit the byte ceiling
    /// once encoded, the recorder stops on its own limit and the upload is
    /// then rejected on ours — a failure the user did nothing to cause and
    /// cannot act on.
    #[test]
    fn voice_duration_ceiling_fits_the_byte_ceiling_as_wav() {
        let encoded = DEFAULT_VOICE_CLIP_BUDGET.max_duration_secs as usize * WAV_BYTES_PER_SECOND;
        assert!(
            encoded <= DEFAULT_VOICE_CLIP_BUDGET.max_bytes,
            "a {}s clip encodes to {encoded} bytes of 16 kHz mono WAV, over the {}-byte ceiling: \
             lower max_duration_secs or raise max_bytes",
            DEFAULT_VOICE_CLIP_BUDGET.max_duration_secs,
            DEFAULT_VOICE_CLIP_BUDGET.max_bytes,
        );
    }

    #[test]
    fn default_budgets_match_webui_contract() {
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_count, 10);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes, 10 * 1024 * 1024);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes, 10 * 1024 * 1024);
    }
}
