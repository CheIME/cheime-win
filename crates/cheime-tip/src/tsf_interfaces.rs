//! TSF COM interface vtables for CheIME TIP.

use crate::tip::CheimeTip;
use std::ffi::c_void;
use windows::core::{GUID, HRESULT, IUnknown, Interface};
use windows::Win32::UI::TextServices::{ITfDocumentMgr, ITfThreadMgr, ITfClientId};

const S_OK: HRESULT = HRESULT(0);
const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x8000_4001u32 as i32);
type P = *mut c_void;

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    if this.is_null() { 0 } else { unsafe { (&*(this as *const CheimeTip)).add_ref() } }
}
unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    if this.is_null() { return 0; }
    let r = unsafe { (&*(this as *const CheimeTip)).release() };
    if r == 0 { unsafe { let _ = Box::from_raw(this as *mut CheimeTip); } }
    r
}
fn qi(this: *mut c_void, riid: *const GUID, iid: &GUID, ppv: *mut P) -> HRESULT {
    if this.is_null() || ppv.is_null() { return E_NOINTERFACE; }
    if unsafe { *riid } == IUnknown::IID || unsafe { *riid } == *iid {
        unsafe { (&*(this as *const CheimeTip)).add_ref(); }
        unsafe { *ppv = this; }
        S_OK
    } else { unsafe { *ppv = std::ptr::null_mut(); } E_NOINTERFACE }
}

// ITfTextInputProcessorEx
#[repr(C)] pub struct TIPVtbl {
    q: unsafe extern "system" fn(*mut c_void, *const GUID, *mut P) -> HRESULT,
    a: unsafe extern "system" fn(*mut c_void) -> u32,
    r: unsafe extern "system" fn(*mut c_void) -> u32,
    act: unsafe extern "system" fn(*mut c_void, *mut ITfThreadMgr, ITfClientId) -> HRESULT,
    deact: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}
unsafe extern "system" fn tip_qi(t: *mut c_void, ri: *const GUID, p: *mut P) -> HRESULT { qi(t, ri, &IID_TIP, p) }
unsafe extern "system" fn tip_act(t: *mut c_void, ptim: *mut ITfThreadMgr, _tid: ITfClientId) -> HRESULT {
    unsafe { (&mut *(t as *mut CheimeTip)).activate(); }
    if !ptim.is_null() { unsafe { (&*(t as *const CheimeTip)).set_thread_mgr(ptim); } }
    S_OK
}
unsafe extern "system" fn tip_deact(t: *mut c_void) -> HRESULT { unsafe { (&mut *(t as *mut CheimeTip)).deactivate(); } S_OK }
static TIP_VTBL: TIPVtbl = TIPVtbl { q: tip_qi, a: add_ref, r: release, act: tip_act, deact: tip_deact };

// ITfKeyEventSink
#[repr(C)] pub struct KeySinkVtbl {
    q: unsafe extern "system" fn(*mut c_void, *const GUID, *mut P) -> HRESULT,
    a: unsafe extern "system" fn(*mut c_void) -> u32, r: unsafe extern "system" fn(*mut c_void) -> u32,
    sf: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    tkd: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, i32, i32) -> HRESULT,
    tku: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, i32, i32) -> HRESULT,
    kd: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, i32, i32) -> HRESULT,
    ku: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, i32, i32) -> HRESULT,
    pk: unsafe extern "system" fn(*mut c_void, *const GUID, *mut i32) -> HRESULT,
}
unsafe extern "system" fn ks_qi(t: *mut c_void, ri: *const GUID, p: *mut P) -> HRESULT { qi(t, ri, &IID_KEY, p) }
unsafe extern "system" fn ks_sf(_: *mut c_void, _f: i32) -> HRESULT { S_OK }
unsafe extern "system" fn ks_tkd(t: *mut c_void, _pic: *mut c_void, wp: u32, _l: u32, pe: i32, _p: i32) -> HRESULT {
    unsafe { key_proc(t, wp, pe, false) }
}
unsafe extern "system" fn ks_tku(_: *mut c_void, _pic: *mut c_void, _w: u32, _l: u32, pe: i32, _p: i32) -> HRESULT {
    if pe != 0 { unsafe { *(pe as *mut i32) = 0; } } S_OK
}
unsafe extern "system" fn ks_kd(t: *mut c_void, _pic: *mut c_void, wp: u32, _l: u32, pe: i32, _p: i32) -> HRESULT {
    unsafe { key_proc(t, wp, pe, true) }
}
unsafe extern "system" fn ks_ku(_: *mut c_void, _pic: *mut c_void, _w: u32, _l: u32, pe: i32, _p: i32) -> HRESULT {
    if pe != 0 { unsafe { *(pe as *mut i32) = 0; } } S_OK
}
unsafe extern "system" fn ks_pk(_: *mut c_void, _g: *const GUID, _p: *mut i32) -> HRESULT { S_OK }
static KEY_SINK_VTBL: KeySinkVtbl = KeySinkVtbl {
    q: ks_qi, a: add_ref, r: release,
    sf: ks_sf, tkd: ks_tkd, tku: ks_tku, kd: ks_kd, ku: ks_ku, pk: ks_pk,
};

unsafe fn key_proc(this: *mut c_void, wparam: u32, pf_eaten: i32, do_handle: bool) -> HRESULT {
    let tip = unsafe { &*(this as *const CheimeTip) };
    let vk = wparam & 0xFF;
    let is_shift = (wparam & 0x100) != 0;
    let is_ctrl = (wparam & 0x200) != 0;
    let is_alt = (wparam & 0x400) != 0;
    let admission = tip.test_key(vk, is_shift, is_ctrl, is_alt);
    let eaten = !matches!(admission, crate::key_handler::KeyAdmission::PassThrough);
    if do_handle && eaten { tip.handle_key(vk, is_shift, is_ctrl, is_alt); }
    if pf_eaten != 0 { unsafe { *(pf_eaten as *mut i32) = if eaten { 1 } else { 0 }; } }
    S_OK
}

