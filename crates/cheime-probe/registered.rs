//! Registered COM and TSF-owned activation probe for the CheIME TIP.
//!
//! This runs in a disposable process. Activating the real TSF thread manager lets
//! TSF activate registered profiles with its own client IDs. The probe verifies
//! separate registered COM creation without manually invoking the TIP lifecycle.

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::TextServices::{CLSID_TF_ThreadMgr, ITfTextInputProcessorEx, ITfThreadMgr};
use windows::core::{GUID, Interface};

const CLSID_CHEIME_TIP: GUID = GUID::from_u128(0xB5F1C9A8_3E7D_4A15_AE2D_F89C1B6E3A07);

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn main() -> windows::core::Result<()> {
    if std::env::var("CHEIME_DISPOSABLE_GUEST").unwrap_or_default() != "1" {
        eprintln!("REFUSING: cheime-registered-probe requires CHEIME_DISPOSABLE_GUEST=1.");
        eprintln!("This binary performs real COM/TSF registration and activation.");
        eprintln!("Run only in Windows Sandbox or a revertible VM.");
        std::process::exit(2);
    }

    eprintln!("[registered-probe] CoInitializeEx(STA)");
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let _apartment = ComApartment;

    eprintln!("[registered-probe] CoCreateInstance(CLSID_TF_ThreadMgr)");
    let thread_mgr: ITfThreadMgr =
        unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)? };

    eprintln!("[registered-probe] ITfThreadMgr::Activate");
    let client_id = unsafe { thread_mgr.Activate()? };
    eprintln!("[registered-probe] client_id={client_id}");

    eprintln!("[registered-probe] CoCreateInstance(CLSID_CHEIME_TIP)");
    let tip: ITfTextInputProcessorEx =
        unsafe { CoCreateInstance(&CLSID_CHEIME_TIP, None, CLSCTX_INPROC_SERVER)? };

    let unknown = tip.cast::<windows::core::IUnknown>()?;
    eprintln!("[registered-probe] QI(IUnknown)={:p}", unknown.as_raw());
    drop(unknown);

    eprintln!("[registered-probe] Releasing registered CheIME COM object");
    drop(tip);

    // ITfThreadMgr::Activate already lets TSF activate registered TIP profiles
    // with TSF-assigned client IDs. Calling tip.ActivateEx with this process's
    // thread-manager client ID is invalid and AdviseKeyEventSink rejects it.
    eprintln!("[registered-probe] ITfThreadMgr::Deactivate");
    unsafe { thread_mgr.Deactivate()? };
    drop(thread_mgr);

    eprintln!("REGISTERED COM PROBE PASSED");
    Ok(())
}
