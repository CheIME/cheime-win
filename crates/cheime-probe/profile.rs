//! Process-scoped TSF profile activation probe.
//!
//! Activates the registered CheIME profile only for this disposable process,
//! pumps a message loop briefly, then deactivates it. It does not affect Explorer
//! or other GUI processes.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    ITfInputProcessorProfileMgr, TF_IPPMF_FORPROCESS, TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY,
    WM_TIMER, WNDCLASSW,
};
use windows::core::{GUID, w};

const CLSID_CHEIME_TIP: GUID = GUID::from_u128(0xB5F1C9A8_3E7D_4A15_AE2D_F89C1B6E3A07);
const GUID_PROFILE: GUID = GUID::from_u128(0xD7E2A3B4_C5F6_7890_ABCD_EF1234567890);
const CLSID_TF_INPUTPROCESSORPROFILES: GUID =
    GUID::from_u128(0x33C53A50_F456_4884_B049_85FD643ECFED);
const LANGID_ZH_CN: u16 = 0x0804;

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            unsafe { DestroyWindow(hwnd).ok() };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> windows::core::Result<()> {
    if std::env::var("CHEIME_DISPOSABLE_GUEST").unwrap_or_default() != "1" {
        eprintln!("REFUSING: cheime-profile-probe requires CHEIME_DISPOSABLE_GUEST=1.");
        eprintln!("This binary activates a registered TSF profile.");
        eprintln!("Run only in Windows Sandbox or a revertible VM.");
        std::process::exit(2);
    }

    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let _apartment = ComApartment;

    eprintln!("[profile-probe] Registering disposable message window");
    let class = w!("CheIMEProfileProbeWindow");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        lpszClassName: class,
        ..Default::default()
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("CheIME Profile Probe"),
            WINDOW_STYLE(0),
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )?
    };

    let manager: ITfInputProcessorProfileMgr =
        unsafe { CoCreateInstance(&CLSID_TF_INPUTPROCESSORPROFILES, None, CLSCTX_INPROC_SERVER)? };

    eprintln!("[profile-probe] Activating CheIME profile FORPROCESS");
    unsafe {
        manager.ActivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            LANGID_ZH_CN,
            &CLSID_CHEIME_TIP,
            &GUID_PROFILE,
            HKL(std::ptr::null_mut()),
            TF_IPPMF_FORPROCESS,
        )?
    };

    unsafe { SetTimer(hwnd, 1, 1500, None) };
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    eprintln!("[profile-probe] Deactivating CheIME profile FORPROCESS");
    unsafe {
        manager.DeactivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            LANGID_ZH_CN,
            &CLSID_CHEIME_TIP,
            &GUID_PROFILE,
            HKL(std::ptr::null_mut()),
            TF_IPPMF_FORPROCESS,
        )?
    };

    eprintln!("PROCESS-SCOPED PROFILE PROBE PASSED");
    Ok(())
}
