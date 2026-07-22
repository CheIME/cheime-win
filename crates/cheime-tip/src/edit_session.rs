//! Edit session helpers — applies PlatformActions inside real TSF edit sessions.
//!
//! Engine responses carry `PlatformAction`s (SetPreedit, Commit, CancelComposition).
//! These must be applied within a TSF edit session on the document context.
//! This module provides the logic that the UI thread executes when it receives
//! a `WM_CHEIME_ACTION` message.

use cheime_model::{
    PlatformAction, PlatformActionKind, PlatformActionOutcome, PlatformActionResult,
};
use cheime_protocol::FrontendMessage;
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use std::sync::mpsc::SyncSender;
use windows::Win32::Foundation::BOOL;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfEditSession,
    ITfEditSession_Vtbl, TF_ANCHOR_END, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_DEFAULT_SELECTION,
    TF_ES_READWRITE, TF_ES_SYNC, TF_SELECTION,
};
use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, Interface};

// Re-export tsf_log from parent module for use in edit session tracing.
use crate::tsf_interfaces::tsf_log;

pub const S_OK: HRESULT = HRESULT(0);
pub const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002u32 as i32);
pub const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);

// ── Payload ─────────────────────────────────────────────────────────────────

/// Data that the `DoEditSession` callback processes.
struct EditSessionData {
    context: ITfContext,
    action: PlatformAction,
    /// Raw pointer to a `SyncSender<FrontendMessage>` stored in `WindowContext`.
    /// Safe because the channel outlives all queued edit sessions.
    channel: *const SyncSender<FrontendMessage>,
    /// Raw pointer to the `Mutex<Option<ITfComposition>>` in `WindowContext`.
    /// Safe because the composition mutex outlives all queued edit sessions.
    composition: *const Mutex<Option<ITfComposition>>,
}

// ── COM callback object ─────────────────────────────────────────────────────

/// Heap-allocated `ITfEditSession` callback.
///
/// Layout: vtable pointer, ref-count, then the payload.  The payload is taken
/// (Option::take) on first `DoEditSession` call so repeated calls are no-ops.
#[repr(C)]
struct EditSessionCallback {
    vtbl: &'static ITfEditSession_Vtbl,
    ref_count: AtomicU32,
    data: Mutex<Option<EditSessionData>>,
}

impl EditSessionCallback {
    fn new(data: EditSessionData) -> Box<Self> {
        Box::new(Self {
            vtbl: &EDIT_SESSION_VTBL,
            ref_count: AtomicU32::new(1),
            data: Mutex::new(Some(data)),
        })
    }

    /// # Safety
    ///
    /// `this` must point to a live `EditSessionCallback`.
    unsafe fn from_raw(this: *mut c_void) -> *mut Self {
        this.cast()
    }

    /// # Safety
    ///
    /// Caller must have a valid reference (ref-count has not reached zero).
    unsafe fn add_ref(this: *mut c_void) -> u32 {
        let cb = unsafe { Self::from_raw(this) };
        unsafe { (*cb).ref_count.fetch_add(1, Ordering::Relaxed) + 1 }
    }

    /// # Safety
    ///
    /// Caller must own a reference.  Returns the new count (0 means freed).
    unsafe fn release(this: *mut c_void) -> u32 {
        let cb = unsafe { Self::from_raw(this) };
        let prev = unsafe { (*cb).ref_count.fetch_sub(1, Ordering::Release) };
        if prev == 1 {
            fence(Ordering::Acquire);
            unsafe { drop(Box::from_raw(cb)) };
            0
        } else {
            prev - 1
        }
    }

