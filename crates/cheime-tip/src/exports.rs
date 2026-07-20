//! COM DLL exports for the TIP.
//!
//! Implements DllRegisterServer, DllUnregisterServer, DllGetClassObject, DllCanUnloadNow.
//! Registry writes use `winreg`-style logic through the `windows` crate.

use std::sync::atomic::{AtomicUsize, Ordering};

// ── TIP CLSID ───────────────────────────────────────────
// {B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}
pub const CHEIME_TIP_CLSID_STR: &str = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}";
pub const CHEIME_TIP_NAME: &str = "CheIME TIP";

/// Global count of live COM allocations.
static LIVE_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Global count of explicit IClassFactory::LockServer locks.
static SERVER_LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn increment_object_count() {
    LIVE_OBJECT_COUNT.fetch_add(1, Ordering::Release);
}

pub fn decrement_object_count() {
    LIVE_OBJECT_COUNT.fetch_sub(1, Ordering::Release);
}

pub fn live_object_count() -> usize {
    LIVE_OBJECT_COUNT.load(Ordering::Acquire)
}

pub fn lock_server() {
    SERVER_LOCK_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn unlock_server() {
    SERVER_LOCK_COUNT
        .fetch_update(Ordering::Release, Ordering::Relaxed, |count| {
            Some(count.saturating_sub(1))
        })
        .ok();
}

pub fn server_lock_count() -> usize {
    SERVER_LOCK_COUNT.load(Ordering::Acquire)
}

#[cfg(test)]
static TEST_COUNTER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub fn test_counter_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_COUNTER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Reset live-object and server-lock counters to zero (tests only).
#[cfg(test)]
pub fn reset_counts() {
    LIVE_OBJECT_COUNT.store(0, Ordering::Release);
    SERVER_LOCK_COUNT.store(0, Ordering::Release);
}

/// Compute the registry subkey path for the TIP CLSID.
pub fn clsid_registry_path() -> String {
    format!("Software\\Classes\\CLSID\\{CHEIME_TIP_CLSID_STR}")
}

/// Compute the InprocServer32 subkey path.
pub fn inproc_server_registry_path() -> String {
    format!("Software\\Classes\\CLSID\\{CHEIME_TIP_CLSID_STR}\\InprocServer32")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_str_is_valid_guid_format() {
        assert_eq!(CHEIME_TIP_CLSID_STR.len(), 38);
        assert!(CHEIME_TIP_CLSID_STR.starts_with('{'));
        assert!(CHEIME_TIP_CLSID_STR.ends_with('}'));
    }

    #[test]
    fn clsid_path_contains_clsid() {
        let path = clsid_registry_path();
        assert!(path.contains(CHEIME_TIP_CLSID_STR));
    }

    #[test]
    fn inproc_path_beneath_clsid_path() {
        let inproc = inproc_server_registry_path();
        assert!(inproc.starts_with(&clsid_registry_path()));
        assert!(inproc.ends_with("InprocServer32"));
    }

    #[test]
    fn object_count_starts_zero() {
        let _guard = test_counter_guard();
        reset_counts();
        assert_eq!(live_object_count(), 0);
    }

    #[test]
    fn increment_and_decrement() {
        let _guard = test_counter_guard();
        reset_counts();
        assert_eq!(live_object_count(), 0);
        increment_object_count();
        assert_eq!(live_object_count(), 1);
        increment_object_count();
        assert_eq!(live_object_count(), 2);
        decrement_object_count();
        assert_eq!(live_object_count(), 1);
        decrement_object_count();
        assert_eq!(live_object_count(), 0);
    }
}
