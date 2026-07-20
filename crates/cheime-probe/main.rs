//! Isolated COM probe: loads `cheime_tip.dll`, exercises every exported and vtable entry
//! point, then shuts down cleanly. This binary never writes the registry and does NOT
//! activate TSF — if it crashes only the probe process dies.

use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use windows::Win32::Foundation::{BOOL, FreeLibrary, LPARAM, WPARAM};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Vtbl};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::TextServices::{
    ITfCompositionSink, ITfCompositionSink_Vtbl, ITfDisplayAttributeProvider,
    ITfDisplayAttributeProvider_Vtbl, ITfKeyEventSink, ITfKeyEventSink_Vtbl, ITfTextInputProcessor,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Vtbl, ITfThreadMgrEventSink,
    ITfThreadMgrEventSink_Vtbl,
};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCSTR, PCWSTR, s};

const CLSID_CHEIME_TIP: GUID = GUID::from_u128(0xB5F1C9A8_3E7D_4A15_AE2D_F89C1B6E3A07);

fn check(hr: HRESULT, label: &str) {
    if hr.is_err() {
        eprintln!("[{label}] FAIL: {hr:?}");
        std::process::abort();
    }
    eprintln!("[{label}] OK ({hr:?})");
}

fn check_expected(hr: HRESULT, expected: HRESULT, label: &str) {
    if hr != expected {
        eprintln!("[{label}] FAIL: expected {expected:?}, got {hr:?}");
        std::process::abort();
    }
    eprintln!("[{label}] OK ({hr:?})");
}

#[allow(dead_code)]
unsafe fn invoke_unknown_methods(ptr: *mut c_void, label: &str) {
    // Read vtable pointer
    let vtbl_ptr = unsafe { *(ptr as *const *const IUnknown_Vtbl) };
    let vtbl = unsafe { &*vtbl_ptr };

    eprintln!("[{label}] vtable at {vtbl_ptr:p}");

    // AddRef
    let ref_after_add = unsafe { (vtbl.AddRef)(ptr) };
    eprintln!("[{label}] AddRef -> {ref_after_add}");

    // QueryInterface IUnknown
    let mut iu: *mut c_void = null_mut();
    let qi_hr = unsafe { (vtbl.QueryInterface)(ptr, &IUnknown::IID, &mut iu) };
    check(qi_hr, &format!("{label} QI(IUnknown)"));
    assert!(!iu.is_null(), "{label} QI(IUnknown) returned null");
    let ref_before_rel = unsafe { (vtbl.Release)(ptr) };
    eprintln!("[{label}] Release after QI -> {ref_before_rel}");
    unsafe { drop(IUnknown::from_raw(iu)) };

    // Release original reference
    let ref_after_release = unsafe { (vtbl.Release)(ptr) };
    eprintln!("[{label}] final Release -> {ref_after_release}");
}

