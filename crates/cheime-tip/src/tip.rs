//! TIP instance — the text input processor object.
//!
//! COM layout: the first pointer-sized field is `vtbl_ptr`, pointing to the
//! active vtable. This is how COM dispatches method calls.
//! Other COM state follows the vtable pointer.

use crate::exports::increment_object_count;
use crate::key_handler::{InputMode, KeyAdmission, check_key};
use cheime_model::{Key, KeyEvent, KeyState};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::TipChannel;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::TextServices::ITfThreadMgr;

#[repr(C)]
pub struct CheimeTip {
    /// COM vtable pointer — MUST be first for COM dispatch.
    pub vtbl_ptr: *const std::ffi::c_void,
    pub ref_count: AtomicU32,
    pub mode: Cell<InputMode>,
    pub activated: bool,
    pub channel: Option<TipChannel>,
    pub connected: bool,
    sequence: Cell<u64>,
    thread_mgr: RefCell<Option<*mut ITfThreadMgr>>,
    candidate_hwnd: Cell<HWND>,
}

impl CheimeTip {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            vtbl_ptr: std::ptr::null(), // set by class_factory after creation
            ref_count: AtomicU32::new(1),
            mode: Cell::new(InputMode::Chinese),
            activated: false,
            channel: None,
            connected: false,
            sequence: Cell::new(1),
            thread_mgr: RefCell::new(None),
            candidate_hwnd: Cell::new(HWND(std::ptr::null_mut())),
        })
    }

    pub fn add_ref(&self) -> u32 {
        self.ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn release(&self) -> u32 {
        let prev = self.ref_count.fetch_sub(1, Ordering::Relaxed);
        prev - 1
    }

    pub fn activate(&mut self) {
        self.activated = true;
        if self.channel.is_none() {
            self.channel = Some(TipChannel::new(64));
        }
        self.connected = false;
    }

    pub fn deactivate(&mut self) {
        self.activated = false;
        self.connected = false;
    }

    pub fn toggle_mode(&self) {
        let next = match self.mode.get() {
            InputMode::Chinese => InputMode::Direct,
            InputMode::Direct => InputMode::Chinese,
        };
        self.mode.set(next);
    }

    pub fn test_key(&self, key_code: u32, is_shift: bool, is_ctrl: bool, is_alt: bool) -> KeyAdmission {
        check_key(self.mode.get(), self.activated, key_code, is_shift, is_ctrl, is_alt)
    }

    pub fn handle_key(&self, key_code: u32, is_shift: bool, is_ctrl: bool, is_alt: bool) -> bool {
        let admission = self.test_key(key_code, is_shift, is_ctrl, is_alt);
        match admission {
            KeyAdmission::ToggleMode => { self.toggle_mode(); true }
            KeyAdmission::Handled => {
                let key = vk_to_key(key_code);
                let state = KeyState { shift: is_shift, control: is_ctrl, alt: is_alt };
                if let Some(ref channel) = self.channel {
                    let seq = self.sequence.get();
                    self.sequence.set(seq + 1);
                    let _ = channel.try_send(FrontendMessage::KeyCommand {
                        header: cheime_protocol::MessageHeader {
                            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
                            client: cheime_model::ClientInstanceId::new(1),
                            session: cheime_model::SessionId::new(1),
                            epoch: cheime_model::SessionEpoch::new(1),
                            sequence: cheime_model::Sequence::new(seq),
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

    pub fn set_thread_mgr(&self, ptim: *mut ITfThreadMgr) {
        *self.thread_mgr.borrow_mut() = Some(ptim);
    }

    pub fn set_candidate_hwnd(&self, hwnd: HWND) {
        self.candidate_hwnd.set(hwnd);
    }

    pub fn candidate_hwnd(&self) -> HWND {
        self.candidate_hwnd.get()
    }
}

fn vk_to_key(vk: u32) -> Key {
    match vk {
        0x08 => Key::Backspace, 0x0D => Key::Enter, 0x1B => Key::Escape, 0x20 => Key::Space,
        0x41..=0x5A => Key::Character(((vk - 0x41) as u8 + b'a') as char),
        0x30..=0x39 => Key::Character(((vk - 0x30) as u8 + b'0') as char),
        0x60..=0x69 => Key::Character(((vk - 0x60) as u8 + b'0') as char),
        0xBC => Key::Character(','), 0xBE => Key::Character('.'), 0xBA => Key::Character(';'),
        0xBF => Key::Character('/'), 0xBB => Key::Character('='), 0xBD => Key::Character('-'),
        0xDB => Key::Character('['), 0xDD => Key::Character(']'), 0xDC => Key::Character('\\'),
        0xC0 => Key::Character('`'), 0xDE => Key::Character('\''),
        _ => Key::Character('?'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn tip_new_has_ref_count_one() { assert_eq!(CheimeTip::new().ref_count.load(Ordering::Relaxed), 1); }
    #[test] fn add_ref_and_release() { let t = CheimeTip::new(); assert_eq!(t.add_ref(), 2); assert_eq!(t.release(), 1); }
    #[test] fn new_tip_starts_in_chinese_mode() { assert_eq!(CheimeTip::new().mode.get(), InputMode::Chinese); }
    #[test] fn toggle_switches_mode() { let t = CheimeTip::new(); assert_eq!(t.mode.get(), InputMode::Chinese); t.toggle_mode(); assert_eq!(t.mode.get(), InputMode::Direct); }
    #[test] fn activate_sets_up_channel() { let mut t = CheimeTip::new(); t.activate(); assert!(t.channel.is_some()); }
    #[test] fn deactivate_clears_state() { let mut t = CheimeTip::new(); t.activate(); t.connected = true; t.deactivate(); assert!(!t.connected); }
    #[test] fn test_key_passes_through_when_not_activated() { assert_eq!(CheimeTip::new().test_key(0x41, false, false, false), KeyAdmission::PassThrough); }
    #[test] fn test_key_handles_letter_when_activated() { let mut t = CheimeTip::new(); t.activate(); assert_eq!(t.test_key(0x41, false, false, false), KeyAdmission::Handled); }
    #[test] fn handle_key_consumes_when_handled() { let mut t = CheimeTip::new(); t.activate(); assert!(t.handle_key(0x41, false, false, false)); }
    #[test] fn handle_key_passes_through_when_not_handled() { let mut t = CheimeTip::new(); t.activate(); assert!(!t.handle_key(0x70, false, false, false)); }
    #[test] fn vk_to_key_conversion() { assert_eq!(vk_to_key(0x41), Key::Character('a')); assert_eq!(vk_to_key(0x30), Key::Character('0')); }
    #[test] fn handle_key_toggle_mode_actually_switches() { let mut t = CheimeTip::new(); t.activate(); t.handle_key(0x20, true, false, false); assert_eq!(t.mode.get(), InputMode::Direct); }
    #[test] fn tip_vtbl_ptr_is_first_field() { let t = CheimeTip::new(); let ptr: *const CheimeTip = &*t; let vtbl_addr = &t.vtbl_ptr as *const _ as usize; let struct_addr = ptr as usize; assert_eq!(vtbl_addr, struct_addr); }
}