    /// # Safety
    ///
    /// `this`, `iid`, `out` must be valid.  Standard COM QI contract.
    unsafe fn query_interface(
        this: *mut c_void,
        iid: *const windows::core::GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        unsafe { *out = ptr::null_mut() };
        if this.is_null() || iid.is_null() {
            return E_POINTER;
        }
        let guid = unsafe { *iid };
        if guid == IUnknown::IID || guid == ITfEditSession::IID {
            unsafe { Self::add_ref(this) };
            unsafe { *out = this };
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
}

// ── Static vtable for ITfEditSession ────────────────────────────────────────

unsafe extern "system" fn es_qi(
    this: *mut c_void,
    iid: *const windows::core::GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    unsafe { EditSessionCallback::query_interface(this, iid, out) }
}

unsafe extern "system" fn es_add_ref(this: *mut c_void) -> u32 {
    unsafe { EditSessionCallback::add_ref(this) }
}

unsafe extern "system" fn es_release(this: *mut c_void) -> u32 {
    unsafe { EditSessionCallback::release(this) }
}

unsafe extern "system" fn es_do_edit_session(this: *mut c_void, ec: u32) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    let cb = unsafe { &mut *EditSessionCallback::from_raw(this) };
    let data = match cb.data.lock().ok().and_then(|mut opt| opt.take()) {
        Some(d) => d,
        None => return S_OK,
    };
    match &data.action.kind {
        PlatformActionKind::Commit { text } => {
            tsf_log(&format!("[CheIME] edit: Commit text={text:?}"));
            handle_commit(
                ec,
                &data.context,
                text,
                data.channel,
                data.composition,
                &data.action,
            )
        }
        PlatformActionKind::SetPreedit { text, cursor } => {
            tsf_log(&format!(
                "[CheIME] edit: SetPreedit text={text:?} cursor={cursor}"
            ));
            handle_set_preedit(
                ec,
                &data.context,
                text,
                *cursor,
                data.channel,
                data.composition,
                &data.action,
            )
        }
        PlatformActionKind::CancelComposition => {
            tsf_log("[CheIME] edit: CancelComposition");
            handle_cancel_composition(ec, data.composition, data.channel, &data.action)
        }
    }
}

static EDIT_SESSION_VTBL: ITfEditSession_Vtbl = ITfEditSession_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: es_qi,
        AddRef: es_add_ref,
        Release: es_release,
    },
    DoEditSession: es_do_edit_session,
};

// ── Action handlers ─────────────────────────────────────────────────────────

/// Handle `Commit`: replace composition text, end composition, send result.
/// Uses the active composition range (not cursor selection) so the correct text
/// is replaced even if the caret has moved.
fn handle_commit(
    ec: u32,
    context: &ITfContext,
    text: &str,
    channel_ptr: *const SyncSender<FrontendMessage>,
    composition_ptr: *const Mutex<Option<ITfComposition>>,
    action: &PlatformAction,
) -> HRESULT {
    tsf_log(&format!(
        "[CheIME] handle_commit START text={text:?} ec={ec}"
    ));
    let result = 'work: {
        let comp_mutex = unsafe { &*composition_ptr };
        let guard = match comp_mutex.lock() {
            Ok(g) => g,
            Err(_) => break 'work Err("composition mutex poisoned".into()),
        };
        let comp = match guard.as_ref() {
            Some(c) => c,
            None => {
                tsf_log(
                    "[CheIME] commit: NO ACTIVE COMPOSITION — falling back to commit_at_selection",
                );
                break 'work commit_at_selection(ec, context, text);
            }
        };
        let range = match unsafe { comp.GetRange() } {
            Ok(r) => r,
            Err(e) => break 'work Err(format!("GetRange: {e}")),
        };

        let text_wide: Vec<u16> = text.encode_utf16().collect();
        if let Err(e) = unsafe { range.SetText(ec, 0, &text_wide) } {
            break 'work Err(format!("SetText: {e}"));
        }
        if let Err(e) = unsafe { range.Collapse(ec, TF_ANCHOR_END) } {
            break 'work Err(format!("Collapse: {e}"));
        }

        // Position cursor at end of committed text.
        let sel = TF_SELECTION {
            range: std::mem::ManuallyDrop::new(Some(range)),
            style: windows::Win32::UI::TextServices::TF_SELECTIONSTYLE {
                ase: windows::Win32::UI::TextServices::TfActiveSelEnd(0),
                fInterimChar: BOOL(0),
            },
        };
        if let Err(e) = unsafe { context.SetSelection(ec, &[sel]) } {
            break 'work Err(format!("SetSelection: {e}"));
        }

        drop(guard);
        match end_active_composition(ec, composition_ptr) {
            Ok(()) => {
                tsf_log("[CheIME] handle_commit: end_active_composition OK");
            }
            Err(e) => {
                tsf_log(&format!(
                    "[CheIME] handle_commit: end_active_composition FAILED: {e}"
                ));
                break 'work Err(e);
            }
        }
        tsf_log("[CheIME] handle_commit SUCCESS");
        Ok(())
    };

    tsf_log(&format!("[CheIME] handle_commit RESULT: {result:?}"));
    send_result(action, channel_ptr, &result);
    S_OK
}

