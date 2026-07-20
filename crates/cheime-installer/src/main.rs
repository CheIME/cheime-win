//! CheIME Installer.
//!
//! CLI tool for installing and uninstalling the TIP DLLs and TSF profile.
//! Usage:
//!   cheime-installer.exe install   — register DLLs + TSF profile
//!   cheime-installer.exe uninstall — unregister TSF profile + DLLs
//!   cheime-installer.exe status    — show registration state

use std::env;

const CHEIME_TIP_CLSID: &str = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}";
const CHEIME_DIR: &str = "CheIME";

fn main() {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("status");

    match command {
        "install" => fail_with_script("install.ps1"),
        "uninstall" => fail_with_script("uninstall.ps1"),
        "status" => cmd_status(),
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: cheime-installer.exe [install|uninstall|status]");
            std::process::exit(1);
        }
    }
}

fn fail_with_script(script: &str) -> ! {
    eprintln!("This command does not perform deployment.");
    eprintln!(r"Run the administrator PowerShell script instead: scripts\{script}");
    std::process::exit(2);
}

fn cmd_status() {
    println!("CheIME Installer v0.1.0 — status");
    println!();

    let local_app_data = get_local_app_data();
    let cheime_dir = format!("{local_app_data}\\{CHEIME_DIR}");

    println!("Installation directory: {cheime_dir}");
    println!("CLSID: {CHEIME_TIP_CLSID}");

    // Check registry
    let clsid_key = format!("Software\\Classes\\CLSID\\{CHEIME_TIP_CLSID}");
    println!();
    println!("Registry check:");
    println!("  HKCU\\{clsid_key} — not implemented (would check via RegOpenKeyExW)");

    // Check file presence
    let bin_dir = format!("{cheime_dir}\\bin");
    println!();
    println!("File check:");
    println!("  {bin_dir}\\cheime-tip-x64.dll — not checked");
    println!("  {bin_dir}\\cheime-tip-x86.dll — not checked");
    println!("  {bin_dir}\\cheime-engine.exe — not checked");

    println!();
    println!("Status: development build — full registration not yet possible.");
}

fn get_local_app_data() -> String {
    env::var("LOCALAPPDATA").unwrap_or_else(|_| String::from("C:\\Users\\Public"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_is_valid_guid_format() {
        assert_eq!(CHEIME_TIP_CLSID.len(), 38);
        assert!(CHEIME_TIP_CLSID.starts_with('{'));
        assert!(CHEIME_TIP_CLSID.ends_with('}'));
    }

    #[test]
    fn get_local_app_data_returns_string() {
        let path = get_local_app_data();
        assert!(!path.is_empty());
    }
}
