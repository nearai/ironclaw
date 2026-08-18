//! The product-facing speech-to-text port and its wire vocabulary.
//!
//! One product operation: take a recorded voice clip, return its transcript.
//! It starts no turn, writes no durable record, and keeps no audio — the
//! transcript is handed straight back for the user to edit in a composer, and
//! only becomes conversation data if they choose to send it.
//!
//! Declaring [`TranscriptionService`] here rather than in `ironclaw_assistant`
//! follows the same reasoning as [`crate::operator_llm`]: the implementation is
//! a composition-owned adapter over `ironclaw_llm`'s `TranscriptionProvider`,
//! and product must not depend on the model tier to name the port it consumes.
//! Product keeps the command descriptor, the bound validation, and the
//! `RebornServices` wiring.
//!
//! Wire-safety: the response carries transcript text and nothing else. Provider
//! bodies, status codes, and model ids never cross this boundary — the adapter
//! classifies a failure into a [`ProductSurfaceError`] and logs the detail
//! host-side.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::descriptors::ProductSurfaceCommandDescriptor;
use crate::surface::{ProductSurfaceCaller, ProductSurfaceError};

/// Transcribe one voice clip. A pure transform: no durable product state, no
/// turn, no retained audio — only the transcript, handed back for the user to
/// edit before they decide to send it.
///
/// The descriptor is declared here rather than in `ironclaw_assistant` because
/// a transport consumes the product *boundary*, not the product crate
/// (PROPOSAL §6.1.3): WebUI imports it from this module and never compiles
/// product to speak this operation.
pub const TRANSCRIBE_AUDIO_COMMAND_ID: &str = "audio.transcribe";
pub const TRANSCRIBE_AUDIO_COMMAND: ProductSurfaceCommandDescriptor<
    RebornTranscribeAudioRequest,
    RebornTranscribeAudioResponse,
> = ProductSurfaceCommandDescriptor::new(TRANSCRIBE_AUDIO_COMMAND_ID);

/// One voice clip submitted for transcription.
///
/// The bytes travel base64-encoded because this is a JSON product operation
/// like every other one on the surface; the transport's body limit and the
/// service's decoded-byte ceiling bound it on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTranscribeAudioRequest {
    /// The recorder's declared container type, e.g. `audio/webm` (Chrome) or
    /// `audio/mp4` (Safari). Checked against the shared audio-format registry
    /// before the clip is decoded, so an unsupported container is rejected
    /// before any egress.
    pub mime_type: String,
    /// Standard base64 (with padding) of the clip bytes.
    pub audio_base64: String,
}

/// The transcript of one voice clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornTranscribeAudioResponse {
    pub text: String,
}

/// Host-side speech-to-text, invoked once per clip.
///
/// Implementations receive already-validated bytes: the caller has checked the
/// media type against the registry and enforced the decoded-byte ceiling, so an
/// implementation's own failures are provider failures.
#[async_trait]
pub trait TranscriptionService: Send + Sync {
    /// Transcribe one clip.
    ///
    /// `caller` is the authenticated identity the transcription is performed
    /// for — carried so an implementation can attribute billable inference,
    /// never so it can be read out of a request payload.
    async fn transcribe(
        &self,
        caller: &ProductSurfaceCaller,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<String, ProductSurfaceError>;
}
