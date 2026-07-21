//! GDI candidate window — config-driven rendering.
//!
//! All visual parameters come from `UiConfig`. No hardcoded sizes or colors.
//! The config is loaded by the TIP at startup and stored in `WindowContext`.

use crate::edit_session::request_edit_session;
use crate::io_thread::{WM_CHEIME_ACTION, WM_CHEIME_SNAPSHOT, WM_CHEIME_STATUS};
use crate::tsf_interfaces::{ComTip, tsf_log};
use cheime_model::{CandidateSnapshot, PlatformAction, PlatformActionKind};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::layout::{hit_test_candidate, layout_snapshot};
use windows::Win32::Graphics::Gdi::RedrawWindow;
use windows::Win32::Graphics::Gdi::{RDW_INVALIDATE, RDW_ERASE};
use cheime_tip_core::ui_config::UiConfig;
use std::sync::Mutex;
use std::sync::mpsc::SyncSender;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject,
    EndPaint, FF_DONTCARE, FW_NORMAL, GetSysColor, OPAQUE, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, TextOutW, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
    COLOR_WINDOW, COLOR_WINDOWTEXT, HBRUSH, HDC, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::TextServices::{ITfComposition, ITfThreadMgr};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, GWLP_USERDATA,
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WINDOW_LONG_PTR_INDEX, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT,
    WNDCLASSW, WNDCLASS_STYLES, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, HMENU,
};

const CANDIDATE_WINDOW_CLASS: &str = "CheIME_CandidateWindow";

pub type SnapshotBox = Mutex<Option<(CandidateSnapshot, Vec<RowRender>)>>;

pub struct RowRender {
    pub text: Vec<u16>,
    pub y: i32,
    pub highlighted: bool,
}

/// Context stored as GWLP_USERDATA on the candidate window.
/// Carries both engine communication state and UI configuration.
pub struct WindowContext {
    pub snapshot: SnapshotBox,
    pub thread_mgr: ITfThreadMgr,
    pub client_id: u32,
    pub channel: SyncSender<FrontendMessage>,
    pub composition: Mutex<Option<ITfComposition>>,
    pub tip: *mut ComTip,
    /// UI configuration (never modified after window creation; safe shared ref).
    pub config: UiConfig,
}

pub struct CandidateWindow {
    hwnd: HWND,
    pub ctx_ptr: *const WindowContext,
}

impl CandidateWindow {
    /// Create a new candidate window. `ctx` ownership transfers to window user data.
    pub fn create(ctx: Box<WindowContext>) -> Result<Self, String> {
        let hinst = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let class_wide: Vec<u16> = CANDIDATE_WINDOW_CLASS
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(candidate_window_proc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: windows::core::PCWSTR::from_raw(class_wide.as_ptr()),
            style: WNDCLASS_STYLES(0),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: HBRUSH(
                unsafe { CreateSolidBrush(COLORREF(GetSysColor(COLOR_WINDOW))) }.0,
            ),
            lpszMenuName: windows::core::PCWSTR::null(),
        };
        unsafe { RegisterClassW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                windows::core::PCWSTR::from_raw(class_wide.as_ptr()),
                windows::core::w!("CheIME Candidate"),
                WS_POPUP,
                -1000, -1000, 200, 100,
                HWND(std::ptr::null_mut()),
                HMENU(std::ptr::null_mut()),
                HINSTANCE(hinst.0),
                None,
            )
        };
        let hwnd = hwnd.map_err(|e| format!("CreateWindowExW: {e}"))?;
        if hwnd.is_invalid() {
            return Err("CreateWindowExW failed".into());
        }

        let ctx_ptr = Box::into_raw(ctx);
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0),
                ctx_ptr as isize,
            );
        }

        Ok(Self { hwnd, ctx_ptr })
    }


    /// Hide the candidate window (e.g. on focus loss, engine disconnect).
    pub fn hide(&self) {
        unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); }
    }
    pub fn hwnd(&self) -> HWND { self.hwnd }

    pub fn new_context(
        thread_mgr: ITfThreadMgr,
        client_id: u32,
        channel: SyncSender<FrontendMessage>,
        tip: *mut ComTip,
    ) -> Box<WindowContext> {
        Box::new(WindowContext {
            snapshot: Mutex::new(None),
            thread_mgr,
            client_id,
            channel,
            composition: Mutex::new(None),
            tip,
            config: UiConfig::default(),
        })
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() {
            unsafe { let _ = DestroyWindow(self.hwnd); }
        }
        // WM_DESTROY frees ctx_ptr; prevent double-free.
        self.ctx_ptr = std::ptr::null();
    }
}

// ── Window procedure ─────────────────────────────────────────────────