// ITfThreadMgrEventSink
#[repr(C)] pub struct ThreadMgrSinkVtbl {
    q: unsafe extern "system" fn(*mut c_void, *const GUID, *mut P) -> HRESULT,
    a: unsafe extern "system" fn(*mut c_void) -> u32, r: unsafe extern "system" fn(*mut c_void) -> u32,
    idm: unsafe extern "system" fn(*mut c_void, *mut ITfDocumentMgr) -> HRESULT,
    udm: unsafe extern "system" fn(*mut c_void, *mut ITfDocumentMgr) -> HRESULT,
    sf: unsafe extern "system" fn(*mut c_void, *mut ITfDocumentMgr, *mut ITfDocumentMgr) -> HRESULT,
}
unsafe extern "system" fn tm_qi(t: *mut c_void, ri: *const GUID, p: *mut P) -> HRESULT { qi(t, ri, &IID_TM, p) }
unsafe extern "system" fn tm_idm(_: *mut c_void, _: *mut ITfDocumentMgr) -> HRESULT { S_OK }
unsafe extern "system" fn tm_udm(_: *mut c_void, _: *mut ITfDocumentMgr) -> HRESULT { S_OK }
unsafe extern "system" fn tm_sf(_: *mut c_void, _: *mut ITfDocumentMgr, _: *mut ITfDocumentMgr) -> HRESULT { S_OK }
static THREAD_MGR_SINK_VTBL: ThreadMgrSinkVtbl = ThreadMgrSinkVtbl {
    q: tm_qi, a: add_ref, r: release, idm: tm_idm, udm: tm_udm, sf: tm_sf,
};

// ITfCompositionSink
#[repr(C)] pub struct CompSinkVtbl {
    q: unsafe extern "system" fn(*mut c_void, *const GUID, *mut P) -> HRESULT,
    a: unsafe extern "system" fn(*mut c_void) -> u32, r: unsafe extern "system" fn(*mut c_void) -> u32,
    oct: unsafe extern "system" fn(*mut c_void, ITfClientId, *mut c_void) -> HRESULT,
}
unsafe extern "system" fn comp_qi(t: *mut c_void, ri: *const GUID, p: *mut P) -> HRESULT { qi(t, ri, &IID_COMP, p) }
unsafe extern "system" fn comp_oct(_: *mut c_void, _: ITfClientId, _: *mut c_void) -> HRESULT { S_OK }
static COMP_SINK_VTBL: CompSinkVtbl = CompSinkVtbl { q: comp_qi, a: add_ref, r: release, oct: comp_oct };

// ITfDisplayAttributeProvider
#[repr(C)] pub struct DisplayAttrVtbl {
    q: unsafe extern "system" fn(*mut c_void, *const GUID, *mut P) -> HRESULT,
    a: unsafe extern "system" fn(*mut c_void) -> u32, r: unsafe extern "system" fn(*mut c_void) -> u32,
    edai: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    gdai: unsafe extern "system" fn(*mut c_void, *const GUID, *mut c_void, *mut u32) -> HRESULT,
}
unsafe extern "system" fn da_qi(t: *mut c_void, ri: *const GUID, p: *mut P) -> HRESULT { qi(t, ri, &IID_DA, p) }
unsafe extern "system" fn da_edai(_: *mut c_void, pp: *mut *mut c_void) -> HRESULT {
    if !pp.is_null() { unsafe { *pp = std::ptr::null_mut(); } } E_NOTIMPL
}
unsafe extern "system" fn da_gdai(_: *mut c_void, _: *const GUID, _: *mut c_void, _: *mut u32) -> HRESULT { E_NOTIMPL }
static DISPLAY_ATTR_VTBL: DisplayAttrVtbl = DisplayAttrVtbl { q: da_qi, a: add_ref, r: release, edai: da_edai, gdai: da_gdai };

// IIDs
pub const IID_TIP: GUID = GUID::from_u128(0xADF95808_5DBF_42E0_B160_57AEA76229D3_u128);
pub const IID_KEY: GUID = GUID::from_u128(0x9F629B0D_D351_4E7C_B6E8_1EB0D2C49D97_u128);
pub const IID_TM: GUID = GUID::from_u128(0x3D61BF12_78B0_4BC2_B7FC_1EBE211F749D_u128);
pub const IID_COMP: GUID = GUID::from_u128(0x86462810_593B_4916_9764_19C08E9CE110_u128);
pub const IID_DA: GUID = GUID::from_u128(0xA15FCEFE_9CB3_4EDB_9DE3_3F20339314DC_u128);

pub fn get_vtable_for_iid(iid: &GUID) -> Option<*const c_void> {
    if *iid == IUnknown::IID || *iid == IID_TIP { Some(&TIP_VTBL as *const TIPVtbl as *const c_void) }
    else if *iid == IID_KEY { Some(&KEY_SINK_VTBL as *const KeySinkVtbl as *const c_void) }
    else if *iid == IID_TM { Some(&THREAD_MGR_SINK_VTBL as *const ThreadMgrSinkVtbl as *const c_void) }
    else if *iid == IID_COMP { Some(&COMP_SINK_VTBL as *const CompSinkVtbl as *const c_void) }
    else if *iid == IID_DA { Some(&DISPLAY_ATTR_VTBL as *const DisplayAttrVtbl as *const c_void) }
    else { None }
}
