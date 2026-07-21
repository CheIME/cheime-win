//! TSF key event admission rules for the TIP.
//!
//! Defines the permission matrix for `OnTestKeyDown`:
//! which keys CheIME handles in each input mode.
//!
//! This is a pure function — no side effects, no engine communication.

/// Input mode state tracked locally in the TIP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    /// Direct pass-through (English-like).
    Direct,
    /// CheIME processes Chinese Pinyin input.
    Chinese,
}

/// Result of a key admission check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAdmission {
    /// CheIME handles this key.
    Handled,
    /// Pass through to the application.
    PassThrough,
    /// Toggle between Chinese/Direct mode.
    ToggleMode,
}

/// Check whether CheIME should handle a key, given the current mode
/// and whether CheIME is activated.
///
/// This is the logic that runs in `OnTestKeyDown` (no side effects).
pub fn check_key(
    mode: InputMode,
    cheime_activated: bool,
    key_code: u32, // Windows virtual key code
    is_shift: bool,
    is_ctrl: bool,
    is_alt: bool,
    has_composition: bool,
) -> KeyAdmission {
    if !cheime_activated {
        // CheIME not active: only mode-toggle shortcut is accepted
        if is_ctrl && !is_shift && !is_alt && key_code == 0x20 {
            // Ctrl+Space
            return KeyAdmission::ToggleMode;
        }
        if is_shift && !is_ctrl && !is_alt && key_code == 0x20 {
            // Shift+Space
            return KeyAdmission::ToggleMode;
        }
        return KeyAdmission::PassThrough;
    }

    match mode {
        InputMode::Direct => {
            // Direct mode: only pass through, except mode toggle
            if !is_alt && key_code == 0x20 && (is_ctrl || is_shift) {
                return KeyAdmission::ToggleMode;
            }
            KeyAdmission::PassThrough
        }
        InputMode::Chinese => {
            chinese_mode_keys(key_code, is_shift, is_ctrl, is_alt, has_composition)
        }
    }
}

