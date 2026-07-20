//! COM DLL exports and registration for the safe TSF shell.

use crate::class_factory::ClassFactory;
use crate::exports::{CHEIME_TIP_NAME, live_object_count, server_lock_count};
use std::ffi::c_void;
use std::path::PathBuf;
use windows::Win32::Foundation::{
    BOOL, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HMODULE, RPC_E_CHANGED_MODE, WIN32_ERROR,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::ITfInputProcessorProfileMgr;
use windows::core::{GUID, HRESULT, Interface, PCWSTR};

pub const CLSID_CHEIME_TIP: GUID = GUID::from_u128(0xB5F1C9A8_3E7D_4A15_AE2D_F89C1B6E3A07);
pub const GUID_PROFILE: GUID = GUID::from_u128(0xD7E2A3B4_C5F6_7890_ABCD_EF1234567890);
pub const GUID_TFCAT_TIP_KEYBOARD: GUID = GUID::from_u128(0x34745C63_B2F0_4784_8B67_5E12C8701A31);
pub const CLSID_TF_INPUTPROCESSORPROFILES: GUID =
    GUID::from_u128(0x33C53A50_F456_4884_B049_85FD643ECFED);
pub const PROFILE_LANGUAGE_ID: u16 = 0x0804;
pub const PROFILE_ENABLED: bool = true;

const CLSID_TEXT: &str = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}";
const PROFILE_TEXT: &str = "{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}";
const CATEGORY_TEXT: &str = "{34745C63-B2F0-4784-8B67-5E12C8701A31}";
const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);
const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);
const E_UNEXPECTED: HRESULT = HRESULT(0x8000_FFFFu32 as i32);
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x8004_0111u32 as i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegistryHive {
    ClassesRoot,
    LocalMachine,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegistryWrite {
    hive: RegistryHive,
    path: String,
    value_name: Option<String>,
    value: Option<String>,
}

impl RegistryWrite {
    fn string(hive: RegistryHive, path: &str, value_name: Option<&str>, value: &str) -> Self {
        Self {
            hive,
            path: path.into(),
            value_name: value_name.map(Into::into),
            value: Some(value.into()),
        }
    }

    fn key(hive: RegistryHive, path: &str) -> Self {
        Self {
            hive,
            path: path.into(),
            value_name: None,
            value: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegistryDelete {
    hive: RegistryHive,
    path: String,
}

struct RegistrationPlan {
    registry_writes: Vec<RegistryWrite>,
}

struct UnregistrationPlan {
    registry_deletes: Vec<RegistryDelete>,
}

fn registration_plan(dll_path: &str) -> RegistrationPlan {
    let clsid_path = format!("CLSID\\{CLSID_TEXT}");
    let inproc_path = format!("{clsid_path}\\InprocServer32");
    let metadata_path = format!(
        "SOFTWARE\\Microsoft\\CTF\\TIP\\{CLSID_TEXT}\\LanguageProfile\\0x00000804\\{PROFILE_TEXT}"
    );
    RegistrationPlan {
        registry_writes: vec![
            RegistryWrite::string(RegistryHive::ClassesRoot, &inproc_path, None, dll_path),
            RegistryWrite::string(
                RegistryHive::ClassesRoot,
                &inproc_path,
                Some("ThreadingModel"),
                "Apartment",
            ),
            RegistryWrite::key(
                RegistryHive::ClassesRoot,
                &format!("{clsid_path}\\Implemented Categories\\{CATEGORY_TEXT}"),
            ),
            RegistryWrite::string(
                RegistryHive::LocalMachine,
                &metadata_path,
                Some("Description"),
                CHEIME_TIP_NAME,
            ),
        ],
    }
}

fn unregistration_plan() -> UnregistrationPlan {
    UnregistrationPlan {
        registry_deletes: vec![
            RegistryDelete {
                hive: RegistryHive::ClassesRoot,
                path: format!("CLSID\\{CLSID_TEXT}"),
            },
            RegistryDelete {
                hive: RegistryHive::LocalMachine,
                path: format!("SOFTWARE\\Microsoft\\CTF\\TIP\\{CLSID_TEXT}"),
            },
        ],
    }
}

/// # Safety
///
/// Caller must pass valid GUID pointers or null; all arguments are pointer parameters
/// expected by COM.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    if unsafe { *rclsid } != CLSID_CHEIME_TIP {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory = Box::into_raw(ClassFactory::new());
    let hr = unsafe { ClassFactory::query_interface(factory, riid, ppv) };
    unsafe { ClassFactory::release(factory) };
    hr
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if live_object_count() == 0 && server_lock_count() == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    ffi_result(register_tip)
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    ffi_result(unregister_tip)
}

fn ffi_result(operation: impl FnOnce() -> windows::core::Result<()>) -> HRESULT {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
        .unwrap_or_else(|_| Err(windows::core::Error::from(E_UNEXPECTED)))
        .map(|()| S_OK)
        .unwrap_or_else(|error| error.code())
}

trait RegistrationExecutor {
    fn apply_registry_write(&mut self, write: &RegistryWrite) -> windows::core::Result<()>;
    fn delete_registry_tree(&mut self, delete: &RegistryDelete) -> windows::core::Result<()>;
    fn register_profile(&mut self) -> windows::core::Result<()>;
    fn unregister_profile(&mut self) -> windows::core::Result<()>;
}

struct SystemRegistrationExecutor;

impl RegistrationExecutor for SystemRegistrationExecutor {
    fn apply_registry_write(&mut self, write: &RegistryWrite) -> windows::core::Result<()> {
        apply_registry_write(write)
    }

    fn delete_registry_tree(&mut self, delete: &RegistryDelete) -> windows::core::Result<()> {
        delete_registry_tree(delete)
    }

    fn register_profile(&mut self) -> windows::core::Result<()> {
        register_profile()
    }

    fn unregister_profile(&mut self) -> windows::core::Result<()> {
        unregister_profile()
    }
}

fn register_with_executor(
    executor: &mut impl RegistrationExecutor,
    dll_path: &str,
) -> windows::core::Result<()> {
    let result = (|| {
        for write in registration_plan(dll_path).registry_writes {
            executor.apply_registry_write(&write)?;
        }
        executor.register_profile()
    })();
    if let Err(error) = result {
        let _ = unregister_with_executor(executor);
        Err(error)
    } else {
        Ok(())
    }
}

fn unregister_with_executor(executor: &mut impl RegistrationExecutor) -> windows::core::Result<()> {
    let mut first_error = executor.unregister_profile().err();
    for delete in unregistration_plan().registry_deletes {
        if let Err(error) = executor.delete_registry_tree(&delete) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn register_tip() -> windows::core::Result<()> {
    let dll_path = module_path_from_address(DllRegisterServer as *const c_void)?;
    register_with_executor(&mut SystemRegistrationExecutor, &dll_path.to_string_lossy())
}

fn unregister_tip() -> windows::core::Result<()> {
    unregister_with_executor(&mut SystemRegistrationExecutor)
}

fn module_path_from_address(address: *const c_void) -> windows::core::Result<PathBuf> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address.cast()),
            &mut module,
        )?;
    }
    let mut buffer = vec![0u16; 260];
    loop {
        let length = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
        if length == 0 {
            return Err(windows::core::Error::from_win32());
        }
        if length < buffer.len() - 1 {
            return Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == S_OK || result == S_FALSE {
            Ok(Self { uninitialize: true })
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self {
                uninitialize: false,
            })
        } else {
            Err(windows::core::Error::from_hresult(result))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn with_profile_manager(
    operation: impl FnOnce(&ITfInputProcessorProfileMgr) -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    let _apartment = ComApartment::initialize()?;
    let manager: ITfInputProcessorProfileMgr =
        unsafe { CoCreateInstance(&CLSID_TF_INPUTPROCESSORPROFILES, None, CLSCTX_INPROC_SERVER) }?;
    operation(&manager)
}

fn register_profile() -> windows::core::Result<()> {
    with_profile_manager(|manager| {
        let description: Vec<u16> = CHEIME_TIP_NAME.encode_utf16().collect();
        unsafe {
            manager.RegisterProfile(
                &CLSID_CHEIME_TIP,
                PROFILE_LANGUAGE_ID,
                &GUID_PROFILE,
                &description,
                &[],
                0,
                HKL(std::ptr::null_mut()),
                0,
                BOOL(PROFILE_ENABLED as i32),
                0,
            )?;
        }
        // RegisterProfile only makes the profile known system-wide.
        // EnableLanguageProfile activates it for the current user so it
        // appears as a usable input method (not just in the candidate list).
        // This method lives on ITfInputProcessorProfiles, reached via QI
        // from the same CLSID_TF_INPUTPROCESSORPROFILES coclass.
        let profiles: windows::Win32::UI::TextServices::ITfInputProcessorProfiles =
            manager.cast()?;
        unsafe {
            profiles.EnableLanguageProfile(
                &CLSID_CHEIME_TIP,
                PROFILE_LANGUAGE_ID,
                &GUID_PROFILE,
                BOOL(1),
            )
        }
    })
}

fn unregister_profile() -> windows::core::Result<()> {
    let result = with_profile_manager(|manager| unsafe {
        manager.UnregisterProfile(&CLSID_CHEIME_TIP, PROFILE_LANGUAGE_ID, &GUID_PROFILE, 0)
    });
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.code() == windows::Win32::Foundation::E_FAIL => {
            // Profile already removed — treat as success for idempotent uninstall.
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn apply_registry_write(write: &RegistryWrite) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    let path = wide(&write.path);
    let status = unsafe {
        RegCreateKeyExW(
            native_hive(write.hive),
            PCWSTR(path.as_ptr()),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    registry_status(status)?;

    let result = if let Some(value) = &write.value {
        let name = write.value_name.as_deref().map(wide);
        let data = wide(value);
        let name = name
            .as_ref()
            .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        registry_status(unsafe {
            RegSetValueExW(
                key,
                name,
                0,
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    data.as_ptr().cast(),
                    data.len() * 2,
                )),
            )
        })
    } else {
        Ok(())
    };
    unsafe {
        let _ = RegCloseKey(key);
    };
    result
}

fn delete_registry_tree(delete: &RegistryDelete) -> windows::core::Result<()> {
    let path = wide(&delete.path);
    let status = unsafe { RegDeleteTreeW(native_hive(delete.hive), PCWSTR(path.as_ptr())) };
    if status.is_ok() || status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(HRESULT::from_win32(
            status.0,
        )))
    }
}

fn registry_status(status: WIN32_ERROR) -> windows::core::Result<()> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(HRESULT::from_win32(
            status.0,
        )))
    }
}

