#![allow(unsafe_op_in_unsafe_fn)]

//! Safe, inert TSF COM shell.
//!
//! One allocation owns every interface header. No TSF activation, edit-session,
//! pipe, or UI behavior is performed here.

use crate::candidate_window::CandidateWindow;
use crate::edit_session::request_edit_session;
use crate::exports::{decrement_object_count, increment_object_count};
use crate::io_thread::IoThread;
use crate::key_handler::{InputMode, KeyAdmission, check_key};
use crate::runtime::{
    ActivationResources, ApartmentState, FocusResources, rollback_before_drop, run_before_drop,
};
use cheime_model::{Key, KeyEvent, KeyState, PlatformAction, PlatformActionKind};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::TipChannel;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use std::sync::mpsc::SyncSender;
use windows::Win32::Foundation::{BOOL, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Vtbl, ITfContext,
    ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Vtbl, ITfDocumentMgr, ITfKeyEventSink,
    ITfKeyEventSink_Vtbl, ITfKeystrokeMgr, ITfSource, ITfTextInputProcessor,
    ITfTextInputProcessor_Vtbl, ITfTextInputProcessorEx, ITfTextInputProcessorEx_Vtbl,
    ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Vtbl, TF_E_ALREADY_EXISTS,
};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface};

/// Write a diagnostic line to %TEMP%\cheime-tsf.log.
pub fn tsf_log(msg: &str) {
    use std::io::Write;
    use std::sync::Mutex;
    static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
    if let Ok(mut guard) = LOG.lock() {
        if guard.is_none() {
            let path = std::env::temp_dir().join("cheime-tsf-log.txt");
            *guard = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok();
        }
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{}", msg);
        }
    }
}

pub const S_OK: HRESULT = HRESULT(0);
pub const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002u32 as i32);
pub const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);
pub const E_NOTIMPL: HRESULT = HRESULT(0x8000_4001u32 as i32);
pub const E_UNEXPECTED: HRESULT = HRESULT(0x8000_FFFFu32 as i32);

pub const IID_TIP: GUID = ITfTextInputProcessor::IID;
pub const IID_TIP_EX: GUID = ITfTextInputProcessorEx::IID;
pub const IID_KEY: GUID = ITfKeyEventSink::IID;
pub const IID_TM: GUID = ITfThreadMgrEventSink::IID;
pub const IID_COMP: GUID = ITfCompositionSink::IID;
pub const IID_DA: GUID = ITfDisplayAttributeProvider::IID;

#[repr(C)]
pub struct PrimaryHeader {
    pub lp_vtbl: *const ITfTextInputProcessorEx_Vtbl,
}

#[repr(C)]
pub struct KeyHeader {
    pub lp_vtbl: *const ITfKeyEventSink_Vtbl,
}

#[repr(C)]
pub struct ThreadMgrHeader {
    pub lp_vtbl: *const ITfThreadMgrEventSink_Vtbl,
}

#[repr(C)]
pub struct CompositionHeader {
    pub lp_vtbl: *const ITfCompositionSink_Vtbl,
}

#[repr(C)]
pub struct DisplayHeader {
    pub lp_vtbl: *const ITfDisplayAttributeProvider_Vtbl,
}

/// The single allocation backing all interfaces exposed by a TIP instance.
#[repr(C)]
pub struct ComTip {
    pub primary: PrimaryHeader,
    pub key: KeyHeader,
    pub thread_mgr: ThreadMgrHeader,
    pub composition: CompositionHeader,
    pub display: DisplayHeader,
    ref_count: AtomicU32,
    runtime: RefCell<ApartmentState>,
    channel: RefCell<Option<TipChannel>>,
    io_thread: RefCell<Option<IoThread>>,
    candidate_window: RefCell<Option<CandidateWindow>>,
    mode: Cell<InputMode>,
    pub has_composition: Cell<bool>,
}

impl ComTip {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            primary: PrimaryHeader { lp_vtbl: &TIP_VTBL },
            key: KeyHeader { lp_vtbl: &KEY_VTBL },
            thread_mgr: ThreadMgrHeader {
                lp_vtbl: &THREAD_MGR_VTBL,
            },
            composition: CompositionHeader {
                lp_vtbl: &COMPOSITION_VTBL,
            },
            display: DisplayHeader {
                lp_vtbl: &DISPLAY_VTBL,
            },
            ref_count: AtomicU32::new(1),
            runtime: RefCell::new(ApartmentState::new()),
            channel: RefCell::new(None),
            io_thread: RefCell::new(None),
            candidate_window: RefCell::new(None),
            mode: Cell::new(InputMode::Chinese),
            has_composition: Cell::new(false),
        })
    }

    unsafe fn interface(owner: *mut Self, iid: &GUID) -> Option<*mut c_void> {
        if *iid == IUnknown::IID || *iid == IID_TIP || *iid == IID_TIP_EX {
            Some(unsafe { std::ptr::addr_of_mut!((*owner).primary).cast() })
        } else if *iid == IID_KEY {
            Some(unsafe { std::ptr::addr_of_mut!((*owner).key).cast() })
        } else if *iid == IID_TM {
            Some(unsafe { std::ptr::addr_of_mut!((*owner).thread_mgr).cast() })
        } else if *iid == IID_COMP {
            Some(unsafe { std::ptr::addr_of_mut!((*owner).composition).cast() })
        } else if *iid == IID_DA {
            Some(unsafe { std::ptr::addr_of_mut!((*owner).display).cast() })
        } else {
            None
        }
    }
}

