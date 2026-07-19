//! Edit session helpers — applies PlatformActions inside TSF edit sessions.
//!
//! Engine responses carry `PlatformAction`s (SetPreedit, Commit, CancelComposition).
//! These must be applied within a TSF edit session on the document context.
//! This module provides the logic that the UI thread executes when it receives
//! a `WM_CHEIME_ACTION` message.

use cheime_model::{PlatformAction, PlatformActionKind, PlatformActionResult, PlatformActionOutcome};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::TipChannel;

/// Apply a single `PlatformAction` to the TSF document context.
///
/// Called on the UI thread from the WindowProc in response to a
/// `WM_CHEIME_ACTION` dispatch.
///
/// `channel` is used to send `PlatformActionResult` back to the I/O thread
/// (which forwards it to the engine for learning/state updates).
pub fn apply_platform_action(
    action: &PlatformAction,
    channel: &TipChannel,
) -> Result<(), String> {
    match &action.kind {
        PlatformActionKind::SetPreedit { text, cursor: _ } => {
            // In a real TIP: RequestEditSession → ITfRange::SetText
            // For now: just send confirmation that we "applied" it
            eprintln!("[tip] SetPreedit: \"{text}\"");
        }
        PlatformActionKind::Commit { text } => {
            // In a real TIP: RequestEditSession → ITfComposition::EndComposition
            // → ITfRange::SetText → PlatformActionResult::Applied
            eprintln!("[tip] Commit: \"{text}\"");
        }
        PlatformActionKind::CancelComposition => {
            eprintln!("[tip] CancelComposition");
        }
    }

    // Send confirmation back to engine
    let result = PlatformActionResult {
        action_id: action.id,
        outcome: PlatformActionOutcome::Applied,
    };

    // Build a minimal header — the I/O thread will fill in the real header
    let header = cheime_protocol::MessageHeader {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client: cheime_model::ClientInstanceId::new(1),
        session: cheime_model::SessionId::new(1),
        epoch: action.epoch,
        sequence: cheime_model::Sequence::new(0), // filled by I/O thread
        revision: action.revision,
        deployment: cheime_model::DeploymentGeneration::new(1),
    };

    let _ = channel.try_send(FrontendMessage::PlatformActionResult { header, result });
    Ok(())
}