unsafe extern "system" fn candidate_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Helper: read WindowContext from user data.
    let ctx = || {
        let p = unsafe { GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0)) }
            as *const WindowContext;
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    };

    match msg {
        WM_CREATE => LRESULT(0),

        WM_ERASEBKGND => {
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                if let Some(ctx) = ctx() {
                    if let Ok(st) = ctx.snapshot.lock() {
                        if let Some((_, rows)) = st.as_ref() {
                            unsafe { paint(hdc, rows, &ctx.config); }
                        }
                    }
                }
                unsafe { let _ = EndPaint(hwnd, &ps); }
            }
            LRESULT(0)
        }

        WM_CHEIME_SNAPSHOT => {
            handle_snapshot(hwnd, lparam, ctx())
        }

        WM_CHEIME_ACTION => {
            handle_action(lparam, ctx())
        }

        WM_CHEIME_STATUS => {
            if lparam.0 != 0 {
                let status = unsafe { Box::from_raw(lparam.0 as *mut (bool, String)) };
                tsf_log(&format!(
                    "[CheIME] WM_STATUS connected={} detail={}",
                    status.0, status.1
                ));
                if !status.0 {
                    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                }
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            handle_click(lparam, ctx())
        }

        WM_DESTROY => {
            let p = unsafe { GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0)) }
                as *mut WindowContext;
            if !p.is_null() {
                drop(unsafe { Box::from_raw(p) });
            }
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ── Message handlers ──────────────────────────────────────────────────

fn handle_snapshot(hwnd: HWND, lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    let cfg = &ctx.config;

    if lparam.0 != 0 {
        let boxed: Box<CandidateSnapshot> =
            unsafe { Box::from_raw(lparam.0 as *mut CandidateSnapshot) };
        tsf_log(&format!(
            "[CheIME] WM_SNAPSHOT preedit={} candidates={}",
            boxed.preedit, boxed.candidates.len()
        ));

        let char_width = cfg.candidate.char_width.unwrap_or(cfg.candidate.font_size);
        let line_height = cfg.candidate.line_height;
        let rows = build_rows(&boxed, line_height, char_width, &cfg.candidate);
        let total_height = (rows.len() as i32) * line_height
            + cfg.candidate.row_padding_y * 2;
        let max_width = rows.iter().map(|r| r.text.len()).max().unwrap_or(0) as i32
            * char_width
            + cfg.candidate.row_padding_x * 2;

        // Sync has_composition from engine preedit
        if !ctx.tip.is_null() {
            unsafe { (*ctx.tip).has_composition.set(!boxed.preedit.is_empty()); }
        }

        if let Ok(mut st) = ctx.snapshot.lock() {
            *st = Some((*boxed, rows));
        }

        let x = cfg.window.caret_offset_x;
        let y = cfg.window.caret_offset_y;
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x, y,
                max_width.max(cfg.window.min_width),
                total_height.max(line_height * 2),
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_ERASE);
        }
    } else {
        unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    }
    LRESULT(0)
}

fn handle_action(lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    if lparam.0 != 0 {
        let action: Box<PlatformAction> =
            unsafe { Box::from_raw(lparam.0 as *mut PlatformAction) };
        tsf_log(&format!("[CheIME] WM_ACTION action={action:?}"));
        match unsafe { ctx.thread_mgr.GetFocus() } {
            Ok(doc) => match unsafe { doc.GetTop() } {
                Ok(context) => {
                    tsf_log("[CheIME] WM_ACTION: requesting edit session");
                    request_edit_session(
                        ctx.client_id, &context, *action,
                        &ctx.channel as *const SyncSender<FrontendMessage>,
                        &ctx.composition as *const Mutex<Option<ITfComposition>>,
                    );
                }
                Err(e) => tsf_log(&format!("[CheIME] WM_ACTION: GetTop failed: {e:?}")),
            },
            Err(e) => tsf_log(&format!("[CheIME] WM_ACTION: GetFocus failed: {e:?}")),
        }
        return LRESULT(0);
    }
    LRESULT(0)
}

fn handle_click(lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    let cfg = &ctx.config;
    let x = (lparam.0 as u16) as i32;
    let y = ((lparam.0 >> 16) as u16) as i32;
    let char_width = cfg.candidate.char_width.unwrap_or(cfg.candidate.font_size);
    let line_height = cfg.candidate.line_height;

    let hit_index = match ctx.snapshot.lock() {
        Ok(guard) => {
            if let Some((snap, _rows)) = guard.as_ref() {
                hit_test_candidate(&layout_snapshot(snap, line_height, char_width), x, y, line_height)
            } else {
                None
            }
        }
        Err(_) => None,
    };

    if let Some(idx) = hit_index {
        if let Ok(guard) = ctx.snapshot.lock() {
            if let Some((snap, _rows)) = guard.as_ref() {
                let candidate = snap.candidates.get(idx.saturating_sub(1));
                if let Some(cand) = candidate {
                    let action = PlatformAction {
                        id: cheime_model::ActionId::new(0),
                        epoch: snap.epoch,
                        revision: snap.revision,
                        kind: PlatformActionKind::Commit { text: cand.text.clone() },
                    };
                    tsf_log(&format!("[CheIME] Click commit: {}", cand.text));
                    if let Ok(doc) = unsafe { ctx.thread_mgr.GetFocus() } {
                        if let Ok(context) = unsafe { doc.GetTop() } {
                            request_edit_session(
                                ctx.client_id, &context, action,
                                &ctx.channel as *const SyncSender<FrontendMessage>,
                                &ctx.composition as *const Mutex<Option<ITfComposition>>,
                            );
                        }
                    }
                }
            }
        }
    }
    LRESULT(0)
}