impl Drop for ComTip {
    fn drop(&mut self) {
        if let Some(mut io_thread) = self.io_thread.get_mut().take() {
            io_thread.shutdown();
        }
        if let Some(candidate_window) = self.candidate_window.get_mut().take() {
            candidate_window.hide();
        }
        self.channel.get_mut().take();
        decrement_object_count();
    }
}

unsafe fn owner_at_offset(this: *mut c_void, offset: usize) -> *mut ComTip {
    unsafe { this.cast::<u8>().sub(offset).cast() }
}

/// # Safety
///
/// Caller must guarantee `this` is a valid `*mut PrimaryHeader` within a live `ComTip`.
pub unsafe fn owner_from_primary(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, primary)) }
}

/// # Safety
///
/// Caller must guarantee `this` is a valid `*mut KeyHeader` within a live `ComTip`.
pub unsafe fn owner_from_key(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, key)) }
}

/// # Safety
///
/// Caller must guarantee `this` is a valid `*mut ThreadMgrHeader` within a live `ComTip`.
pub unsafe fn owner_from_thread_mgr(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, thread_mgr)) }
}

/// # Safety
///
/// Caller must guarantee `this` is a valid `*mut CompositionHeader` within a live `ComTip`.
pub unsafe fn owner_from_composition(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, composition)) }
}

/// # Safety
///
/// Caller must guarantee `this` is a valid `*mut DisplayHeader` within a live `ComTip`.
pub unsafe fn owner_from_display(this: *mut c_void) -> *mut ComTip {
    unsafe { owner_at_offset(this, std::mem::offset_of!(ComTip, display)) }
}

unsafe fn query_owner(owner: *mut ComTip, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = null_mut() };
    if owner.is_null() || iid.is_null() {
        return E_POINTER;
    }
    let result = unsafe { ComTip::interface(owner, &*iid) };
    if let Some(interface) = result {
        unsafe { add_ref_owner(owner) };
        unsafe { *out = interface };
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe fn add_ref_owner(owner: *mut ComTip) -> u32 {
    unsafe { (*owner).ref_count.fetch_add(1, Ordering::Relaxed) + 1 }
}

unsafe fn release_owner(owner: *mut ComTip) -> u32 {
    let previous = unsafe { (*owner).ref_count.fetch_sub(1, Ordering::Release) };
    if previous == 1 {
        fence(Ordering::Acquire);
        unsafe { drop(Box::from_raw(owner)) };
        0
    } else {
        previous - 1
    }
}

macro_rules! iunknown_for_header {
    ($qi:ident, $add_ref:ident, $release:ident, $owner:ident) => {
        unsafe extern "system" fn $qi(
            this: *mut c_void,
            iid: *const GUID,
            out: *mut *mut c_void,
        ) -> HRESULT {
            if this.is_null() {
                return E_POINTER;
            }
            unsafe { query_owner($owner(this), iid, out) }
        }

        unsafe extern "system" fn $add_ref(this: *mut c_void) -> u32 {
            if this.is_null() {
                return 0;
            }
            unsafe { add_ref_owner($owner(this)) }
        }

        unsafe extern "system" fn $release(this: *mut c_void) -> u32 {
            if this.is_null() {
                return 0;
            }
            unsafe { release_owner($owner(this)) }
        }
    };
}

iunknown_for_header!(tip_qi, tip_add_ref, tip_release, owner_from_primary);
iunknown_for_header!(key_qi, key_add_ref, key_release, owner_from_key);
iunknown_for_header!(tm_qi, tm_add_ref, tm_release, owner_from_thread_mgr);
iunknown_for_header!(comp_qi, comp_add_ref, comp_release, owner_from_composition);
iunknown_for_header!(
    display_qi,
    display_add_ref,
    display_release,
    owner_from_display
);

const fn unknown_vtbl(
    query_interface: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
) -> IUnknown_Vtbl {
    IUnknown_Vtbl {
        QueryInterface: query_interface,
        AddRef: add_ref,
        Release: release,
    }
}

fn activation_current(owner: *mut ComTip, token: crate::runtime::ActivationToken) -> bool {
    ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.can_complete_activation(token)
    }) == Some(true)
}

fn abort_activation(owner: *mut ComTip, token: crate::runtime::ActivationToken) {
    let _ = ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.abort_activation(token);
    });
}