/// Handle `SetPreedit`: start (or re-use) a composition and set its range text.
/// Uses the active composition range, or inserts at the cursor if no composition
/// exists yet. Always sends a result so the engine doesn't accumulate pending resolves.
fn handle_set_preedit(
    ec: u32,
    context: &ITfContext,
    text: &str,
    _cursor: usize,
    channel_ptr: *const SyncSender<FrontendMessage>,
    composition_ptr: *const Mutex<Option<ITfComposition>>,
    action: &PlatformAction,
) -> HRESULT {
    let result = 'work: {
        let composition_mutex = unsafe { &*composition_ptr };
        let mut comp_guard = match composition_mutex.lock() {
            Ok(g) => g,
            Err(_) => break 'work Err("composition mutex poisoned".into()),
        };

        let range = match comp_guard.as_ref() {
            // Active composition — get range from the composition object.
            Some(comp) => match unsafe { comp.GetRange() } {
                Ok(r) => r,
                Err(e) => break 'work Err(format!("GetRange: {e}")),
            },
            // No composition yet — get current selection as insertion point.
            None => {
                let mut selection = [zeroed_selection()];
                let mut fetched = 0u32;
                if unsafe {
                    context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
                }
                .is_err()
                {
                    release_selection_range(selection);
                    break 'work Err("GetSelection failed".into());
                }
                if fetched == 0 {
                    release_selection_range(selection);
                    break 'work Err("GetSelection fetched 0".into());
                }
                let sel_range = match selection[0].range.as_ref() {
                    Some(r) => r,
                    None => {
                        release_selection_range(selection);
                        break 'work Err("GetSelection returned None range".into());
                    }
                };
                let cloned = match unsafe { sel_range.Clone() } {
                    Ok(c) => c,
                    Err(e) => {
                        release_selection_range(selection);
                        break 'work Err(format!("Clone: {e}"));
                    }
                };
                release_selection_range(selection);

                let ctx_comp: ITfContextComposition = match context.cast() {
                    Ok(c) => c,
                    Err(_) => break 'work Err("cast to ITfContextComposition failed".into()),
                };
                *comp_guard = match unsafe {
                    ctx_comp.StartComposition(ec, &cloned, Option::<&ITfCompositionSink>::None)
                } {
                    Ok(c) => Some(c),
                    Err(e) => break 'work Err(format!("StartComposition: {e}")),
                };
                cloned
            }
        };

        // Set the preedit text into the composition range.
        let text_wide: Vec<u16> = text.encode_utf16().collect();
        if unsafe { range.SetText(ec, 0, &text_wide) }.is_err() {
            break 'work Err("SetText failed".into());
        }
        let _ = unsafe { range.Collapse(ec, TF_ANCHOR_END) };

        Ok(())
    };

    send_result(action, channel_ptr, &result);
    S_OK
}

