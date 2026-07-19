//! DLL export functions for COM registration and class object retrieval.
//!
//! These are the four entry points that COM and `regsvr32.exe` call:
//! - `DllGetClassObject` — returns the class factory for a given CLSID
//! - `DllCanUnloadNow` — tells COM whether the DLL can be safely unloaded
//! - `DllRegisterServer` — writes registry keys to register the TIP
//! - `DllUnregisterServer` — removes registry keys

use crate::class_factory::ClassFactory;
use crate::exports::{
    CHEIME_TIP_CLSID_STR, CHEIME_TIP_NAME, decrement_object_count, increment_object_count,
    live_object_count,
};
use std::ffi::c_void;
use windows::core::{GUID, HRESULT};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CLASSES_ROOT, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::TextServices::{
    ITfInputProcessorProfileMgr, TF_PROFILETYPE_INPUTPROCESSOR,
};

// ── COM GUIDs ──────────────────────────────────────────────────

/// TIP CLSID: {B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}
pub const CLSID_CHEIME_TIP: GUID = GUID::from_u128(0xB5F1C9A8_3E7D_4A15_AE2D_F89C1B6E3A07);

/// TSF profile GUID (distinct from CLSID): {D7E2A3B4-C5F6-7890-ABCD-EF1234567890}
pub const GUID_PROFILE: GUID = GUID::from_u128(0xD7E2A3B4_C5F6_7890_ABCD_EF1234567890);

/// GUID_TFCAT_TIP_KEYBOARD: {34745C63-B2F0-4784-8B67-5E12C8701A31}
pub const GUID_TFCAT_TIP_KEYBOARD: GUID = GUID::from_u128(0x34745C63_B2F0_4784_8B67_5E12C8701A31);

/// CLSID_TF_InputProcessorProfiles: {33C53A50-F456-4884-B049-85FD643ECFED}
pub const CLSID_TF_INPUTPROCESSORPROFILES: GUID = GUID::from_u128(0x33C53A50_F456_4884_B049_85FD643ECFED);

// ── DLL Exports ─────────────────────────────────────────────────

/// COM calls this to get the class factory for a specific CLSID.
///
/// We only support one CLSID — our TIP's CLSID.
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return windows::core::HRESULT(0x8000_4003u32 as i32); // E_POINTER
    }

    let clsid = unsafe { &*rclsid };
    if *clsid != CLSID_CHEIME_TIP {
        return windows::core::HRESULT(0x8004_0111u32 as i32); // CLASS_E_CLASSNOTAVAILABLE
    }

    let factory = ClassFactory::new();
    let factory_ptr: *mut ClassFactory = Box::into_raw(factory);

    // QueryInterface on the factory for the requested riid
    let hr = unsafe {
        ClassFactory::query_interface(factory_ptr, riid, ppv)
    };

    if hr.is_err() {
        // Release the factory if QI failed
        unsafe { let _ = Box::from_raw(factory_ptr); }
    }

    hr
}

/// Called by COM to check if the DLL can be unloaded.
/// Returns S_OK if no live objects remain, S_FALSE otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if live_object_count() == 0 {
        windows::core::HRESULT(0) // S_OK
    } else {
        windows::core::HRESULT(1) // S_FALSE
    }
}

/// Writes registry keys to register the TIP COM class and TSF profile.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register_tip() {
        Ok(()) => windows::core::HRESULT(0),
        Err(e) => {
            eprintln!("[cheime-tip] DllRegisterServer failed: {e}");
            windows::core::HRESULT(0x8000_FFFFu32 as i32) // E_UNEXPECTED
        }
    }
}

/// Removes registry keys for the TIP COM class and TSF profile.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unregister_tip() {
        Ok(()) => windows::core::HRESULT(0),
        Err(e) => {
            eprintln!("[cheime-tip] DllUnregisterServer failed: {e}");
            windows::core::HRESULT(0x8000_FFFFu32 as i32)
        }
    }
}

// ── Registration Helpers ────────────────────────────────────────

fn register_tip() -> Result<(), String> {
    let dll_path = get_dll_path()?;

    // 1. Register CLSID → InprocServer32
    let clsid_path = format!("CLSID\\{CHEIME_TIP_CLSID_STR}");
    let inproc_path = format!("{clsid_path}\\InprocServer32");

    write_reg_string(&clsid_path, "", CHEIME_TIP_NAME)?;
    write_reg_string(&inproc_path, "", &dll_path)?;
    write_reg_string(&inproc_path, "ThreadingModel", "Apartment")?;

    // 2. Register implemented categories (keyboard TIP)
    let cat_guid = guid_to_string(&GUID_TFCAT_TIP_KEYBOARD);
    write_reg_string(
        &format!("{clsid_path}\\Implemented Categories\\{cat_guid}"),
        "", "",
    )?;

    // 3. Register TSF profile
    let profile_guid = guid_to_string(&GUID_PROFILE);
    let tsf_path = format!(
        "SOFTWARE\\Microsoft\\CTF\\TIP\\{CHEIME_TIP_CLSID_STR}\\LanguageProfile\\0x00000804\\{profile_guid}"
    );
    write_reg_string(&tsf_path, "", CHEIME_TIP_NAME)?;
    write_reg_string(&tsf_path, "Description", "CheIME Chinese Input Method")?;
    write_reg_dword(&tsf_path, "EnableCategory", 1)?;

    // 4. Try ITfInputProcessorProfileMgr::RegisterProfile
    if let Err(e) = register_via_profile_mgr() {
        eprintln!("[cheime-tip] ITfInputProcessorProfileMgr::RegisterProfile failed: {e}");
        eprintln!("[cheime-tip] TIP may still work via direct registry registration.");
    }

    Ok(())
}