unsafe extern "system" fn activate(
    this: *mut c_void,
    thread_mgr: *mut c_void,
    client_id: u32,
) -> HRESULT {
    unsafe { activate_ex(this, thread_mgr, client_id, 0) }
}
unsafe extern "system" fn deactivate(this: *mut c_void) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_primary(this) };
    let resources = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_deactivation()
    }) {
        Some(resources) => resources,
        None => return E_UNEXPECTED,
    };
    // Shutdown I/O thread and hide candidate window
    if let Ok(mut io_opt) = unsafe { (*owner).io_thread.try_borrow_mut() } {
        if let Some(mut io) = io_opt.take() {
            io.shutdown();
        }
    }
    if let Ok(mut cw_opt) = unsafe { (*owner).candidate_window.try_borrow_mut() } {
        if let Some(cw) = cw_opt.as_ref() {
            cw.hide();
        }
        *cw_opt = None;
    }
    if let Ok(mut chan_opt) = unsafe { (*owner).channel.try_borrow_mut() } {
        *chan_opt = None;
    }
    if let Some(resources) = resources {
        run_before_drop(resources, |resources| {
            if let (Some(source), Some(cookie)) =
                (resources.source.as_ref(), resources.thread_sink_cookie)
            {
                let _ = unsafe { source.UnadviseSink(cookie) };
            }
            if let Some(keystroke_mgr) = resources.keystroke_mgr.as_ref() {
                let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(resources.client_id) };
            }
        });
    }
    S_OK
}
unsafe extern "system" fn activate_ex(
    this: *mut c_void,
    thread_mgr: *mut c_void,
    client_id: u32,
    _: u32,
) -> HRESULT {
    if this.is_null() || thread_mgr.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_primary(this) };
    let token = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_activation(client_id)
    }) {
        Some(Some(token)) => token,
        Some(None) => {
            tsf_log("[CheIME] TF_E_ALREADY_EXISTS");
            return TF_E_ALREADY_EXISTS;
        }
        None => {
            tsf_log("[CheIME] E_NOINTERFACE (wrong thread)");
            return E_NOINTERFACE;
        }
    };
    tsf_log(&format!("[CheIME] ActivateEx client_id={client_id}"));

    let manager = unsafe { ITfThreadMgr::from_raw_borrowed(&thread_mgr) }
        .expect("non-null ITfThreadMgr")
        .clone();
    // Clone for WindowContext (the original moves into ActivationResources below).
    let manager_for_window = manager.clone();
    if !activation_current(owner, token) {
        drop(manager);
        return E_UNEXPECTED;
    }
    let keystroke_mgr: ITfKeystrokeMgr = match manager.cast() {
        Ok(value) => value,
        Err(error) => {
            abort_activation(owner, token);
            return error.code();
        }
    };
    if !activation_current(owner, token) {
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let source: ITfSource = match manager.cast() {
        Ok(value) => value,
        Err(error) => {
            abort_activation(owner, token);
            return error.code();
        }
    };
    if !activation_current(owner, token) {
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let key_sink_raw = std::ptr::addr_of_mut!((*owner).key).cast();
    let key_sink =
        unsafe { ITfKeyEventSink::from_raw_borrowed(&key_sink_raw) }.expect("embedded key sink");
    if let Err(error) = unsafe { keystroke_mgr.AdviseKeyEventSink(client_id, key_sink, true) } {
        abort_activation(owner, token);
        return error.code();
    }
    if !activation_current(owner, token) {
        let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let thread_sink_raw = std::ptr::addr_of_mut!((*owner).thread_mgr).cast();
    let thread_sink = unsafe { IUnknown::from_raw_borrowed(&thread_sink_raw) }
        .expect("embedded thread manager sink");
    let cookie = match unsafe { source.AdviseSink(&IID_TM, thread_sink) } {
        Ok(cookie) => cookie,
        Err(error) => {
            let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
            abort_activation(owner, token);
            return error.code();
        }
    };
    if !activation_current(owner, token) {
        let _ = unsafe { source.UnadviseSink(cookie) };
        let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let focused_document = unsafe { manager.GetFocus() }.ok();
    if !activation_current(owner, token) {
        let _ = unsafe { source.UnadviseSink(cookie) };
        let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
        drop(focused_document);
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let focused_document_identity = focused_document.as_ref().and_then(canonical_identity);
    if !activation_current(owner, token) {
        let _ = unsafe { source.UnadviseSink(cookie) };
        let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
        drop(focused_document);
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let focused_context = focused_document
        .as_ref()
        .and_then(|document| unsafe { document.GetTop() }.ok());
    if !activation_current(owner, token) {
        let _ = unsafe { source.UnadviseSink(cookie) };
        let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
        drop(focused_context);
        drop(focused_document);
        drop(source);
        drop(keystroke_mgr);
        drop(manager);
        return E_UNEXPECTED;
    }
    let resources = ActivationResources {
        thread_mgr: manager,
        keystroke_mgr: keystroke_mgr.clone(),
        source: source.clone(),
        thread_sink_cookie: cookie,
        focused_document,
        focused_document_identity,
        focused_context,
    };
    let completed = ApartmentState::try_with_owned(
        unsafe { &(*owner).runtime },
        resources,
        |state, resources| state.complete_activation(token, resources),
    );
    let rejected = match completed {
        Ok(Ok(())) => None,
        Ok(Err(resources)) | Err(resources) => Some(resources),
    };
    if let Some(resources) = rejected {
        tsf_log("[CheIME] complete_activation REJECTED");
        rollback_before_drop(
            resources,
            |resources| {
                let _ = unsafe { resources.source.UnadviseSink(resources.thread_sink_cookie) };
            },
            |resources| {
                let _ = unsafe { resources.keystroke_mgr.UnadviseKeyEventSink(client_id) };
            },
        );
        return E_NOINTERFACE;
    }

    tsf_log("[CheIME] ActivateEx ACCEPTED");

    // --- I/O and candidate window startup ---
    let mut channel = TipChannel::new(64);
    let receiver = channel.take_receiver();

    // Build WindowContext with thread_mgr (cloned above before it moved),
    // client_id, and channel sender.
    let channel_sender = channel.clone_sender();
    let window_ctx =
        CandidateWindow::new_context(manager_for_window, client_id, channel_sender, owner);

    let cw = match CandidateWindow::create(window_ctx) {
        Ok(cw) => cw,
        Err(_) => {
            // Candidate window creation failed — continue without it
            let _ = unsafe { keystroke_mgr.UnadviseKeyEventSink(client_id) };
            abort_activation(owner, token);
            return E_UNEXPECTED;
        }
    };
    let candidate_hwnd = cw.hwnd();

    let io = IoThread::spawn(
        receiver.expect("receiver from fresh channel"),
        candidate_hwnd,
        r"\\.\pipe\cheime-engine",
    );

    // Store into owner
    if let Ok(mut chan) = (*owner).channel.try_borrow_mut() {
        *chan = Some(channel);
    }
    if let Ok(mut cw_opt) = (*owner).candidate_window.try_borrow_mut() {
        *cw_opt = Some(cw);
    }
    if let Ok(mut io_opt) = (*owner).io_thread.try_borrow_mut() {
        *io_opt = Some(io);
    }

    S_OK
}

static TIP_VTBL: ITfTextInputProcessorEx_Vtbl = ITfTextInputProcessorEx_Vtbl {
    base__: ITfTextInputProcessor_Vtbl {
        base__: unknown_vtbl(tip_qi, tip_add_ref, tip_release),
        Activate: activate,
        Deactivate: deactivate,
    },
    ActivateEx: activate_ex,
};

unsafe extern "system" fn key_focus(_: *mut c_void, _: BOOL) -> HRESULT {
    S_OK
}
/// `OnTestKeyDown` / `OnTestKeyUp` — check whether CheIME handles this key
/// without producing side effects (no state mutation, no engine messages).
unsafe extern "system" fn test_key(
    this: *mut c_void,
    _: *mut c_void,
    wparam: WPARAM,
    _lparam: LPARAM,
    eaten: *mut BOOL,
) -> HRESULT {
    if this.is_null() || eaten.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_key(this) };
    let key_code = wparam.0 as u32;
    let is_shift = unsafe { GetAsyncKeyState(0x10) } < 0;
    let is_ctrl = unsafe { GetAsyncKeyState(0x11) } < 0;
    let is_alt = unsafe { GetAsyncKeyState(0x12) } < 0;
    let ctrl_space = key_code == 0x20 && is_ctrl && !is_alt && !is_shift;

    let admission = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        Some((state.key_admission_enabled(), unsafe {
            ((*owner).mode.get(), (*owner).has_composition.get())
        }))
    }) {
        Some(Some((activated, (mode, has_comp)))) => check_key(
            mode,
            activated,
            key_code,
            is_shift,
            is_ctrl || ctrl_space,
            is_alt,
            has_comp,
        ),
        _ => KeyAdmission::PassThrough,
    };

    match admission {
        KeyAdmission::Handled | KeyAdmission::ToggleMode => unsafe { *eaten = BOOL(1) },
        KeyAdmission::PassThrough => unsafe { *eaten = BOOL(0) },
    }
    S_OK
}

/// `OnKeyDown` — actually process the key (send to engine, toggle mode, etc.).
unsafe extern "system" fn key_down(
    this: *mut c_void,
    _: *mut c_void,
    wparam: WPARAM,
    _lparam: LPARAM,
    eaten: *mut BOOL,
) -> HRESULT {
    if this.is_null() || eaten.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_key(this) };
    let key_code = wparam.0 as u32;
    let is_shift = unsafe { GetAsyncKeyState(0x10) } < 0;
    let is_ctrl = unsafe { GetAsyncKeyState(0x11) } < 0;
    let is_alt = unsafe { GetAsyncKeyState(0x12) } < 0;
    let ctrl_space = key_code == 0x20 && is_ctrl && !is_alt && !is_shift;

    let admission = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        Some((state.key_admission_enabled(), unsafe {
            ((*owner).mode.get(), (*owner).has_composition.get())
        }))
    }) {
        Some(Some((activated, (mode, has_comp)))) => check_key(
            mode,
            activated,
            key_code,
            is_shift,
            is_ctrl || ctrl_space,
            is_alt,
            has_comp,
        ),
        _ => KeyAdmission::PassThrough,
    };

    // --- Digit key: commit candidate directly in TIP layer (before engine) ---
    let is_digit = (0x30..=0x39).contains(&key_code) || (0x60..=0x69).contains(&key_code);
    if is_digit && unsafe { (*owner).has_composition.get() } {
        let digit_idx = if key_code >= 0x60 {
            key_code - 0x60
        } else {
            key_code - 0x30
        };
        let candidate_offset = if digit_idx == 0 {
            9
        } else {
            (digit_idx as usize).saturating_sub(1)
        };
        let ctx_ref = {
            let cw = unsafe { (*owner).candidate_window.try_borrow() };
            match cw.as_ref().ok().and_then(|cw| cw.as_ref()) {
                Some(cw) => cw.ctx_ptr,
                _ => std::ptr::null(),
            }
        };
        tsf_log(&format!(
            "[CheIME] Digit{}: ctx_ref null={}",
            digit_idx,
            ctx_ref.is_null()
        ));
        if !ctx_ref.is_null() {
            let ctx = unsafe { &*ctx_ref };
            if let Ok(st) = ctx.snapshot.lock() {
                if let Some((snap, _)) = st.as_ref() {
                    if let Some(cand) = snap.candidates.get(candidate_offset) {
                        let action = PlatformAction {
                            id: cheime_model::ActionId::new(0),
                            epoch: snap.epoch,
                            revision: snap.revision,
                            kind: PlatformActionKind::Commit {
                                text: cand.text.clone(),
                            },
                        };
                        tsf_log(&format!(
                            "[CheIME] Digit{} commit (offset={}): {:?}",
                            digit_idx, candidate_offset, action.kind
                        ));
                        if let Ok(doc) = unsafe { ctx.thread_mgr.GetFocus() } {
                            if let Ok(context) = unsafe { doc.GetTop() } {
                                request_edit_session(
                                    ctx.client_id,
                                    &context,
                                    action,
                                    &ctx.channel as *const SyncSender<FrontendMessage>,
                                    &ctx.composition as *const Mutex<Option<ITfComposition>>,
                                );
                            }
                        }
                        unsafe { *eaten = BOOL(1) };
                        return S_OK;
                    }
                }
            }
        }
    }

    match admission {
        KeyAdmission::Handled => {
            if let Ok(channel) = unsafe { (*owner).channel.try_borrow() } {
                if let Some(ref channel) = *channel {
                    let key = vk_to_key(key_code);
                    let state = KeyState {
                        shift: is_shift,
                        control: is_ctrl,
                        alt: is_alt,
                    };
                    tsf_log(&format!(
                        "[CheIME] OnKeyDown vk={key_code:#04x} key={key:?} mode={:?}",
                        unsafe { (*owner).mode.get() }
                    ));
                    tsf_log(&format!(
                        "[CheIME] KeyCommand sending vk={key_code:#04x} key={key:?}"
                    ));
                    let send_result = channel.try_send(FrontendMessage::KeyCommand {
                        header: cheime_protocol::MessageHeader {
                            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
                            client: cheime_model::ClientInstanceId::new(1),
                            session: cheime_model::SessionId::new(1),
                            epoch: cheime_model::SessionEpoch::new(1),
                            sequence: cheime_model::Sequence::new(0),
                            revision: cheime_model::Revision::new(0),
                            deployment: cheime_model::DeploymentGeneration::new(1),
                        },
                        event: KeyEvent { key, state },
                    });
                    tsf_log(&format!(
                        "[CheIME] KeyCommand sent vk={key_code:#04x} result={send_result:?}"
                    ));
                }
            }
            unsafe { *eaten = BOOL(1) };
            S_OK
        }
        KeyAdmission::ToggleMode => {
            unsafe {
                let prev = (*owner).mode.get();
                (*owner).mode.set(match prev {
                    InputMode::Chinese => InputMode::Direct,
                    InputMode::Direct => InputMode::Chinese,
                });
                // Reset composition tracking on toggle so empty backspace passes through
                (*owner).has_composition.set(false);
                tsf_log(&format!(
                    "[CheIME] ToggleMode {:?} → {:?}",
                    prev,
                    (*owner).mode.get()
                ));
            };
            unsafe { *eaten = BOOL(1) };
            S_OK
        }
        KeyAdmission::PassThrough => {
            unsafe { *eaten = BOOL(0) };
            S_OK
        }
    }
}