fn main() {
    // Locate the DLL next to the probe binary, or in the release output dir.
    let exe = std::env::current_exe().expect("exe path");
    let exe_dir = exe.parent().expect("exe dir");

    let dll_rel = exe_dir.join("cheime_tip.dll");
    let dll_path = if dll_rel.exists() {
        dll_rel
    } else {
        // Fallback: search target/release relative to workspace root
        let mut probe_crate = exe_dir.to_path_buf();
        while probe_crate
            .file_name()
            .map(|n| n != "target")
            .unwrap_or(false)
        {
            probe_crate.pop();
        }
        let release = probe_crate
            .join("target")
            .join("release")
            .join("cheime_tip.dll");
        if release.exists() {
            release
        } else {
            eprintln!("Cannot find cheime_tip.dll. Build the DLL first:");
            eprintln!("  cargo build -p cheime-tip --release");
            eprintln!("then run the probe:");
            eprintln!("  cargo run -p cheime-probe --release");
            std::process::exit(1);
        }
    };

    let dll_path = dll_path.canonicalize().expect("canonical DLL path");
    let mut dll_file = File::open(&dll_path).expect("open DLL for fingerprint");
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = dll_file
            .read(&mut buffer)
            .expect("read DLL for fingerprint");
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    eprintln!("Probing exact DLL: {}", dll_path.display());
    eprintln!("DLL SHA-256: {:x}", hasher.finalize());

    // ── 1. Load DLL ────────────────────────────────────────────
    let dll_wide: Vec<u16> = dll_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dll =
        unsafe { LoadLibraryW(PCWSTR::from_raw(dll_wide.as_ptr())) }.expect("LoadLibraryW failed");
    eprintln!("[load] OK — DLL loaded at {dll:?}");

    // ── 2. Look up exports ──────────────────────────────────────
    type DllGetClassObjectFn =
        unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
    type DllCanUnloadNowFn = unsafe extern "system" fn() -> HRESULT;

    let get_class_object: DllGetClassObjectFn = unsafe {
        let farproc = GetProcAddress(dll, PCSTR(s!("DllGetClassObject").as_ptr().cast()))
            .expect("GetProcAddress failed");
        // FARPROC is unsafe extern "system" fn() -> isize — transmute to target signature
        std::mem::transmute(farproc)
    };
    eprintln!(
        "[export] DllGetClassObject at {:p}",
        get_class_object as *const ()
    );

    let can_unload: DllCanUnloadNowFn = unsafe {
        let farproc = GetProcAddress(dll, PCSTR(s!("DllCanUnloadNow").as_ptr().cast()))
            .expect("GetProcAddress failed");
        std::mem::transmute(farproc)
    };
    eprintln!("[export] DllCanUnloadNow at {:p}", can_unload as *const ());

    // DllCanUnloadNow — should be S_OK (no live objects yet)
    let unload_before = unsafe { (can_unload)() };
    eprintln!("[unload-before] {unload_before:?}");
    assert_eq!(
        unload_before,
        HRESULT(0),
        "DllCanUnloadNow should be S_OK before any objects"
    );

    // ── 3. DllGetClassObject — get IClassFactory ──────────────────
    let mut factory_raw: *mut c_void = null_mut();
    let hr =
        unsafe { (get_class_object)(&CLSID_CHEIME_TIP, &IClassFactory::IID, &mut factory_raw) };
    check(hr, "DllGetClassObject(CLSID, IClassFactory)");
    assert!(!factory_raw.is_null());

    let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_raw) };
    eprintln!("[factory] obtained");
    let factory_unknown: IUnknown = factory.cast().expect("factory QI(IUnknown)");
    eprintln!("[factory] QI(IUnknown) OK");
    drop(factory_unknown);
    let factory_vtbl = unsafe { &**(factory.as_raw() as *const *const IClassFactory_Vtbl) };
    let mut aggregate_out: *mut c_void = std::ptr::dangling_mut();
    let aggregate_hr = unsafe {
        (factory_vtbl.CreateInstance)(
            factory.as_raw(),
            factory.as_raw(),
            &ITfTextInputProcessorEx::IID,
            &mut aggregate_out,
        )
    };
    assert_eq!(
        aggregate_hr,
        HRESULT(0x8004_0110u32 as i32),
        "aggregation must be rejected"
    );
    assert!(
        aggregate_out.is_null(),
        "aggregation rejection must clear output"
    );
    eprintln!("[factory] aggregation rejection OK");
    check(
        unsafe { (factory_vtbl.LockServer)(factory.as_raw(), BOOL(1)) },
        "factory LockServer(TRUE)",
    );
    check(
        unsafe { (factory_vtbl.LockServer)(factory.as_raw(), BOOL(0)) },
        "factory LockServer(FALSE)",
    );

    // DllCanUnloadNow — should be S_FALSE while factory is alive
    let unload_with_factory = unsafe { (can_unload)() };
    eprintln!("[unload-with-factory] {unload_with_factory:?}");
    assert_ne!(
        unload_with_factory,
        HRESULT(0),
        "DllCanUnloadNow should be S_FALSE with live factory"
    );

    // ── 4. CreateInstance — ITfTextInputProcessorEx ──────────────
    let mut tip_raw: *mut c_void = null_mut();
    let hr = unsafe {
        let vtbl = *(factory.as_raw() as *const *const IClassFactory_Vtbl);
        ((*vtbl).CreateInstance)(
            factory.as_raw(),
            null_mut(), // no aggregation
            &ITfTextInputProcessorEx::IID,
            &mut tip_raw,
        )
    };
    check(hr, "CreateInstance(ITfTextInputProcessorEx)");
    assert!(!tip_raw.is_null());
    let tip_ex: ITfTextInputProcessorEx = unsafe { ITfTextInputProcessorEx::from_raw(tip_raw) };
    eprintln!("[tip] ITfTextInputProcessorEx created");

    // ── 5. Cast to ITfTextInputProcessor (base interface) ────────
    let tip: ITfTextInputProcessor = tip_ex.cast().expect("cast TIPEx -> TIP");
    eprintln!("[tip] cast to ITfTextInputProcessor OK");

    // ── 6. Activate with null thread mgr ──────────────────────────
    let activate_hr = unsafe {
        let vtbl = *(tip.as_raw() as *const *const ITfTextInputProcessorEx_Vtbl);
        ((*vtbl).base__.Activate)(tip.as_raw(), null_mut(), 0)
    };
    check_expected(
        activate_hr,
        HRESULT(0x8000_4003u32 as i32),
        "Activate(null-thread-mgr)",
    );

    // ── 7. ActivateEx ─────────────────────────────────────────────
    let activate_ex_hr = unsafe {
        let vtbl = *(tip_ex.as_raw() as *const *const ITfTextInputProcessorEx_Vtbl);
        ((*vtbl).ActivateEx)(tip_ex.as_raw(), null_mut(), 0, 0)
    };
    check_expected(
        activate_ex_hr,
        HRESULT(0x8000_4003u32 as i32),
        "ActivateEx(null-thread-mgr)",
    );

    // ── 8. Deactivate ─────────────────────────────────────────────
    let deactivate_hr = unsafe {
        let vtbl = *(tip.as_raw() as *const *const ITfTextInputProcessorEx_Vtbl);
        ((*vtbl).base__.Deactivate)(tip.as_raw())
    };
    check(deactivate_hr, "Deactivate");

    // ── 9. QI for ITfKeyEventSink ─────────────────────────────────
    let key: ITfKeyEventSink = tip.cast().expect("QI TIP -> KeyEventSink");
    eprintln!("[key] ITfKeyEventSink obtained via windows Interface::cast");

    // Drop the key sink reference — verify it doesn't crash
    drop(key);
    eprintln!("[key] released");

    // ── 10. QI for ITfThreadMgrEventSink ──────────────────────────
    let tm: ITfThreadMgrEventSink = tip.cast().expect("QI TIP -> ThreadMgrEventSink");
    eprintln!("[tm] ITfThreadMgrEventSink obtained");
    drop(tm);
    eprintln!("[tm] released");

    // ── 11. QI for ITfCompositionSink ─────────────────────────────
    let comp: ITfCompositionSink = tip.cast().expect("QI TIP -> CompositionSink");
    eprintln!("[comp] ITfCompositionSink obtained");
    drop(comp);
    eprintln!("[comp] released");

    // ── 12. QI for ITfDisplayAttributeProvider ────────────────────
    let disp: ITfDisplayAttributeProvider = tip.cast().expect("QI TIP -> DisplayAttributeProvider");
    eprintln!("[disp] ITfDisplayAttributeProvider obtained");

    // Call EnumDisplayAttributeInfo
    {
        let vtbl = unsafe { &**(disp.as_raw() as *const *const ITfDisplayAttributeProvider_Vtbl) };
        let mut out_enum: *mut c_void = null_mut();
        let hr = unsafe { (vtbl.EnumDisplayAttributeInfo)(disp.as_raw(), &mut out_enum) };
        eprintln!("[disp] EnumDisplayAttributeInfo -> {hr:?}");
        assert!(out_enum.is_null());
    }
    // Call GetDisplayAttributeInfo
    {
        let vtbl = unsafe { &**(disp.as_raw() as *const *const ITfDisplayAttributeProvider_Vtbl) };
        let mut out_info: *mut c_void = null_mut();
        let hr = unsafe {
            (vtbl.GetDisplayAttributeInfo)(
                disp.as_raw(),
                &ITfDisplayAttributeProvider::IID,
                &mut out_info,
            )
        };
        eprintln!("[disp] GetDisplayAttributeInfo -> {hr:?}");
        assert!(out_info.is_null());
    }
    drop(disp);
    eprintln!("[disp] released");

    // ── 13. Call key sink callbacks via raw vtable ─────────────────
    {
        let key2: ITfKeyEventSink = tip.cast().expect("QI TIP -> KeyEventSink again");
        let vtbl = unsafe { &**(key2.as_raw() as *const *const ITfKeyEventSink_Vtbl) };

        // OnSetFocus
        let hr = unsafe { (vtbl.OnSetFocus)(key2.as_raw(), BOOL(1)) };
        check(hr, "key OnSetFocus(TRUE)");

        // OnTestKeyDown — 'a' press
        let mut eaten = BOOL(0);
        let hr = unsafe {
            (vtbl.OnTestKeyDown)(
                key2.as_raw(),
                null_mut(),
                WPARAM(0x41),
                LPARAM(0),
                &mut eaten,
            )
        };
        check(hr, "key OnTestKeyDown('a')");
        eprintln!("[key] OnTestKeyDown eaten={}", eaten.as_bool());

        // OnTestKeyUp — 'a' release
        let mut test_up_eaten = BOOL(1);
        let hr = unsafe {
            (vtbl.OnTestKeyUp)(
                key2.as_raw(),
                null_mut(),
                WPARAM(0x41),
                LPARAM(0),
                &mut test_up_eaten,
            )
        };
        check(hr, "key OnTestKeyUp('a')");
        assert!(!test_up_eaten.as_bool());

        // OnKeyDown — 'a' press
        let mut eaten1 = BOOL(0);
        let hr = unsafe {
            (vtbl.OnKeyDown)(
                key2.as_raw(),
                null_mut(),
                WPARAM(0x41),
                LPARAM(0),
                &mut eaten1,
            )
        };
        check(hr, "key OnKeyDown('a')");
        eprintln!("[key] OnKeyDown eaten={}", eaten1.as_bool());

        // OnKeyUp
        let mut eaten2 = BOOL(0);
        let hr = unsafe {
            (vtbl.OnKeyUp)(
                key2.as_raw(),
                null_mut(),
                WPARAM(0x41),
                LPARAM(0),
                &mut eaten2,
            )
        };
        check(hr, "key OnKeyUp('a')");
        eprintln!("[key] OnKeyUp eaten={}", eaten2.as_bool());

        // OnPreservedKey
        let mut eaten_pk = BOOL(0);
        let hr = unsafe {
            (vtbl.OnPreservedKey)(
                key2.as_raw(),
                null_mut(),
                &ITfKeyEventSink::IID,
                &mut eaten_pk,
            )
        };
        check(hr, "key OnPreservedKey");
        eprintln!("[key] OnPreservedKey eaten={}", eaten_pk.as_bool());

        drop(key2);
    }

    // Exercise all non-registration thread-manager callbacks.
    {
        let tm2: ITfThreadMgrEventSink =
            tip.cast().expect("QI TIP -> ThreadMgrEventSink callbacks");
        let vtbl = unsafe { &**(tm2.as_raw() as *const *const ITfThreadMgrEventSink_Vtbl) };
        check(
            unsafe { (vtbl.OnInitDocumentMgr)(tm2.as_raw(), null_mut()) },
            "tm OnInitDocumentMgr",
        );
        check(
            unsafe { (vtbl.OnUninitDocumentMgr)(tm2.as_raw(), null_mut()) },
            "tm OnUninitDocumentMgr",
        );
        check(
            unsafe { (vtbl.OnSetFocus)(tm2.as_raw(), null_mut(), null_mut()) },
            "tm OnSetFocus",
        );
        check(
            unsafe { (vtbl.OnPushContext)(tm2.as_raw(), null_mut()) },
            "tm OnPushContext",
        );
        check(
            unsafe { (vtbl.OnPopContext)(tm2.as_raw(), null_mut()) },
            "tm OnPopContext",
        );
    }
    // Exercise the non-registration composition callback.
    {
        let comp2: ITfCompositionSink = tip.cast().expect("QI TIP -> CompositionSink callback");
        let vtbl = unsafe { &**(comp2.as_raw() as *const *const ITfCompositionSink_Vtbl) };
        check(
            unsafe { (vtbl.OnCompositionTerminated)(comp2.as_raw(), 0, null_mut()) },
            "composition OnCompositionTerminated",
        );
    }

    // ── 14. Drop TIP — should be last reference ──────────────────
    drop(tip_ex);
    drop(tip);
    eprintln!("[tip] all TIP references released");

    // ── 15. Drop factory ─────────────────────────────────────────
    drop(factory);
    eprintln!("[factory] released");

    // ── 16. DllCanUnloadNow — should be S_OK again ───────────────
    let unload_after = unsafe { (can_unload)() };
    eprintln!("[unload-after] {unload_after:?}");
    assert_eq!(
        unload_after,
        HRESULT(0),
        "DllCanUnloadNow should be S_OK after releasing everything"
    );

    // ── 17. FreeLibrary ──────────────────────────────────────────
    unsafe {
        let _ = FreeLibrary(dll);
    };
    eprintln!("[free] DLL unloaded");

    // ── 18. Summary ──────────────────────────────────────────────
    eprintln!();
    eprintln!("╔══════════════════════════════════════╗");
    eprintln!("║  PROBE PASSED — all non-registration callbacks OK      ║");
    eprintln!("╚══════════════════════════════════════╝");
}