// ── Rendering ─────────────────────────────────────────────────────────

unsafe fn paint(hdc: HDC, rows: &[RowRender], config: &UiConfig) {
    let fg = parse_hex(&config.theme.colors.candidate_text)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }));
    let hl_bg = parse_hex(&config.theme.colors.selected_background)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHT) }));
    let hl_fg = parse_hex(&config.theme.colors.selected_text)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) }));

    let font_size = config.candidate.font_size;
    let face: Vec<u16> = "Microsoft YaHei\0".encode_utf16().collect();

    let font = unsafe {
        CreateFontW(
            font_size, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            0,
            DEFAULT_QUALITY.0 as u32,
            FF_DONTCARE.0 as u32 | DEFAULT_CHARSET.0 as u32,
            windows::core::PCWSTR::from_raw(face.as_ptr()),
        )
    };
    let old = unsafe { SelectObject(hdc, font) };

    let pad_x = config.candidate.row_padding_x;
    for row in rows {
        unsafe {
            if row.highlighted {
                SetBkColor(hdc, hl_bg);
                SetTextColor(hdc, hl_fg);
                SetBkMode(hdc, OPAQUE);
            } else {
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, fg);
            }
            let _ = TextOutW(hdc, pad_x, row.y, &row.text);
        }
    }
    if !old.is_invalid() {
        unsafe { SelectObject(hdc, old); }
    }
    unsafe { let _ = DeleteObject(font); }
}

fn build_rows(
    snapshot: &CandidateSnapshot,
    line_height: i32,
    char_width: i32,
    config: &cheime_tip_core::ui_config::CandidateConfig,
) -> Vec<RowRender> {
    let layout = layout_snapshot(snapshot, line_height, char_width);
    let mut rows = Vec::new();
    let mut y = config.row_padding_y;

    if !layout.preedit.is_empty() {
        rows.push(RowRender {
            text: layout.preedit.encode_utf16().collect(),
            y,
            highlighted: false,
        });
        y += line_height;
    }
    for row in &layout.rows {
        if row.is_preedit { continue; }
        let mut s = String::new();
        use std::fmt::Write;
        if let Some(idx) = row.index {
            let _ = write!(s, "{}. {}", idx, row.text);
            if let Some(ref ann) = row.annotation {
                let _ = write!(s, " {}", ann);
            }
        } else {
            s = row.text.clone();
        }
        rows.push(RowRender {
            text: s.encode_utf16().collect(),
            y,
            highlighted: row.is_highlighted,
        });
        y += line_height;
    }
    rows
}

// ── Color helpers ─────────────────────────────────────────────────────

/// Parse a CSS hex color like "#1e1e2e" or "#fff" into a COLORREF (0x00BBGGRR).
fn parse_hex(s: &str) -> Option<COLORREF> {
    let hex = s.strip_prefix('#')?;
    let (r, g, b) = match hex.len() {
        6 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            ((n >> 16) as u8, ((n >> 8) & 0xff) as u8, (n & 0xff) as u8)
        }
        3 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            let r = ((n >> 8) & 0xf) as u8;
            let g = ((n >> 4) & 0xf) as u8;
            let b = (n & 0xf) as u8;
            (r * 17, g * 17, b * 17)
        }
        _ => return None,
    };
    Some(COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_hex_6_digit() {
        let c = parse_hex("#1e1e2e").unwrap();
        assert_eq!(c.0 & 0xff, 0x1e);        // R
        assert_eq!((c.0 >> 8) & 0xff, 0x1e); // G
        assert_eq!((c.0 >> 16) & 0xff, 0x2e);// B
    }

    #[test]
    fn parse_hex_3_digit() {
        let c = parse_hex("#fff").unwrap();
        assert_eq!(c.0, 0xffffff);
    }

    #[test]
    fn parse_hex_no_prefix() {
        assert!(parse_hex("ffffff").is_none());
    }

    #[test]
    fn build_rows_with_config() {
        use cheime_model::{Candidate, CandidateId, DeploymentGeneration, Revision, SessionEpoch, SessionStatus};
        let snap = CandidateSnapshot {
            epoch: SessionEpoch::new(1),
            revision: Revision::new(1),
            deployment: DeploymentGeneration::new(1),
            page: 0,
            page_size: 10,
            preedit: "ni".into(),
            cursor: 2,
            candidates: vec![
                Candidate { id: CandidateId::new(1), text: "你".into(), annotation: Some("ni3".into()), source: "dict".into(), is_emoji: false },
                Candidate { id: CandidateId::new(2), text: "尼".into(), annotation: None, source: "dict".into(), is_emoji: false },
            ],
            highlighted: Some(CandidateId::new(1)),
            status: SessionStatus::Composing,
        };
        let cfg = cheime_tip_core::ui_config::CandidateConfig::default();
        let rows = build_rows(&snap, 22, 18, &cfg);
        assert!(rows.len() >= 2, "preedit + at least 1 candidate");
        // First row = preedit, not highlighted
        assert!(!rows[0].highlighted);
    }
}