/// `OnKeyUp` — always pass through (we already handled the key on `OnKeyDown`).
unsafe extern "system" fn key_up(
    _this: *mut c_void,
    _: *mut c_void,
    _wparam: WPARAM,
    _lparam: LPARAM,
    eaten: *mut BOOL,
) -> HRESULT {
    if eaten.is_null() {
        return E_POINTER;
    }
    unsafe { *eaten = BOOL(0) };
    S_OK
}

/// Convert a Windows virtual key code to a `Key` for protocol messages.
fn vk_to_key(vk: u32) -> Key {
    match vk {
        0x08 => Key::Backspace,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x41..=0x5A => Key::Character(((vk - 0x41) as u8 + b'a') as char),
        0x30..=0x39 => Key::Character(((vk - 0x30) as u8 + b'0') as char),
        0x60..=0x69 => Key::Character(((vk - 0x60) as u8 + b'0') as char),
        0xBC => Key::Character(','),
        0xBE => Key::Character('.'),
        0xBA => Key::Character(';'),
        0xBF => Key::Character('/'),
        0xBB => Key::Character('='),
        0xBD => Key::Character('-'),
        0xDB => Key::Character('['),
        0xDD => Key::Character(']'),
        0xDC => Key::Character('\\'),
        0xC0 => Key::Character('`'),
        0xDE => Key::Character('\''),
        _ => Key::Character('?'),
    }
}
unsafe extern "system" fn preserved_key(
    _: *mut c_void,
    _: *mut c_void,
    _: *const GUID,
    eaten: *mut BOOL,
) -> HRESULT {
    if eaten.is_null() {
        return E_POINTER;
    }
    unsafe { *eaten = BOOL(0) };
    S_OK
}

