//! Windows-side candidate-window style configuration.

use cheime_tip_core::ui_config::{UiConfig, load_ui_config};
use std::ffi::c_void;
use std::path::PathBuf;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::PCWSTR;

const SANDBOX_LIVE_CONFIG: &str = r"C:\CheIMELiveConfig\ui.yaml";

pub fn config_path() -> PathBuf {
    let sandbox_path = PathBuf::from(SANDBOX_LIVE_CONFIG);
    if sandbox_path.is_file() {
        return sandbox_path;
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CheIME")
        .join("config")
        .join("ui.yaml")
}

pub fn load_config() -> UiConfig {
    let path = config_path();
    match load_ui_config(&path) {
        Ok(config) => config,
        Err(error) => {
            crate::tsf_interfaces::tsf_log(&format!(
                "[CheIME] failed to load UI config from {}: {error}",
                path.display()
            ));
            UiConfig::default()
        }
    }
}

pub fn system_uses_dark_theme() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast::<c_void>()),
            Some(&mut size),
        )
    }
    .is_ok()
        && value == 0
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
