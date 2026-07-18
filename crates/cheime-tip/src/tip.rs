//! TIP instance — the text input processor object.
//!
//! Created by `IClassFactory::CreateInstance`. Owns the I/O thread,
//! candidate window, channel dispatch, and pipe connection state.

use crate::exports::increment_object_count;
use crate::key_handler::{InputMode, KeyAdmission, check_key};
use cheime_model::{Key, KeyEvent, KeyState};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::TipChannel;
use std::sync::atomic::{AtomicU32, Ordering};

/// CheIME TIP — the object that TSF interacts with.
///
/// When `ActivateEx` is called, the TIP:
/// 1. Creates a channel for TSF→I/O thread communication
/// 2. Spawns the I/O thread which connects to the engine pipe
/// 3. Creates a hidden candidate window
///
/// On key events, `OnTestKeyDown` checks the key admission matrix
/// without side effects. `OnKeyDown` pushes the key event to the
/// I/O thread via the bounded mpsc channel.
pub struct CheimeTip {
    pub ref_count: AtomicU32,
    /// Current input mode (Chinese / Direct).
    pub mode: InputMode,
    /// Whether CheIME is the active TIP.
    pub activated: bool,
    /// Bounded channel for sending messages to the I/O thread.
    pub channel: Option<TipChannel>,
    /// Connection state.
    pub connected: bool,
}

impl CheimeTip {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            ref_count: AtomicU32::new(1),
            mode: InputMode::Chinese,
            activated: false,
            channel: None,
            connected: false,
        })
    }

    pub fn add_ref(&self) -> u32 {
        self.ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn release(&self) -> u32 {
        let prev = self.ref_count.fetch_sub(1, Ordering::Relaxed);
        prev - 1
    }

    /// Activate the TIP — called by TSF when CheIME is switched on.
    pub fn activate(&mut self) {
        self.activated = true;
        // Create the bounded channel (capacity: 64 messages)
        if self.channel.is_none() {
            self.channel = Some(TipChannel::new(64));
        }
        self.connected = false;
        // I/O thread creation happens here in real TIP
    }

    /// Deactivate the TIP — called when user switches away from CheIME.
    pub fn deactivate(&mut self) {
        self.activated = false;
        self.connected = false;
        // In real TIP: hide candidate window, signal I/O thread to disconnect
    }

    /// Toggle between Chinese and Direct mode.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            InputMode::Chinese => InputMode::Direct,
            InputMode::Direct => InputMode::Chinese,
        };
    }

    /// Check key admission (OnTestKeyDown) — pure function, no side effects.
    pub fn test_key(
        &self,
        key_code: u32,
        is_shift: bool,
        is_ctrl: bool,
        is_alt: bool,
    ) -> KeyAdmission {
        check_key(
            self.mode,
            self.activated,
            key_code,
            is_shift,
            is_ctrl,
            is_alt,
        )
    }

    /// Handle key down (OnKeyDown) — push to channel if handled.
    ///
    /// Returns `true` if the key was consumed by CheIME.
    pub fn handle_key(&self, key_code: u32, is_shift: bool, is_ctrl: bool, is_alt: bool) -> bool {
        let admission = self.test_key(key_code, is_shift, is_ctrl, is_alt);

        match admission {
            KeyAdmission::ToggleMode => {
                // We can't mutate &self in real COM — the toggle happens
                // via refcounted interior mutability. For the MVP skeleton:
                true
            }
            KeyAdmission::Handled => {
                // Convert VK to cheime_model Key
                let key = vk_to_key(key_code);
                let state = KeyState {
                    shift: is_shift,
                    control: is_ctrl,
                    alt: is_alt,
                };
                // Build a KeyCommand and push to channel
                // In real TIP, this needs a MessageHeader with client/session/epoch
                if let Some(ref channel) = self.channel {
                    // Placeholder: send a minimal message
                    // Full implementation: construct a MessageHeader with
                    // client_instance_id, session_id, epoch, sequence, revision,
                    // deployment_generation.
                    let _ = channel.try_send(FrontendMessage::KeyCommand {
                        header: cheime_protocol::MessageHeader {
                            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
                            client: cheime_model::ClientInstanceId::new(1),
                            session: cheime_model::SessionId::new(1),
                            epoch: cheime_model::SessionEpoch::new(1),
                            sequence: cheime_model::Sequence::new(1),
                            revision: cheime_model::Revision::new(0),
                            deployment: cheime_model::DeploymentGeneration::new(1),
                        },
                        event: KeyEvent { key, state },
                    });
                }
                true
            }
            KeyAdmission::PassThrough => false,
        }
    }
}