static KEY_VTBL: ITfKeyEventSink_Vtbl = ITfKeyEventSink_Vtbl {
    base__: unknown_vtbl(key_qi, key_add_ref, key_release),
    OnSetFocus: key_focus,
    OnTestKeyDown: test_key,
    OnTestKeyUp: test_key,
    OnKeyDown: key_down,
    OnKeyUp: key_up,
    OnPreservedKey: preserved_key,
};

fn canonical_identity<T: Interface>(value: &T) -> Option<usize> {
    value
        .cast::<IUnknown>()
        .ok()
        .map(|unknown| unknown.as_raw() as usize)
}

unsafe fn document_from_raw(raw: *mut c_void) -> Option<ITfDocumentMgr> {
    unsafe { ITfDocumentMgr::from_raw_borrowed(&raw) }.cloned()
}

unsafe fn context_from_raw(raw: *mut c_void) -> Option<ITfContext> {
    unsafe { ITfContext::from_raw_borrowed(&raw) }.cloned()
}

unsafe fn set_runtime_focus(
    owner: *mut ComTip,
    ticket: crate::runtime::FocusTicket,
    document: Option<ITfDocumentMgr>,
) -> HRESULT {
    let identity = document.as_ref().and_then(canonical_identity);
    let context = document
        .as_ref()
        .and_then(|document| unsafe { document.GetTop() }.ok());
    let resources = FocusResources {
        document,
        document_identity: identity,
        context,
    };
    let result = ApartmentState::try_with_owned(
        unsafe { &(*owner).runtime },
        resources,
        |state, resources| state.set_focus_if_current(ticket, resources),
    );
    match result {
        Ok(Ok(old)) => {
            drop(old);
            S_OK
        }
        Ok(Err(rejected)) | Err(rejected) => {
            drop(rejected);
            E_UNEXPECTED
        }
    }
}