/// Handle `CancelComposition`: clear preedit text, then end the composition.
/// Erases the preedit text *before* ending the composition to prevent text
/// from leaking into the document.
fn handle_cancel_composition(
    ec: u32,
    composition_ptr: *const Mutex<Option<ITfComposition>>,
    channel_ptr: *const SyncSender<FrontendMessage>,
    action: &PlatformAction,
) -> HRESULT {
    let result = 'work: {
        let comp_mutex = unsafe { &*composition_ptr };
        let guard = match comp_mutex.lock() {
            Ok(g) => g,
            Err(_) => break 'work Err("composition mutex poisoned".into()),
        };
        if let Some(comp) = guard.as_ref() {
            let range = match unsafe { comp.GetRange() } {
                Ok(r) => r,
                Err(e) => break 'work Err(format!("GetRange: {e}")),
            };
            // Set empty string to erase the preedit text before ending composition.
            if unsafe { range.SetText(ec, 0, &[]) }.is_err() {
                break 'work Err("SetText (cancel) failed".into());
            }
        }
        drop(guard);
        match end_active_composition(ec, composition_ptr) {
            Ok(()) => {
                tsf_log("[CheIME] handle_commit: end_active_composition OK");
            }
            Err(e) => {
                tsf_log(&format!(
                    "[CheIME] handle_commit: end_active_composition FAILED: {e}"
                ));
                break 'work Err(e);
            }
        }
        tsf_log("[CheIME] handle_commit SUCCESS");
        Ok(())
    };

    tsf_log(&format!("[CheIME] handle_commit RESULT: {result:?}"));
    send_result(action, channel_ptr, &result);
    S_OK
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Commit text at the current selection point without an active composition.
fn commit_at_selection(ec: u32, context: &ITfContext, text: &str) -> Result<(), String> {
    let mut selection = [zeroed_selection()];
    let mut fetched = 0u32;
    if unsafe { context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched) }
        .is_err()
    {
        release_selection_range(selection);
        return Err("GetSelection failed".into());
    }
    if fetched == 0 {
        release_selection_range(selection);
        return Err("GetSelection fetched 0".into());
    }
    let sel_range = match selection[0].range.as_ref() {
        Some(r) => r,
        None => {
            release_selection_range(selection);
            return Err("GetSelection returned None range".into());
        }
    };
    let text_wide: Vec<u16> = text.encode_utf16().collect();
    let set_result = unsafe { sel_range.SetText(ec, 0, &text_wide) };
    if let Err(e) = set_result {
        release_selection_range(selection);
        return Err(format!("SetText: {e}"));
    }
    let collapse_result = unsafe { sel_range.Collapse(ec, TF_ANCHOR_END) };
    if let Err(e) = collapse_result {
        release_selection_range(selection);
        return Err(format!("Collapse: {e}"));
    }
    release_selection_range(selection);
    Ok(())
}

/// End the active composition tracked in the composition mutex.
fn end_active_composition(
    ec: u32,
    composition_ptr: *const Mutex<Option<ITfComposition>>,
) -> Result<(), String> {
    let comp_mutex = unsafe { &*composition_ptr };
    let mut guard = match comp_mutex.lock() {
        Ok(g) => g,
        Err(_) => return Err("composition mutex poisoned".into()),
    };
    if let Some(comp) = guard.take() {
        unsafe {
            comp.EndComposition(ec)
                .map_err(|e| format!("EndComposition: {e}"))?;
        }
    }
    Ok(())
}

