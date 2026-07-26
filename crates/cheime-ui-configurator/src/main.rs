#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, PostMessageW};
use windows::core::PCWSTR;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};
use wry::{WebView, WebViewBuilder};

#[derive(Default)]
struct App {
    window: Option<Window>,
    webview: Option<WebView>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("CheIME UI Studio")
                    .with_inner_size(LogicalSize::new(1280, 800))
                    .with_min_inner_size(LogicalSize::new(960, 640)),
            )
            .expect("create CheIME UI window");

        let config_path = config_path();
        let config =
            std::fs::read_to_string(&config_path).unwrap_or_else(|_| serde_yaml_fallback());
        let encoded = serde_json::to_string(&config).expect("encode initial config");
        let init_script =
            format!("window.__CHEIME_DESKTOP__=true;window.__CHEIME_CONFIG__={encoded};");
        let save_path = config_path.clone();

        let webview = WebViewBuilder::new()
            .with_initialization_script(init_script)
            .with_ipc_handler(move |request| {
                let Ok(message) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                if message.get("type").and_then(|value| value.as_str()) != Some("save") {
                    return;
                }
                let Some(yaml) = message.get("yaml").and_then(|value| value.as_str()) else {
                    return;
                };
                if let Err(error) = save_validated(&save_path, yaml) {
                    eprintln!("CheIME UI save failed: {error}");
                } else {
                    notify_candidate_windows();
                }
            })
            .with_html(include_str!("studio.html"))
            .build(&window)
            .expect("create WebView2");

        self.webview = Some(webview);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }
}

fn config_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CheIME")
        .join("config")
        .join("ui.yaml")
}

fn serde_yaml_fallback() -> String {
    serde_yaml::to_string(&cheime_tip_core::ui_config::UiConfig::default())
        .expect("serialize default UI config")
}

fn save_validated(path: &Path, yaml: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("configuration path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = parent.join("ui.yaml.tmp");
    std::fs::write(&temp, yaml.as_bytes()).map_err(|error| error.to_string())?;
    cheime_tip_core::ui_config::load_ui_config(&temp)?;

    let from = wide(&temp);
    let to = wide(path);
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
        )
    }
    .map_err(|error| error.to_string())
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn notify_candidate_windows() {
    const WM_CHEIME_RELOAD_CONFIG: u32 = 0x8000 + 0x124;
    let class: Vec<u16> = "CheIME_CandidateWindow\0".encode_utf16().collect();
    let mut after = windows::Win32::Foundation::HWND::default();
    loop {
        let Ok(hwnd) = (unsafe {
            FindWindowExW(
                windows::Win32::Foundation::HWND::default(),
                after,
                windows::core::PCWSTR(class.as_ptr()),
                windows::core::PCWSTR::null(),
            )
        }) else {
            break;
        };
        if hwnd.is_invalid() {
            break;
        }
        unsafe {
            let _ = PostMessageW(hwnd, WM_CHEIME_RELOAD_CONFIG, None, None);
        }
        after = hwnd;
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App::default();
    event_loop.run_app(&mut app).expect("run event loop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_yaml_before_replacing_destination() {
        let root = std::env::temp_dir().join(format!("cheime-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ui.yaml");
        std::fs::write(&path, "original").unwrap();
        assert!(save_validated(&path, "style:\n  bogus: true\n").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        let _ = std::fs::remove_dir_all(root);
    }
}
