//! IClassFactory COM shell for creating inert TIP instances.

use crate::exports::{decrement_object_count, increment_object_count, lock_server, unlock_server};
use crate::tsf_interfaces::{self, E_NOINTERFACE, E_POINTER, S_OK};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Vtbl};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface};

const CLASS_E_NOAGGREGATION: HRESULT = HRESULT(0x8004_0110u32 as i32);
type RawComPtr = *mut c_void;

#[repr(C)]
pub struct ClassFactory {
    pub lp_vtbl: *const IClassFactory_Vtbl,
    ref_count: AtomicU32,
}

static CLASS_FACTORY_VTBL: IClassFactory_Vtbl = IClassFactory_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: class_factory_query_interface,
        AddRef: class_factory_add_ref,
        Release: class_factory_release,
    },
    CreateInstance: class_factory_create_instance,
    LockServer: class_factory_lock_server,
};

impl ClassFactory {
    pub fn new() -> Box<Self> {
        increment_object_count();
        Box::new(Self {
            lp_vtbl: &CLASS_FACTORY_VTBL,
            ref_count: AtomicU32::new(1),
        })
    }

    /// # Safety
    ///
    /// `this` must be a valid, live ClassFactory pointer; `riid` and `ppv` must be
    /// valid for the standard COM QueryInterface contract.
    pub unsafe fn query_interface(
        this: *mut Self,
        riid: *const GUID,
        ppv: *mut RawComPtr,
    ) -> HRESULT {
        unsafe { class_factory_query_interface(this.cast(), riid, ppv) }
    }

    /// # Safety
    ///
    /// `this` must be a valid, live ClassFactory pointer. After a call that
    /// returns 0 the pointer is invalidated.
    pub unsafe fn release(this: *mut Self) -> u32 {
        unsafe { class_factory_release(this.cast()) }
    }
}

unsafe extern "system" fn class_factory_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut RawComPtr,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = null_mut() };
    if this.is_null() || riid.is_null() {
        return E_POINTER;
    }
    let iid = unsafe { *riid };
    if iid == IUnknown::IID || iid == IClassFactory::IID {
        unsafe { class_factory_add_ref(this) };
        unsafe { *ppv = this };
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn class_factory_add_ref(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    unsafe {
        (*this.cast::<ClassFactory>())
            .ref_count
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}

unsafe extern "system" fn class_factory_release(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    let factory = this.cast::<ClassFactory>();
    let previous = unsafe { (*factory).ref_count.fetch_sub(1, Ordering::Release) };
    if previous == 1 {
        fence(Ordering::Acquire);
        decrement_object_count();
        unsafe { drop(Box::from_raw(factory)) };
        0
    } else {
        previous - 1
    }
}

unsafe extern "system" fn class_factory_create_instance(
    this: *mut c_void,
    outer: *mut c_void,
    riid: *const GUID,
    ppv: *mut RawComPtr,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = null_mut() };
    if this.is_null() || riid.is_null() {
        return E_POINTER;
    }
    if !outer.is_null() {
        return CLASS_E_NOAGGREGATION;
    }
    unsafe { tsf_interfaces::create_instance(riid, ppv) }
}

unsafe extern "system" fn class_factory_lock_server(
    this: *mut c_void,
    lock: windows::Win32::Foundation::BOOL,
) -> HRESULT {
    if this.is_null() {
        return E_POINTER;
    }
    if lock.as_bool() {
        lock_server();
    } else {
        unlock_server();
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::{live_object_count, reset_counts, server_lock_count, test_counter_guard};
    use crate::tsf_interfaces::{IID_KEY, IID_TIP_EX};
    use std::ptr::dangling_mut;

    #[test]
    fn create_instance_rejects_aggregation_and_handles_nulls() {
        let _guard = test_counter_guard();
        let factory = Box::into_raw(ClassFactory::new());
        let mut out = dangling_mut::<c_void>();
        assert_eq!(
            unsafe {
                class_factory_create_instance(
                    factory.cast(),
                    dangling_mut::<c_void>(),
                    &IID_TIP_EX,
                    &mut out,
                )
            },
            CLASS_E_NOAGGREGATION
        );
        assert!(out.is_null());
        out = dangling_mut::<c_void>();
        assert_eq!(
            unsafe {
                class_factory_create_instance(
                    factory.cast(),
                    null_mut(),
                    std::ptr::null(),
                    &mut out,
                )
            },
            E_POINTER
        );
        assert!(out.is_null());
        assert_eq!(
            unsafe {
                class_factory_create_instance(factory.cast(), null_mut(), &IID_KEY, null_mut())
            },
            E_POINTER
        );
        assert_eq!(unsafe { ClassFactory::release(factory) }, 0);
    }

    #[test]
    fn create_instance_hands_off_exactly_one_reference() {
        let _guard = test_counter_guard();
        reset_counts();
        let factory = Box::into_raw(ClassFactory::new());
        let after_factory = live_object_count();
        let mut out = null_mut();
        assert_eq!(
            unsafe {
                class_factory_create_instance(factory.cast(), null_mut(), &IID_KEY, &mut out)
            },
            S_OK
        );
        assert_eq!(live_object_count(), after_factory + 1);
        let after_create = live_object_count();
        let unknown = unsafe { IUnknown::from_raw(out) };
        drop(unknown);
        assert_eq!(live_object_count(), after_create - 1);
        assert_eq!(unsafe { ClassFactory::release(factory) }, 0);
        assert_eq!(live_object_count(), after_factory - 1);
    }

    #[test]
    fn factory_objects_and_server_locks_are_counted_separately() {
        let _guard = test_counter_guard();
        reset_counts();
        let factory = Box::into_raw(ClassFactory::new());
        assert_eq!(live_object_count(), 1);
        assert_eq!(
            unsafe { class_factory_lock_server(factory.cast(), true.into()) },
            S_OK
        );
        assert_eq!(server_lock_count(), 1);
        assert_eq!(live_object_count(), 1);
        assert_eq!(
            unsafe { class_factory_lock_server(factory.cast(), false.into()) },
            S_OK
        );
        assert_eq!(server_lock_count(), 0);
        assert_eq!(unsafe { ClassFactory::release(factory) }, 0);
        assert_eq!(live_object_count(), 0);
    }
}