fn native_hive(hive: RegistryHive) -> HKEY {
    match hive {
        RegistryHive::ClassesRoot => HKEY_CLASSES_ROOT,
        RegistryHive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::{lock_server, reset_counts, test_counter_guard, unlock_server};
    use std::ptr::dangling_mut;
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Vtbl};
    use windows::core::{IUnknown, Interface};

    #[test]
    fn dll_get_class_object_hands_off_one_factory_reference() {
        let _guard = test_counter_guard();
        reset_counts();
        let mut out = std::ptr::null_mut();
        assert_eq!(
            unsafe { DllGetClassObject(&CLSID_CHEIME_TIP, &IClassFactory::IID, &mut out) },
            S_OK
        );
        assert_eq!(live_object_count(), 1);
        drop(unsafe { IClassFactory::from_raw(out) });
        assert_eq!(live_object_count(), 0);
    }

    #[test]
    fn dll_get_class_object_clears_output_for_null_unknown_and_bad_iid() {
        let _guard = test_counter_guard();
        let unknown = GUID::from_u128(0xdeadbeef_0000_0000_0000_000000000000);
        let mut out = dangling_mut::<c_void>();
        assert_eq!(
            unsafe { DllGetClassObject(std::ptr::null(), &IUnknown::IID, &mut out) },
            E_POINTER
        );
        assert!(out.is_null());
        out = dangling_mut::<c_void>();
        assert_eq!(
            unsafe { DllGetClassObject(&unknown, &IUnknown::IID, &mut out) },
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert!(out.is_null());
        assert_eq!(
            unsafe { DllGetClassObject(&CLSID_CHEIME_TIP, &unknown, &mut out) },
            crate::tsf_interfaces::E_NOINTERFACE
        );
        assert!(out.is_null());
    }

    #[test]
    fn dll_can_unload_uses_separate_object_and_lock_counts() {
        let _guard = test_counter_guard();
        reset_counts();
        assert_eq!(DllCanUnloadNow(), S_OK);
        let mut out = std::ptr::null_mut();
        assert_eq!(
            unsafe { DllGetClassObject(&CLSID_CHEIME_TIP, &IClassFactory::IID, &mut out) },
            S_OK
        );
        assert_eq!(DllCanUnloadNow(), S_FALSE);
        let factory = out.cast::<*const IClassFactory_Vtbl>();
        let vtbl = unsafe { *factory };
        assert_eq!(unsafe { ((*vtbl).LockServer)(out, true.into()) }, S_OK);
        drop(unsafe { IClassFactory::from_raw(out) });
        assert_eq!(DllCanUnloadNow(), S_FALSE);
        unlock_server();
        assert_eq!(DllCanUnloadNow(), S_OK);

        lock_server();
        assert_eq!(DllCanUnloadNow(), S_FALSE);
        unlock_server();
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum RecordedOperation {
        Write(RegistryWrite),
        RegisterProfile,
        UnregisterProfile,
        Delete(RegistryDelete),
    }

    struct FakeExecutor {
        operations: Vec<RecordedOperation>,
        fail_at: Option<usize>,
    }

    impl FakeExecutor {
        fn succeeding() -> Self {
            Self {
                operations: Vec::new(),
                fail_at: None,
            }
        }

        fn failing_at(index: usize) -> Self {
            Self {
                operations: Vec::new(),
                fail_at: Some(index),
            }
        }

        fn record(&mut self, operation: RecordedOperation) -> windows::core::Result<()> {
            let index = self.operations.len();
            self.operations.push(operation);
            if self.fail_at == Some(index) {
                Err(windows::core::Error::from_hresult(HRESULT(
                    0x8000_4005u32 as i32,
                )))
            } else {
                Ok(())
            }
        }
    }

    impl RegistrationExecutor for FakeExecutor {
        fn apply_registry_write(&mut self, write: &RegistryWrite) -> windows::core::Result<()> {
            self.record(RecordedOperation::Write(write.clone()))
        }

        fn delete_registry_tree(&mut self, delete: &RegistryDelete) -> windows::core::Result<()> {
            self.record(RecordedOperation::Delete(delete.clone()))
        }

        fn register_profile(&mut self) -> windows::core::Result<()> {
            self.record(RecordedOperation::RegisterProfile)
        }

        fn unregister_profile(&mut self) -> windows::core::Result<()> {
            self.record(RecordedOperation::UnregisterProfile)
        }
    }

    #[test]
    fn registration_rolls_back_all_registration_surfaces_after_failure() {
        let mut executor = FakeExecutor::failing_at(4);
        let error = register_with_executor(&mut executor, r"C:\CheIME\cheime-tip.dll")
            .expect_err("profile failure must fail registration");
        assert_eq!(error.code(), HRESULT(0x8000_4005u32 as i32));
        assert_eq!(executor.operations.len(), 8);
        assert_eq!(executor.operations[4], RecordedOperation::RegisterProfile);
        assert_eq!(executor.operations[5], RecordedOperation::UnregisterProfile);
        assert!(matches!(
            executor.operations[6],
            RecordedOperation::Delete(_)
        ));
        assert!(matches!(
            executor.operations[7],
            RecordedOperation::Delete(_)
        ));
    }

    #[test]
    fn unregister_continues_cleanup_and_preserves_first_error() {
        let mut executor = FakeExecutor::failing_at(0);
        let error = unregister_with_executor(&mut executor)
            .expect_err("first cleanup error must be returned");
        assert_eq!(error.code(), HRESULT(0x8000_4005u32 as i32));
        assert_eq!(executor.operations.len(), 3);
        assert_eq!(executor.operations[0], RecordedOperation::UnregisterProfile);
        assert!(matches!(
            executor.operations[1],
            RecordedOperation::Delete(_)
        ));
        assert!(matches!(
            executor.operations[2],
            RecordedOperation::Delete(_)
        ));
    }

    #[test]
    fn unregister_is_successful_when_all_idempotent_cleanup_succeeds() {
        let mut executor = FakeExecutor::succeeding();
        unregister_with_executor(&mut executor).unwrap();
        assert_eq!(executor.operations.len(), 3);
    }

    #[test]
    fn registration_plan_uses_exact_hives_paths_and_values() {
        let plan = registration_plan(r"C:\Program Files\CheIME\cheime-tip.dll");
        assert_eq!(plan.registry_writes.len(), 4);
        assert_eq!(
            plan.registry_writes[0],
            RegistryWrite::string(
                RegistryHive::ClassesRoot,
                r"CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\InprocServer32",
                None,
                r"C:\Program Files\CheIME\cheime-tip.dll",
            )
        );
        assert_eq!(
            plan.registry_writes[1],
            RegistryWrite::string(
                RegistryHive::ClassesRoot,
                r"CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\InprocServer32",
                Some("ThreadingModel"),
                "Apartment",
            )
        );
        assert_eq!(
            plan.registry_writes[2],
            RegistryWrite::key(
                RegistryHive::ClassesRoot,
                r"CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\Implemented Categories\{34745C63-B2F0-4784-8B67-5E12C8701A31}",
            )
        );
        assert_eq!(
            plan.registry_writes[3],
            RegistryWrite::string(
                RegistryHive::LocalMachine,
                r"SOFTWARE\Microsoft\CTF\TIP\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}\LanguageProfile\0x00000804\{D7E2A3B4-C5F6-7890-ABCD-EF1234567890}",
                Some("Description"),
                "CheIME TIP",
            )
        );
        assert!(
            plan.registry_writes
                .iter()
                .all(|write| write.hive != RegistryHive::ClassesRoot
                    || !write.path.starts_with("SOFTWARE\\Microsoft\\CTF"))
        );
    }

    #[test]
    fn unregistration_plan_removes_only_clsid_and_machine_tip_tree() {
        assert_eq!(
            unregistration_plan().registry_deletes,
            vec![
                RegistryDelete {
                    hive: RegistryHive::ClassesRoot,
                    path: r"CLSID\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}".into(),
                },
                RegistryDelete {
                    hive: RegistryHive::LocalMachine,
                    path: r"SOFTWARE\Microsoft\CTF\TIP\{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
                        .into(),
                },
            ]
        );
    }

    #[test]
    fn profile_constants_describe_enabled_zh_cn_tip_profile() {
        assert_eq!(
            GUID_PROFILE,
            GUID::from_u128(0xD7E2A3B4_C5F6_7890_ABCD_EF1234567890)
        );
        assert_eq!(
            GUID_TFCAT_TIP_KEYBOARD,
            GUID::from_u128(0x34745C63_B2F0_4784_8B67_5E12C8701A31)
        );
        assert_eq!(PROFILE_LANGUAGE_ID, 0x0804);
        const { assert!(PROFILE_ENABLED) };
    }

    #[test]
    fn address_module_lookup_uses_the_register_export_address() {
        let module_path = module_path_from_address(DllRegisterServer as *const c_void).unwrap();
        let export_address = DllRegisterServer as *const c_void as usize;
        assert!(export_address != 0);
        assert!(module_path.is_absolute());
    }
}