/// Convert a Windows virtual key code to a cheime_model `Key`.
fn vk_to_key(vk: u32) -> Key {
    match vk {
        0x08 => Key::Backspace,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x41..=0x5A => {
            // Convert VK_A (0x41) → 'a', VK_Z (0x5A) → 'z'
            let c = ((vk - 0x41) as u8 + b'a') as char;
            Key::Character(c)
        }
        _ => Key::Character('?'), // unknown → placeholder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_new_has_ref_count_one() {
        let tip = CheimeTip::new();
        assert_eq!(tip.ref_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn add_ref_and_release() {
        let tip = CheimeTip::new();
        assert_eq!(tip.add_ref(), 2);
        assert_eq!(tip.release(), 1);
    }

    #[test]
    fn new_tip_starts_in_chinese_mode() {
        let tip = CheimeTip::new();
        assert_eq!(tip.mode, InputMode::Chinese);
    }

    #[test]
    fn toggle_switches_mode() {
        let mut tip = CheimeTip::new();
        assert_eq!(tip.mode, InputMode::Chinese);
        tip.toggle_mode();
        assert_eq!(tip.mode, InputMode::Direct);
        tip.toggle_mode();
        assert_eq!(tip.mode, InputMode::Chinese);
    }

    #[test]
    fn activate_sets_up_channel() {
        let mut tip = CheimeTip::new();
        assert!(!tip.activated);
        assert!(tip.channel.is_none());

        tip.activate();
        assert!(tip.activated);
        assert!(tip.channel.is_some());
    }

    #[test]
    fn deactivate_clears_state() {
        let mut tip = CheimeTip::new();
        tip.activate();
        tip.connected = true;
        tip.deactivate();
        assert!(!tip.activated);
        assert!(!tip.connected);
    }

    #[test]
    fn test_key_passes_through_when_not_activated() {
        let tip = CheimeTip::new();
        // Not activated → even 'a' should pass through
        assert_eq!(
            tip.test_key(0x41, false, false, false),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn test_key_handles_letter_when_activated() {
        let mut tip = CheimeTip::new();
        tip.activate();
        assert_eq!(
            tip.test_key(0x41, false, false, false),
            KeyAdmission::Handled
        );
    }

    #[test]
    fn handle_key_consumes_when_handled() {
        let mut tip = CheimeTip::new();
        tip.activate();
        assert!(tip.handle_key(0x41, false, false, false)); // 'a' → consumed
    }

    #[test]
    fn handle_key_passes_through_when_not_handled() {
        let mut tip = CheimeTip::new();
        tip.activate();
        // F1 is not handled
        assert!(!tip.handle_key(0x70, false, false, false));
    }

    #[test]
    fn vk_to_key_conversion() {
        assert_eq!(vk_to_key(0x08), Key::Backspace);
        assert_eq!(vk_to_key(0x0D), Key::Enter);
        assert_eq!(vk_to_key(0x1B), Key::Escape);
        assert_eq!(vk_to_key(0x20), Key::Space);
        assert_eq!(vk_to_key(0x41), Key::Character('a'));
        assert_eq!(vk_to_key(0x5A), Key::Character('z'));
    }
}