unsafe extern "system" fn init_document(_: *mut c_void, _: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn uninit_document(this: *mut c_void, document: *mut c_void) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_thread_mgr(this) };
    let ticket = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_focus_update()
    }) {
        Some(ticket) => ticket,
        None => return E_UNEXPECTED,
    };
    let identity = unsafe { document_from_raw(document) }
        .as_ref()
        .and_then(canonical_identity);
    let old = ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.clear_if_document_identity_current(ticket, identity)
    });
    match old {
        Some(Some(old)) => {
            drop(old);
            // Hide candidate window when document is uninitialized
            if let Ok(cw) = (*owner).candidate_window.try_borrow() {
                if let Some(ref cw) = *cw {
                    cw.hide();
                }
            }
            S_OK
        }
        Some(None) => E_UNEXPECTED,
        None => E_UNEXPECTED,
    }
}

unsafe extern "system" fn document_focus(
    this: *mut c_void,
    focused: *mut c_void,
    _: *mut c_void,
) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_thread_mgr(this) };
    let ticket = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_focus_update()
    }) {
        Some(ticket) => ticket,
        None => return E_UNEXPECTED,
    };
    unsafe { set_runtime_focus(owner, ticket, document_from_raw(focused)) }
}

unsafe extern "system" fn push_context(this: *mut c_void, context: *mut c_void) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_thread_mgr(this) };
    let ticket = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_focus_update()
    }) {
        Some(ticket) => ticket,
        None => return E_UNEXPECTED,
    };
    let context = unsafe { context_from_raw(context) };
    let context_document = context
        .as_ref()
        .and_then(|context| unsafe { context.GetDocumentMgr() }.ok());
    let context_identity = context_document.as_ref().and_then(canonical_identity);
    drop(context_document);
    let replaced =
        ApartmentState::try_with_owned(unsafe { &(*owner).runtime }, context, |state, context| {
            state.replace_context_if_current(ticket, context_identity, context)
        });
    match replaced {
        Ok(Ok(old)) => {
            drop(old);
            S_OK
        }
        Ok(Err(rejected)) | Err(rejected) => {
            drop(rejected);
            S_OK
        }
    }
}

unsafe extern "system" fn pop_context(this: *mut c_void, context: *mut c_void) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let owner = unsafe { owner_from_thread_mgr(this) };
    let ticket = match ApartmentState::try_with(unsafe { &(*owner).runtime }, |state| {
        state.begin_focus_update()
    }) {
        Some(ticket) => ticket,
        None => return E_UNEXPECTED,
    };
    let popped = unsafe { context_from_raw(context) };
    let document = popped
        .as_ref()
        .and_then(|context| unsafe { context.GetDocumentMgr() }.ok());
    let identity = document.as_ref().and_then(canonical_identity);
    let top = document
        .as_ref()
        .and_then(|document| unsafe { document.GetTop() }.ok());
    drop(popped);
    drop(document);
    let replaced =
        ApartmentState::try_with_owned(unsafe { &(*owner).runtime }, top, |state, top| {
            state.replace_context_if_current(ticket, identity, top)
        });
    match replaced {
        Ok(Ok(old)) => {
            drop(old);
            S_OK
        }
        Ok(Err(rejected)) | Err(rejected) => {
            drop(rejected);
            S_OK
        }
    }
}

static THREAD_MGR_VTBL: ITfThreadMgrEventSink_Vtbl = ITfThreadMgrEventSink_Vtbl {
    base__: unknown_vtbl(tm_qi, tm_add_ref, tm_release),
    OnInitDocumentMgr: init_document,
    OnUninitDocumentMgr: uninit_document,
    OnSetFocus: document_focus,
    OnPushContext: push_context,
    OnPopContext: pop_context,
};

unsafe extern "system" fn composition_terminated(
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
) -> HRESULT {
    S_OK
}

static COMPOSITION_VTBL: ITfCompositionSink_Vtbl = ITfCompositionSink_Vtbl {
    base__: unknown_vtbl(comp_qi, comp_add_ref, comp_release),
    OnCompositionTerminated: composition_terminated,
};

unsafe extern "system" fn enum_display(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = null_mut() };
    E_NOTIMPL
}
unsafe extern "system" fn get_display(
    _: *mut c_void,
    guid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if guid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = null_mut() };
    E_NOTIMPL
}

static DISPLAY_VTBL: ITfDisplayAttributeProvider_Vtbl = ITfDisplayAttributeProvider_Vtbl {
    base__: unknown_vtbl(display_qi, display_add_ref, display_release),
    EnumDisplayAttributeInfo: enum_display,
    GetDisplayAttributeInfo: get_display,
};