/// Key admission for Chinese input mode.
fn chinese_mode_keys(
    key_code: u32,
    is_shift: bool,
    is_ctrl: bool,
    is_alt: bool,
    has_composition: bool,
) -> KeyAdmission {
    // Ctrl+Space / Shift+Space: toggle mode
    // Must check Shift+Space first (Ctrl key may also be reported as pressed
    // by the keyboard hardware or GetKeyState in TSF callbacks).
    if key_code == 0x20 && !is_alt {
        if is_ctrl && !is_shift {
            return KeyAdmission::ToggleMode;
        }
        if is_shift && !is_ctrl {
            return KeyAdmission::ToggleMode;
        }
    }

    // Ctrl/Alt modifiers → pass through (application shortcuts)
    // Shift alone (no Ctrl/Alt) is fine — shifted letter or Space
    if is_ctrl || is_alt {
        return KeyAdmission::PassThrough;
    }

    // Only process keys when no Ctrl/Alt held
    match key_code {
        // a-z: handled (with Shift = uppercase, without = lowercase)
        0x41..=0x5A => {
            // VK_A through VK_Z
            KeyAdmission::Handled
        }

        // Backspace: only handled when there is composition text
        0x08 => {
            if has_composition {
                KeyAdmission::Handled
            } else {
                KeyAdmission::PassThrough
            }
        }

        // Enter: handled
        0x0D => KeyAdmission::Handled,

        // Escape: handled
        0x1B => KeyAdmission::Handled,

        // Space: handle commit/select.  When no composition, pass-through
        // so the application receives a real space character.
        0x20 => {
            if has_composition {
                KeyAdmission::Handled
            } else {
                KeyAdmission::PassThrough
            }
        }

        // Digits 0-9 and numpad digits: candidate selection — NOT sent to engine
        // The TIP layer intercepts these in key_down to commit the numbered candidate.
        0x30..=0x39 | 0x60..=0x69 => {
            if has_composition {
                KeyAdmission::Handled
            } else {
                KeyAdmission::PassThrough
            }
        }

        // + and -: page up/down
        0xBB | 0x6B => KeyAdmission::Handled, // =/+
        0xBD | 0x6D => KeyAdmission::Handled, // -/_

        // PageUp/PageDown: handled
        0x21 | 0x22 => KeyAdmission::Handled,

        // Up/Down arrows: handled
        0x26 | 0x28 => KeyAdmission::Handled,

        // Left/Right: pass through (navigation in the app's text)
        0x25 | 0x27 => KeyAdmission::PassThrough,

        // Everything else: pass through
        _ => KeyAdmission::PassThrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Virtual key constants
    const VK_BACK: u32 = 0x08;
    const VK_RETURN: u32 = 0x0D;
    const VK_ESCAPE: u32 = 0x1B;
    const VK_SPACE: u32 = 0x20;
    const VK_PRIOR: u32 = 0x21; // PageUp
    const VK_NEXT: u32 = 0x22; // PageDown
    const VK_LEFT: u32 = 0x25;
    const VK_UP: u32 = 0x26;
    const VK_RIGHT: u32 = 0x27;
    const VK_DOWN: u32 = 0x28;
    const VK_A: u32 = 0x41;
    const VK_Z: u32 = 0x5A;
    const VK_0: u32 = 0x30;
    const VK_9: u32 = 0x39;

    #[test]
    fn not_activated_passes_through_most_keys() {
        assert_eq!(
            check_key(InputMode::Chinese, false, VK_A, false, false, false, false),
            KeyAdmission::PassThrough
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                false,
                VK_RETURN,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::PassThrough
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                false,
                VK_SPACE,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn ctrl_space_toggles_even_when_not_activated() {
        assert_eq!(
            check_key(
                InputMode::Direct,
                false,
                VK_SPACE,
                false,
                true,
                false,
                false
            ),
            KeyAdmission::ToggleMode
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                false,
                VK_SPACE,
                false,
                true,
                false,
                false
            ),
            KeyAdmission::ToggleMode
        );
    }

    #[test]
    fn shift_space_toggles_when_activated() {
        // Shift+Space toggles when CheIME is activated
        assert_eq!(
            check_key(InputMode::Direct, true, VK_SPACE, true, false, false, false),
            KeyAdmission::ToggleMode
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_SPACE,
                true,
                false,
                false,
                false
            ),
            KeyAdmission::ToggleMode
        );
    }

    #[test]
    fn direct_mode_passes_through_letters() {
        assert_eq!(
            check_key(InputMode::Direct, true, VK_A, false, false, false, false),
            KeyAdmission::PassThrough
        );
        assert_eq!(
            check_key(InputMode::Direct, true, VK_Z, false, false, false, false),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn chinese_mode_handles_letters() {
        assert_eq!(
            check_key(InputMode::Chinese, true, VK_A, false, false, false, false),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(InputMode::Chinese, true, VK_Z, false, false, false, false),
            KeyAdmission::Handled
        );
    }

    #[test]
    fn chinese_mode_handles_special_keys() {
        // Backspace is handled when composition exists
        assert_eq!(
            check_key(InputMode::Chinese, true, VK_BACK, false, false, false, true),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_RETURN,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_ESCAPE,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_SPACE,
                false,
                false,
                false,
                true
            ),
            KeyAdmission::Handled
        );
    }

    #[test]
    fn backspace_passes_through_when_no_composition() {
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_BACK,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn chinese_mode_handles_digits() {
        // Digits now pass-through to avoid crashing engine (candidate selection in TIP layer)
        for vk in VK_0..=VK_9 {
            assert_eq!(
                check_key(InputMode::Chinese, true, vk, false, false, false, false),
                KeyAdmission::PassThrough
            );
        }
    }

    #[test]
    fn chinese_mode_handles_page_and_arrows() {
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_PRIOR,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_NEXT,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(InputMode::Chinese, true, VK_UP, false, false, false, false),
            KeyAdmission::Handled
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_DOWN,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::Handled
        );
    }

    #[test]
    fn chinese_mode_passes_through_left_right() {
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_LEFT,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::PassThrough
        );
        assert_eq!(
            check_key(
                InputMode::Chinese,
                true,
                VK_RIGHT,
                false,
                false,
                false,
                false
            ),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn ctrl_combos_pass_through() {
        // Ctrl+C should pass through (copy)
        assert_eq!(
            check_key(InputMode::Chinese, true, 0x43, false, true, false, false),
            KeyAdmission::PassThrough
        );
        // Ctrl+V should pass through (paste)
        assert_eq!(
            check_key(InputMode::Chinese, true, 0x56, false, true, false, false),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn alt_combos_pass_through() {
        // Alt+F should pass through
        assert_eq!(
            check_key(InputMode::Chinese, true, 0x46, false, false, true, false),
            KeyAdmission::PassThrough
        );
    }

    #[test]
    fn unknown_keys_pass_through() {
        // F1
        assert_eq!(
            check_key(InputMode::Chinese, true, 0x70, false, false, false, false),
            KeyAdmission::PassThrough
        );
        // Tab
        assert_eq!(
            check_key(InputMode::Chinese, true, 0x09, false, false, false, false),
            KeyAdmission::PassThrough
        );
    }
}