fn unregister_tip() -> Result<(), String> {
    let clsid_path = format!("CLSID\\{CHEIME_TIP_CLSID_STR}");
    let tsf_path = format!(
        "SOFTWARE\\Microsoft\\CTF\\TIP\\{CHEIME_TIP_CLSID_STR}"
    );

    // Delete the CLSID key (and all subkeys)
    delete_reg_key(&clsid_path)?;
    // Delete the TSF profile key
    delete_reg_key(&tsf_path)?;

    Ok(())
}

fn register_via_profile_mgr() -> Result<(), String> {
    let profile_mgr: ITfInputProcessorProfileMgr = unsafe {
        CoCreateInstance(&CLSID_TF_INPUTPROCESSORPROFILES, None, CLSCTX_INPROC_SERVER)
    }.map_err(|e| format!("CoCreateInstance failed: {e}"))?;

    let desc: Vec<u16> = CHEIME_TIP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let icon: Vec<u16> = vec![0]; // empty icon string
    let hkl_null: windows::Win32::UI::Input::KeyboardAndMouse::HKL = windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut());
    let b_true: windows::Win32::Foundation::BOOL = windows::Win32::Foundation::BOOL(1);

    unsafe {
        profile_mgr.RegisterProfile(
            &CLSID_CHEIME_TIP,
            0x0804,
            &GUID_PROFILE,
            &desc,
            &icon,
            0,
            hkl_null,
            0,
            b_true,
            0,
        )
    }.map_err(|e| format!("RegisterProfile failed: {e}"))?;

    Ok(())
}

// ── Registry helpers ────────────────────────────────────────────

fn write_reg_string(key: &str, value_name: &str, data: &str) -> Result<(), String> {
    let key_wide: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
    let data_wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();

    let mut hkey = windows::Win32::System::Registry::HKEY::default();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            windows::core::PCWSTR::from_raw(key_wide.as_ptr()),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if rc.is_err() {
        return Err(format!("RegCreateKeyExW({key}) failed: {rc:?}"));
    }

    let value_pcwstr = if value_name.is_empty() {
        None
    } else {
        Some(windows::core::PCWSTR::from_raw(value_wide.as_ptr()))
    };
    let value_ptr = value_pcwstr.map_or(std::ptr::null(), |p| unsafe { p.as_wide().as_ptr() });

    let rc2 = unsafe {
        RegSetValueExW(
            hkey,
            windows::core::PCWSTR(value_ptr),
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(data_wide.as_ptr() as *const u8, data_wide.len() * 2)),
        )
    };
    unsafe { let _ = RegCloseKey(hkey); }

    if rc2.is_err() {
        return Err(format!("RegSetValueExW({key}, {value_name}) failed: {rc2:?}"));
    }
    Ok(())
}

fn write_reg_dword(key: &str, value_name: &str, data: u32) -> Result<(), String> {
    let key_wide: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();

    let mut hkey = windows::Win32::System::Registry::HKEY::default();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            windows::core::PCWSTR::from_raw(key_wide.as_ptr()),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if rc.is_err() {
        return Err(format!("RegCreateKeyExW({key}) failed: {rc:?}"));
    }

    let data_bytes = data.to_le_bytes();
    let rc2 = unsafe {
        RegSetValueExW(
            hkey,
            windows::core::PCWSTR::from_raw(value_wide.as_ptr()),
            0,
            windows::Win32::System::Registry::REG_DWORD,
            Some(&data_bytes),
        )
    };
    unsafe { let _ = RegCloseKey(hkey); }

    if rc2.is_err() {
        return Err(format!("RegSetValueExW({key}, {value_name}) failed: {rc2:?}"));
    }
    Ok(())
}

fn delete_reg_key(key: &str) -> Result<(), String> {
    let key_wide: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let rc = unsafe {
        windows::Win32::System::Registry::RegDeleteTreeW(
            HKEY_CLASSES_ROOT,
            windows::core::PCWSTR::from_raw(key_wide.as_ptr()),
        )
    };
    if rc.is_err() {
        return Err(format!("RegDeleteTreeW({key}) failed: {rc:?}"));
    }
    Ok(())
}

// ── Utility ─────────────────────────────────────────────────────

/// Find the full path of the current DLL.
fn get_dll_path() -> Result<String, String> {
    let mut buf = vec![0u16; 512];
    let module = unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
    }.map_err(|e| format!("GetModuleHandleW failed: {e}"))?;

    let len = unsafe {
        windows::Win32::System::LibraryLoader::GetModuleFileNameW(
            module,
            &mut buf,
        )
    };
    if len == 0 {
        return Err("GetModuleFileNameW returned 0".into());
    }

    Ok(String::from_utf16_lossy(&buf[..len as usize]))
}

/// Format a GUID as a registry-style string: {XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}
fn guid_to_string(guid: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0], guid.data4[1],
        guid.data4[2], guid.data4[3], guid.data4[4], guid.data4[5],
        guid.data4[6], guid.data4[7],
    )
}
