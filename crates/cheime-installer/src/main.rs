//! CheIME Installer.
//!
//! CLI tool for registering and unregistering the TIP DLLs:
//! - `cheime-installer.exe install`   — register x64/x86 TIPs + TSF profile
//! - `cheime-installer.exe uninstall` — unregister TIPs + TSF profile
//! - `cheime-installer.exe status`    — check registration state
//!
//! The TIP DLLs implement standard DllRegisterServer/DllUnregisterServer
//! exports. This tool loads the DLLs and calls those functions, then
//! registers the TSF input processor profile via ITfInputProcessorProfileMgr.

fn main() {
    println!("CheIME Installer v0.1.0");
}
