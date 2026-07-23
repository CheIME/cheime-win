//! Windows-side candidate-window style configuration.

use cheime_tip_core::ui_config::{UiConfig, load_ui_config};
use std::path::PathBuf;

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
    load_ui_config(&config_path()).unwrap_or_default()
}