/// Send a `PlatformActionResult` through the channel.
fn send_result(
    action: &PlatformAction,
    channel_ptr: *const SyncSender<FrontendMessage>,
    result: &Result<(), String>,
) {
    let outcome = match result {
        Ok(()) => PlatformActionOutcome::Applied,
        Err(reason) => PlatformActionOutcome::Rejected {
            reason: reason.clone(),
        },
    };
    let result_msg = PlatformActionResult {
        action_id: action.id,
        outcome,
    };
    let header = cheime_protocol::MessageHeader {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client: cheime_model::ClientInstanceId::new(1),
        session: cheime_model::SessionId::new(1),
        epoch: action.epoch,
        sequence: cheime_model::Sequence::new(0),
        revision: action.revision,
        deployment: cheime_model::DeploymentGeneration::new(1),
    };
    if let Some(channel) = unsafe { channel_ptr.as_ref() } {
        let _ = channel.try_send(FrontendMessage::PlatformActionResult {
            header,
            result: result_msg,
        });
    }
}

/// Create a zeroed `TF_SELECTION` (range is `ManuallyDrop<Option<ITfRange>>`).
fn zeroed_selection() -> TF_SELECTION {
    TF_SELECTION {
        range: std::mem::ManuallyDrop::new(None),
        style: windows::Win32::UI::TextServices::TF_SELECTIONSTYLE {
            ase: windows::Win32::UI::TextServices::TfActiveSelEnd(0),
            fInterimChar: BOOL(0),
        },
    }
}

/// Extract and Release the ITfRange from a TF_SELECTION to avoid leaking.
fn release_selection_range(mut sel: [TF_SELECTION; 1]) {
    unsafe {
        let range = ptr::read(&sel[0].range);
        ptr::write(&mut sel[0].range, std::mem::ManuallyDrop::new(None));
        drop(std::mem::ManuallyDrop::into_inner(range));
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Request a TSF edit session to apply `action` on the given `context`.
///
/// Called on the UI thread from the `WM_CHEIME_ACTION` handler.
///
/// * `client_id` — the TIP's client ID from `ActivateEx`.
/// * `context` — the focused `ITfContext` obtained via `ITfDocumentMgr::GetTop`.
/// * `action` — the platform action to apply.
/// * `channel` — raw pointer to the `SyncSender` (lives in `WindowContext`).
/// * `composition` — raw pointer to the `Mutex<Option<ITfComposition>>` for
///   tracking composition state.
pub fn request_edit_session(
    client_id: u32,
    context: &ITfContext,
    action: PlatformAction,
    channel: *const SyncSender<FrontendMessage>,
    composition: *const Mutex<Option<ITfComposition>>,
) {
    let channel_backup = channel;
    let action_backup = action.clone();
    let data = EditSessionData {
        context: context.clone(),
        action,
        channel,
        composition,
    };
    let callback = EditSessionCallback::new(data); // ref_count = 1

    // Convert to raw pointer — we manage lifetime via COM AddRef/Release.
    let raw = Box::into_raw(callback);
    let raw_void: *mut c_void = raw.cast();

    // Build a borrow for the ITfEditSession vtable to pass to RequestEditSession.
    let session_ref = match unsafe { ITfEditSession::from_raw_borrowed(&raw_void) } {
        Some(s) => s,
        None => {
            // Shouldn't happen since we just created the object, but handle gracefully.
            unsafe { EditSessionCallback::release(raw_void) };
            return;
        }
    };

    // Request a synchronous write edit session.
    let flags = TF_CONTEXT_EDIT_CONTEXT_FLAGS(TF_ES_SYNC.0 | TF_ES_READWRITE.0);
    tsf_log(&format!(
        "[CheIME] RequestEditSession: client_id={client_id} flags={flags:?}"
    ));
    let hr = unsafe { context.RequestEditSession(client_id, session_ref, flags) };
    tsf_log(&format!("[CheIME] RequestEditSession result: {hr:?}"));

    // Release our reference.  If TSF took a reference during the call it has
    // already released it by now (synchronous call), so the count reaches zero
    // and the object is freed.
    unsafe { EditSessionCallback::release(raw_void) };

    if hr.is_err() {
        // TSF rejected the edit session — report failure so the engine
        // doesn't wait indefinitely for a result.
        send_result(
            &action_backup,
            channel_backup,
            &Err(format!("RequestEditSession: {hr:?}")),
        );
    }
}