/// # Safety
///
/// Caller must pass valid GUID pointers; `out` receives exactly one AddRef'd reference
/// on success or is set to null.
pub unsafe fn create_instance(iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = null_mut() };
    if iid.is_null() {
        return E_POINTER;
    }
    let owner = Box::into_raw(ComTip::new());
    let hr = unsafe { query_owner(owner, iid, out) };
    unsafe { release_owner(owner) };
    hr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::{live_object_count, test_counter_guard};
    use std::ptr::{dangling_mut, null};

    const UNKNOWN: GUID = GUID::from_u128(0xdeadbeef_0000_0000_0000_000000000000);

    unsafe fn qi(this: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
        let vtbl = unsafe { *this.cast::<*const IUnknown_Vtbl>() };
        let mut out = null_mut();
        let hr = unsafe { ((*vtbl).QueryInterface)(this, iid, &mut out) };
        (hr, out)
    }

    unsafe fn release(this: *mut c_void) -> u32 {
        let vtbl = unsafe { *this.cast::<*const IUnknown_Vtbl>() };
        unsafe { ((*vtbl).Release)(this) }
    }

    #[test]
    fn layout_and_each_owner_recovery_are_exact() {
        let _guard = test_counter_guard();
        let tip = ComTip::new();
        let owner = (&*tip as *const ComTip).cast_mut();
        assert_eq!(std::mem::offset_of!(ComTip, primary), 0);
        unsafe {
            assert_eq!(
                owner_from_primary((&tip.primary as *const PrimaryHeader).cast_mut().cast()),
                owner
            );
            assert_eq!(
                owner_from_key((&tip.key as *const KeyHeader).cast_mut().cast()),
                owner
            );
            assert_eq!(
                owner_from_thread_mgr(
                    (&tip.thread_mgr as *const ThreadMgrHeader)
                        .cast_mut()
                        .cast()
                ),
                owner
            );
            assert_eq!(
                owner_from_composition(
                    (&tip.composition as *const CompositionHeader)
                        .cast_mut()
                        .cast()
                ),
                owner
            );
            assert_eq!(
                owner_from_display((&tip.display as *const DisplayHeader).cast_mut().cast()),
                owner
            );
        }
    }

    #[test]
    fn reentrant_focus_callbacks_fail_before_touching_incoming_com_pointers() {
        let _guard = test_counter_guard();
        let tip = ComTip::new();
        let owner = Box::into_raw(tip);
        let thread = unsafe { ComTip::interface(owner, &IID_TM).unwrap() };
        let held = unsafe { (*owner).runtime.borrow_mut() };
        assert_eq!(
            unsafe { document_focus(thread, dangling_mut(), null_mut()) },
            E_UNEXPECTED
        );
        assert_eq!(
            unsafe { uninit_document(thread, dangling_mut()) },
            E_UNEXPECTED
        );
        assert_eq!(
            unsafe { push_context(thread, dangling_mut()) },
            E_UNEXPECTED
        );
        assert_eq!(unsafe { pop_context(thread, dangling_mut()) }, E_UNEXPECTED);
        drop(held);
        assert_eq!(unsafe { release(thread) }, 0);
    }

    #[test]
    fn reentrant_deactivate_returns_failure_instead_of_success() {
        let _guard = test_counter_guard();
        let tip = ComTip::new();
        let owner = Box::into_raw(tip);
        let primary = unsafe { ComTip::interface(owner, &IID_TIP_EX).unwrap() };
        let held = unsafe { (*owner).runtime.borrow_mut() };
        assert_eq!(unsafe { deactivate(primary) }, E_UNEXPECTED);
        drop(held);
        assert_eq!(unsafe { release(primary) }, 0);
    }

    #[test]
    fn activation_rejects_null_manager_without_changing_state() {
        let _guard = test_counter_guard();
        let tip = ComTip::new();
        let owner = Box::into_raw(tip);
        let primary = unsafe { ComTip::interface(owner, &IID_TIP_EX).unwrap() };
        assert_eq!(unsafe { activate(primary, null_mut(), 17) }, E_POINTER);
        assert!(!unsafe { (*owner).runtime.borrow().is_activated() });
        assert_eq!(unsafe { release(primary) }, 0);
    }

    #[test]
    fn new_tip_starts_with_inactive_runtime_state() {
        let _guard = test_counter_guard();
        let tip = ComTip::new();
        let state = tip.runtime.borrow();
        assert!(!state.is_activated());
        assert!(!state.key_admission_enabled());
        assert_eq!(state.activation_generation(), 0);
        assert_eq!(state.focus_generation(), 0);
        assert!(!state.has_focus());
    }

    #[test]
    fn qi_matrix_is_symmetric_stable_and_has_canonical_identity() {
        let _guard = test_counter_guard();
        let owner = Box::into_raw(ComTip::new());
        let primary = unsafe { ComTip::interface(owner, &IID_TIP_EX).unwrap() };
        let iids = [
            IUnknown::IID,
            IID_TIP,
            IID_TIP_EX,
            IID_KEY,
            IID_TM,
            IID_COMP,
            IID_DA,
        ];
        let mut interfaces = Vec::new();
        for iid in iids {
            let (hr, ptr) = unsafe { qi(primary, &iid) };
            assert_eq!(hr, S_OK);
            let (_, stable) = unsafe { qi(ptr, &iid) };
            assert_eq!(stable, ptr);
            unsafe { release(stable) };
            let (_, identity) = unsafe { qi(ptr, &IUnknown::IID) };
            assert_eq!(identity, primary);
            unsafe { release(identity) };
            interfaces.push(ptr);
        }
        for &from in &interfaces {
            for iid in iids {
                let (hr, ptr) = unsafe { qi(from, &iid) };
                assert_eq!(hr, S_OK);
                unsafe { release(ptr) };
            }
        }
        for ptr in interfaces.into_iter().rev() {
            unsafe { release(ptr) };
        }
        assert_eq!(unsafe { release_owner(owner) }, 0);
    }

    #[test]
    fn arbitrary_secondary_can_perform_final_release_and_drop_once() {
        let _guard = test_counter_guard();
        let start = live_object_count();
        let owner = Box::into_raw(ComTip::new());
        let display = unsafe { ComTip::interface(owner, &IID_DA).unwrap() };
        assert_eq!(unsafe { release(display) }, 0);
        assert_eq!(live_object_count(), start);
    }

    #[test]
    fn cross_interface_release_order_keeps_single_owner_alive() {
        let _guard = test_counter_guard();
        let start = live_object_count();
        let owner = Box::into_raw(ComTip::new());
        let primary = unsafe { ComTip::interface(owner, &IID_TIP_EX).unwrap() };
        let (_, key) = unsafe { qi(primary, &IID_KEY) };
        let (_, composition) = unsafe { qi(key, &IID_COMP) };
        assert_eq!(unsafe { release(key) }, 2);
        assert_eq!(unsafe { release(primary) }, 1);
        assert_eq!(unsafe { release(composition) }, 0);
        assert_eq!(live_object_count(), start);
    }

    #[test]
    fn qi_rejects_null_and_unknown_and_clears_output() {
        let _guard = test_counter_guard();
        let owner = Box::into_raw(ComTip::new());
        let primary = unsafe { ComTip::interface(owner, &IID_TIP_EX).unwrap() };
        let mut out = dangling_mut::<c_void>();
        assert_eq!(unsafe { tip_qi(primary, null(), &mut out) }, E_POINTER);
        assert!(out.is_null());
        assert_eq!(
            unsafe { tip_qi(primary, &IUnknown::IID, null_mut()) },
            E_POINTER
        );
        let (hr, unknown) = unsafe { qi(primary, &UNKNOWN) };
        assert_eq!(hr, E_NOINTERFACE);
        assert!(unknown.is_null());
        assert_eq!(unsafe { release(primary) }, 0);
    }

    #[test]
    fn create_instance_hands_off_creator_reference_for_every_interface() {
        let _guard = test_counter_guard();
        let start = live_object_count();
        for iid in [IUnknown::IID, IID_TIP_EX, IID_KEY, IID_TM, IID_COMP, IID_DA] {
            let mut out = null_mut();
            assert_eq!(unsafe { create_instance(&iid, &mut out) }, S_OK);
            assert!(!out.is_null());
            assert_eq!(unsafe { release(out) }, 0);
        }
        let mut out = dangling_mut::<c_void>();
        assert_eq!(
            unsafe { create_instance(&UNKNOWN, &mut out) },
            E_NOINTERFACE
        );
        assert!(out.is_null());
        out = dangling_mut::<c_void>();
        assert_eq!(unsafe { create_instance(null(), &mut out) }, E_POINTER);
        assert!(out.is_null());
        assert_eq!(unsafe { create_instance(&IID_KEY, null_mut()) }, E_POINTER);
        assert_eq!(live_object_count(), start);
    }

    #[test]
    fn all_key_callbacks_remain_transparent() {
        let _guard = test_counter_guard();
        let owner = Box::into_raw(ComTip::new());
        // Deactivate to make key admission PassThrough for all callbacks.
        unsafe { (*owner).runtime.get_mut().deactivate() };
        let key = unsafe { ComTip::interface(owner, &IID_KEY).unwrap() };
        for callback in [
            KEY_VTBL.OnTestKeyDown,
            KEY_VTBL.OnTestKeyUp,
            KEY_VTBL.OnKeyDown,
            KEY_VTBL.OnKeyUp,
        ] {
            let mut eaten = BOOL(1);
            assert_eq!(
                unsafe { callback(key, null_mut(), WPARAM(0x41), LPARAM(0), &mut eaten) },
                S_OK
            );
            assert_eq!(eaten, BOOL(0));
        }
        let mut eaten = BOOL(1);
        assert_eq!(
            unsafe { preserved_key(key, null_mut(), &IID_KEY, &mut eaten) },
            S_OK
        );
        assert_eq!(eaten, BOOL(0));
        assert_eq!(unsafe { release(key) }, 0);
    }

    #[test]
    fn key_callbacks_reject_null_eaten_pointer() {
        let _guard = test_counter_guard();
        let owner = Box::into_raw(ComTip::new());
        let key = unsafe { ComTip::interface(owner, &IID_KEY).unwrap() };
        assert_eq!(
            unsafe { test_key(key, null_mut(), WPARAM(0), LPARAM(0), null_mut()) },
            E_POINTER
        );
        assert_eq!(
            unsafe { preserved_key(key, null_mut(), &IID_KEY, null_mut()) },
            E_POINTER
        );
        assert_eq!(unsafe { release(key) }, 0);
    }

    #[test]
    fn display_provider_uses_exact_generated_abi() {
        let _guard = test_counter_guard();
        let _: ITfDisplayAttributeProvider_Vtbl = ITfDisplayAttributeProvider_Vtbl {
            base__: unknown_vtbl(display_qi, display_add_ref, display_release),
            EnumDisplayAttributeInfo: enum_display,
            GetDisplayAttributeInfo: get_display,
        };
        let owner = Box::into_raw(ComTip::new());
        let display = unsafe { ComTip::interface(owner, &IID_DA).unwrap() };
        let mut out = dangling_mut::<c_void>();
        assert_eq!(unsafe { enum_display(display, &mut out) }, E_NOTIMPL);
        assert!(out.is_null());
        assert_eq!(
            unsafe { get_display(display, &IID_DA, &mut out) },
            E_NOTIMPL
        );
        assert!(out.is_null());
        assert_eq!(unsafe { release(display) }, 0);
    }

    #[test]
    fn windows_interface_cast_clone_and_drop_use_our_vtables() {
        let _guard = test_counter_guard();
        let start = live_object_count();
        let mut raw = null_mut();
        assert_eq!(unsafe { create_instance(&IID_TIP_EX, &mut raw) }, S_OK);
        let tip = unsafe { ITfTextInputProcessorEx::from_raw(raw) };
        let key: ITfKeyEventSink = tip.cast().expect("QI through windows Interface::cast");
        let key_clone = key.clone();
        let unknown_from_tip: IUnknown = tip.cast().unwrap();
        let unknown_from_key: IUnknown = key.cast().unwrap();
        assert_eq!(unknown_from_tip.as_raw(), unknown_from_key.as_raw());
        drop(key_clone);
        drop(key);
        drop(unknown_from_key);
        drop(unknown_from_tip);
        drop(tip);
        assert_eq!(live_object_count(), start);
    }
}
