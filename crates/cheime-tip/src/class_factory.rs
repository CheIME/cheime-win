//! IClassFactory COM implementation for the TIP.
//!
//! The class factory is returned by `DllGetClassObject` and creates
//! TIP instances via `CreateInstance`. Hand-written COM vtable.

use crate::exports::{decrement_object_count, increment_object_count};
use crate::tip::CheimeTip;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{GUID, IUnknown, Interface};

// COM HRESULT constants (not exposed by windows-rs 0.58 under Win32::System::Com)
const S_OK: windows::core::HRESULT = windows::core::HRESULT(0);
const E_NOINTERFACE: windows::core::HRESULT = windows::core::HRESULT(0x8000_4002u32 as i32);
const CLASS_E_CLASSNOTAVAILABLE: windows::core::HRESULT =
    windows::core::HRESULT(0x8004_0111u32 as i32);

// Type aliases for raw COM pointer types
type RawComPtr = *mut std::ffi::c_void;

// ── IClassFactory vtable ─────────────────────────────────

#[repr(C)]
pub struct ClassFactoryVtbl {
    pub query_interface: unsafe extern "system" fn(
        *mut ClassFactory,
        *const GUID,
        *mut RawComPtr,
    ) -> windows::core::HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut ClassFactory) -> u32,
    pub release: unsafe extern "system" fn(*mut ClassFactory) -> u32,
    pub create_instance: unsafe extern "system" fn(
        *mut ClassFactory,
        *mut IUnknown,
        *const GUID,
        *mut RawComPtr,
    ) -> windows::core::HRESULT,
    pub lock_server: unsafe extern "system" fn(*mut ClassFactory, i32) -> windows::core::HRESULT,
}

#[repr(C)]
pub struct ClassFactory {
    pub lp_vtbl: *const ClassFactoryVtbl,
    ref_count: AtomicU32,
    server_locked: AtomicU32,
}

static CLASS_FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    query_interface: class_factory_query_interface,
    add_ref: class_factory_add_ref,
    release: class_factory_release,
    create_instance: class_factory_create_instance,
    lock_server: class_factory_lock_server,
};

impl ClassFactory {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            lp_vtbl: &CLASS_FACTORY_VTBL,
            ref_count: AtomicU32::new(1),
            server_locked: AtomicU32::new(0),
        })
    }
}

unsafe extern "system" fn class_factory_query_interface(
    this: *mut ClassFactory,
    riid: *const GUID,
    ppv: *mut RawComPtr,
) -> windows::core::HRESULT {
    if this.is_null() {
        return E_NOINTERFACE;
    }

    let this = unsafe { &*this };

    if unsafe { *riid } == IUnknown::IID || unsafe { *riid } == ICLASSFACTORY_IID {
        unsafe {
            *ppv = (this as *const ClassFactory).cast_mut().cast();
        }
        unsafe {
            class_factory_add_ref(this as *const ClassFactory as *mut ClassFactory);
        }
        S_OK
    } else {
        unsafe { *ppv = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn class_factory_add_ref(this: *mut ClassFactory) -> u32 {
    if this.is_null() {
        return 0;
    }
    let this = unsafe { &*this };
    this.ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn class_factory_release(this: *mut ClassFactory) -> u32 {
    if this.is_null() {
        return 0;
    }
    let this = unsafe { &*this };
    let prev = this.ref_count.fetch_sub(1, Ordering::Relaxed);
    if prev == 1 {
        decrement_object_count();
        unsafe {
            let _ = Box::from_raw(this as *const ClassFactory as *mut ClassFactory);
        }
        return 0;
    }
    prev - 1
}

unsafe extern "system" fn class_factory_create_instance(
    this: *mut ClassFactory,
    _punk_outer: *mut IUnknown,
    riid: *const GUID,
    ppv: *mut RawComPtr,
) -> windows::core::HRESULT {
    if this.is_null() {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let tip = CheimeTip::new();
    let tip_ptr: *mut CheimeTip = Box::into_raw(tip);

    if unsafe { *riid } == IUnknown::IID {
        unsafe {
            tip_ptr.as_mut().unwrap().add_ref();
            *ppv = tip_ptr.cast();
        }
        S_OK
    } else {
        unsafe {
            let _ = Box::from_raw(tip_ptr);
            *ppv = std::ptr::null_mut();
        }
        E_NOINTERFACE
    }
}

unsafe extern "system" fn class_factory_lock_server(
    this: *mut ClassFactory,
    lock: i32,
) -> windows::core::HRESULT {
    if this.is_null() {
        return E_NOINTERFACE;
    }
    let this = unsafe { &*this };
    if lock != 0 {
        this.server_locked.fetch_add(1, Ordering::Relaxed);
    } else {
        this.server_locked.fetch_sub(1, Ordering::Relaxed);
    }
    S_OK
}

// ── IID for IClassFactory: {00000001-0000-0000-C000-000000000046} ──
const ICLASSFACTORY_IID: GUID = GUID::from_u128(0x00000001_0000_0000_C000_000000000046_u128);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::live_object_count;

    #[test]
    fn new_factory_increments_object_count() {
        let start = live_object_count();
        let cf = ClassFactory::new();
        assert!(live_object_count() > start);
        let ptr = Box::into_raw(cf);
        unsafe { class_factory_release(ptr) };
    }

    #[test]
    fn add_ref_and_release_lifecycle() {
        let cf = ClassFactory::new();
        let ptr: *mut ClassFactory = Box::into_raw(cf);
        let prev = unsafe { class_factory_add_ref(ptr) };
        assert!(prev > 1);
        let after = unsafe { class_factory_release(ptr) };
        assert!(after > 0);
        unsafe { class_factory_release(ptr) };
    }

    #[test]
    fn query_interface_for_iunknown_succeeds() {
        let cf = ClassFactory::new();
        let ptr: *mut ClassFactory = Box::into_raw(cf);
        let mut ppv: RawComPtr = std::ptr::null_mut();
        let hr = unsafe { class_factory_query_interface(ptr, &IUnknown::IID, &mut ppv) };
        assert_eq!(hr, S_OK);
        assert!(!ppv.is_null());
        unsafe { class_factory_release(ptr) };
    }

    #[test]
    fn query_interface_for_unknown_iid_fails() {
        let cf = ClassFactory::new();
        let ptr: *mut ClassFactory = Box::into_raw(cf);
        let unknown_iid = GUID::from_u128(0xDEADBEEF_0000_0000_0000_000000000000_u128);
        let mut ppv: RawComPtr = std::ptr::null_mut();
        let hr = unsafe { class_factory_query_interface(ptr, &unknown_iid, &mut ppv) };
        assert_eq!(hr, E_NOINTERFACE);
        assert!(ppv.is_null());
        unsafe { class_factory_release(ptr) };
    }
}
