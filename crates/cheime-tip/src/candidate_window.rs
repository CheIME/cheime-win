//! GDI candidate window — config-driven rendering.
//!
//! All visual parameters come from `UiConfig`. No hardcoded sizes or colors.
//! The config is loaded by the TIP at startup and stored in `WindowContext`.

use crate::edit_session::request_edit_session;
use crate::io_thread::{WM_CHEIME_ACTION, WM_CHEIME_SNAPSHOT, WM_CHEIME_STATUS};
use crate::tsf_interfaces::{ComTip, tsf_log};
use cheime_model::{CandidateSnapshot, PlatformAction};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::ui_config::{CandidateOrientation, UiConfig};
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering, fence};
use std::sync::mpsc::SyncSender;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, COLOR_WINDOWTEXT, ClientToScreen, CreateFontW, CreateRectRgn,
    CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject, EndPaint,
    FF_DONTCARE, FW_NORMAL, FillRect, FrameRgn, GetSysColor, HBRUSH, HDC, HFONT,
    InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RDW_ERASE, RDW_INVALIDATE, RedrawWindow,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT, TextOutW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfContextView, ITfEditSession, ITfEditSession_Vtbl, ITfRange, ITfThreadMgr,
    TF_ANCHOR_START, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_ES_SYNC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, HMENU, HWND_TOPMOST, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_LONG_PTR_INDEX, WM_CREATE,
    WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSELEAVE, WM_MOUSEMOVE, WM_PAINT,
    WNDCLASS_STYLES, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, Interface};

const CANDIDATE_WINDOW_CLASS: &str = "CheIME_CandidateWindow";

// ── COM constants (local copies to avoid coupling) ────────────────────
const S_OK: HRESULT = HRESULT(0);
const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);

/// One-time guard for `RegisterClassW` (Fix 4: prevents GDI brush leak).
static REGISTER_WNDCLASS: Once = Once::new();

pub type SnapshotBox = Mutex<Option<(CandidateSnapshot, Vec<RowRender>)>>;

pub struct RowRender {
    pub text: Vec<u16>,
    pub x: i32,
    pub y: i32,
    pub bounds: RECT,
    pub candidate_index: Option<usize>,
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
    /// Cached GDI font handle; created once, freed on drop (Fix 3).
    pub cached_font: HFONT,
}

impl Drop for WindowContext {
    fn drop(&mut self) {
        if !self.cached_font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.cached_font);
            }
        }
    }
}

pub struct CandidateWindow {
    hwnd: HWND,
    pub ctx_ptr: *const WindowContext,
}

/// Create a GDI font for the given pixel size (Microsoft YaHei, normal weight).
fn create_gdi_font(font_size: i32) -> HFONT {
    let face: Vec<u16> = "Microsoft YaHei\0".encode_utf16().collect();
    unsafe {
        CreateFontW(
            font_size,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            0,
            DEFAULT_QUALITY.0 as u32,
            FF_DONTCARE.0 as u32 | DEFAULT_CHARSET.0 as u32,
            windows::core::PCWSTR::from_raw(face.as_ptr()),
        )
    }
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

        // Fix 4: Register class only once to avoid GDI brush leak.
        REGISTER_WNDCLASS.call_once(|| {
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
        });

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                windows::core::PCWSTR::from_raw(class_wide.as_ptr()),
                windows::core::w!("CheIME Candidate"),
                WS_POPUP,
                -1000,
                -1000,
                200,
                100,
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
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn new_context(
        thread_mgr: ITfThreadMgr,
        client_id: u32,
        channel: SyncSender<FrontendMessage>,
        tip: *mut ComTip,
    ) -> Box<WindowContext> {
        let config = crate::ui_settings::load_config();
        let cached_font = create_gdi_font(config.candidate.font_size);
        Box::new(WindowContext {
            snapshot: Mutex::new(None),
            thread_mgr,
            client_id,
            channel,
            composition: Mutex::new(None),
            tip,
            cached_font,
            config,
        })
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
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
        if p.is_null() {
            None
        } else {
            Some(unsafe { &*p })
        }
    };

    match msg {
        WM_CREATE => LRESULT(0),

        WM_ERASEBKGND => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                if let Some(ctx) = ctx() {
                    let background = parse_hex(&ctx.config.theme.colors.background)
                        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOW) }));
                    let brush = unsafe { CreateSolidBrush(background) };
                    let mut client = RECT::default();
                    if unsafe { GetClientRect(hwnd, &mut client) }.is_ok() {
                        unsafe {
                            FillRect(hdc, &client, brush);
                        }
                    }
                    unsafe {
                        let _ = DeleteObject(brush);
                    }
                    if let Ok(st) = ctx.snapshot.lock() {
                        if let Some((_, rows)) = st.as_ref() {
                            // Fix 3: use cached font instead of creating one per paint.
                            unsafe {
                                paint(hdc, rows, &ctx.config, ctx.cached_font);
                            }
                        }
                    }
                }
                unsafe {
                    let _ = EndPaint(hwnd, &ps);
                }
            }
            LRESULT(0)
        }

        WM_CHEIME_SNAPSHOT => handle_snapshot(hwnd, lparam, ctx()),

        WM_CHEIME_ACTION => handle_action(lparam, ctx()),

        WM_CHEIME_STATUS => {
            if lparam.0 != 0 {
                let status = unsafe { Box::from_raw(lparam.0 as *mut (bool, String)) };
                tsf_log(&format!(
                    "[CheIME] WM_STATUS connected={} detail={}",
                    status.0, status.1
                ));
                if !status.0 {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => handle_click(lparam, ctx()),

        WM_MOUSEMOVE => handle_mouse_move(hwnd, lparam, ctx()),

        WM_MOUSELEAVE => handle_mouse_leave(hwnd, ctx()),

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

/// Get the screen (left, bottom) of the composition text via a synchronous
/// Get the screen position for the candidate window.
/// Tries: (1) TSF GetTextExt, (2) GetGUIThreadInfo caret rect.
fn get_composition_screen_rect(ctx: &WindowContext) -> Option<(i32, i32)> {
    // Try 1: TSF GetTextExt via edit session
    if let Some(pos) = try_get_text_ext(ctx) {
        tsf_log(&format!("[CheIME] GetTextExt OK: ({}, {})", pos.0, pos.1));
        return Some(pos);
    }

    // Try 2: GetGUIThreadInfo — returns the caret rect in screen coordinates
    // This works with TSF applications that may not have a system caret.
    use windows::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo};
    let mut gui_info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // Thread 0 = foreground thread
    if unsafe { GetGUIThreadInfo(0, &mut gui_info) }.is_ok() {
        let rc = gui_info.rcCaret;
        if rc.left != 0 || rc.right != 0 {
            // rcCaret is in client coordinates of hwndCaret
            let hwnd = gui_info.hwndCaret;
            if !hwnd.is_invalid() {
                let mut screen_point = POINT {
                    x: rc.left,
                    y: rc.bottom,
                };
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut screen_point);
                };
                tsf_log(&format!(
                    "[CheIME] GetGUIThreadInfo: caret=({}, {}) screen=({}, {})",
                    rc.left, rc.bottom, screen_point.x, screen_point.y
                ));
                return Some((screen_point.x, screen_point.y));
            }
        }
    }

    // Try 3: ITfContextView::GetScreenExt — returns the entire document area.
    // Less precise than GetTextExt but works in more applications (e.g. Explorer).
    if let Ok(doc) = unsafe { ctx.thread_mgr.GetFocus() } {
        if let Ok(context) = unsafe { doc.GetTop() } {
            if let Ok(view) = unsafe { context.GetActiveView() } {
                let rc = unsafe { view.GetScreenExt() };
                if let Ok(rc) = rc {
                    if rc.left != 0 || rc.top != 0 || rc.right != 0 || rc.bottom != 0 {
                        tsf_log(&format!(
                            "[CheIME] GetScreenExt: left={} top={} right={} bottom={}",
                            rc.left, rc.top, rc.right, rc.bottom
                        ));
                        // Position at bottom-left of the document area
                        return Some((rc.left, rc.bottom));
                    }
                }
            }
        }
    }

    tsf_log("[CheIME] All cursor position methods failed");
    None
}

/// Try GetTextExt via TSF edit session.
/// Tries composition range first, then falls back to selection range.
fn try_get_text_ext(ctx: &WindowContext) -> Option<(i32, i32)> {
    let doc = unsafe { ctx.thread_mgr.GetFocus() }.ok()?;
    let context = unsafe { doc.GetTop() }.ok()?;
    let view = unsafe { context.GetActiveView() }.ok()?;

    // Try composition range first
    let range = {
        let comp_guard = ctx.composition.lock().ok()?;
        comp_guard
            .as_ref()
            .and_then(|comp| unsafe { comp.GetRange() }.ok())
    };

    // If no composition range, use current selection range
    let range = match range {
        Some(r) => r,
        None => {
            use windows::Win32::UI::TextServices::{TF_DEFAULT_SELECTION, TF_SELECTION};
            let mut sel = [TF_SELECTION::default()];
            let mut fetched = 0u32;
            if unsafe {
                context.GetSelection(0xFFFFFFFFu32, TF_DEFAULT_SELECTION, &mut sel, &mut fetched)
            }
            .is_err()
                || fetched == 0
            {
                return None;
            }
            unsafe { sel[0].range.as_ref()?.Clone() }.ok()?
        }
    };

    // Collapse to start for reliable point-based extent
    let _ = unsafe { range.Collapse(0, TF_ANCHOR_START) };

    let result = Cell::new(None::<RECT>);
    let session = TextExtentSession::new(view, range, &result as *const Cell<Option<RECT>>);
    let raw = Box::into_raw(session);
    let raw_void: *mut c_void = raw.cast();

    if let Some(session_ref) = unsafe { ITfEditSession::from_raw_borrowed(&raw_void) } {
        let flags = TF_CONTEXT_EDIT_CONTEXT_FLAGS(TF_ES_SYNC.0);
        let _ = unsafe { context.RequestEditSession(ctx.client_id, session_ref, flags) };
    }

    unsafe { TextExtentSession::release(raw_void) };

    result.take().map(|r| (r.left, r.bottom))
}

fn handle_snapshot(hwnd: HWND, lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    let cfg = &ctx.config;

    if lparam.0 != 0 {
        let boxed: Box<CandidateSnapshot> =
            unsafe { Box::from_raw(lparam.0 as *mut CandidateSnapshot) };
        tsf_log(&format!(
            "[CheIME] WM_SNAPSHOT preedit={} candidates={}",
            boxed.preedit,
            boxed.candidates.len()
        ));

        let char_width = cfg
            .candidate
            .char_width
            .unwrap_or(cfg.candidate.font_size)
            .max(1);
        let line_height = cfg.candidate.line_height.max(1);
        let (rows, content_width, content_height) =
            build_rows(&boxed, line_height, char_width, &cfg.candidate);
        let window_width = content_width.max(cfg.window.min_width).max(1);
        let window_height = if cfg.window.height > 0 {
            cfg.window.height
        } else {
            content_height
        }
        .max(1);
        // Sync has_composition from engine preedit
        if !ctx.tip.is_null() {
            unsafe {
                (*ctx.tip).has_composition.set(!boxed.preedit.is_empty());
            }
        }

        // Hide window when there's no composition (e.g. after Backspace clears all)
        if boxed.preedit.is_empty() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            return LRESULT(0);
        }

        if let Ok(mut st) = ctx.snapshot.lock() {
            *st = Some((*boxed, rows));
        }

        // Fix 1: Position window below composition text via GetTextExt.
        let (x, y) = get_composition_screen_rect(ctx)
            .map(|(left, bottom)| {
                (
                    left + cfg.window.caret_offset_x,
                    bottom + cfg.window.caret_offset_y,
                )
            })
            .unwrap_or_else(|| {
                tsf_log("[CheIME] GetTextExt failed, using config offsets");
                (cfg.window.caret_offset_x, cfg.window.caret_offset_y)
            });

        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                window_width,
                window_height,
                SWP_NOACTIVATE,
            );
            apply_corner_radius(hwnd, window_width, window_height, cfg.window.corner_radius);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_ERASE);
        }
    } else {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    LRESULT(0)
}

fn handle_action(lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    if lparam.0 != 0 {
        let action: Box<PlatformAction> = unsafe { Box::from_raw(lparam.0 as *mut PlatformAction) };
        tsf_log(&format!("[CheIME] WM_ACTION action={action:?}"));
        match unsafe { ctx.thread_mgr.GetFocus() } {
            Ok(doc) => match unsafe { doc.GetTop() } {
                Ok(context) => {
                    tsf_log("[CheIME] WM_ACTION: requesting edit session");
                    request_edit_session(
                        ctx.client_id,
                        &context,
                        *action,
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

// Fix 2: Single lock scope — eliminates TOCTOU race between hit_test and candidate lookup.
fn handle_click(lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    let x = (lparam.0 as u16) as i32;
    let y = ((lparam.0 >> 16) as u16) as i32;

    if let Ok(guard) = ctx.snapshot.lock() {
        if let Some((snap, rows)) = guard.as_ref() {
            let hit_index = rows.iter().find_map(|row| {
                let hit = x >= row.bounds.left
                    && x < row.bounds.right
                    && y >= row.bounds.top
                    && y < row.bounds.bottom;
                hit.then_some(row.candidate_index).flatten()
            });
            if let Some(idx) = hit_index {
                let candidate = snap.candidates.get(idx);
                if let Some(cand) = candidate {
                    tsf_log(&format!("[CheIME] Click select: {}", cand.text));
                    let _ = ctx.channel.try_send(FrontendMessage::UiCommand {
                        header: cheime_protocol::MessageHeader {
                            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
                            client: cheime_model::ClientInstanceId::new(1),
                            session: cheime_model::SessionId::new(1),
                            epoch: cheime_model::SessionEpoch::new(1),
                            sequence: cheime_model::Sequence::new(0),
                            revision: cheime_model::Revision::new(0),
                            deployment: cheime_model::DeploymentGeneration::new(1),
                        },
                        command: cheime_model::UiCommand::SelectCandidate {
                            epoch: snap.epoch,
                            snapshot_revision: snap.revision,
                            candidate_id: cand.id,
                        },
                    });
                }
            }
        }
    }
    LRESULT(0)
}

// ── Rendering ─────────────────────────────────────────────────────────

// Fix 3: accept cached font handle; no longer creates a font per paint call.
unsafe fn paint(hdc: HDC, rows: &[RowRender], config: &UiConfig, font: HFONT) {
    let fg = parse_hex(&config.theme.colors.candidate_text)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }));
    let outline = parse_hex(&config.selection_box.outline_color)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }));

    let old = unsafe { SelectObject(hdc, font) };

    for row in rows {
        unsafe {
            if row.highlighted {
                draw_selection_box(hdc, row, config, outline);
            }
            SetTextColor(hdc, fg);
            SetBkMode(hdc, TRANSPARENT);
            let _ = TextOutW(hdc, row.x, row.y, &row.text);
        }
    }
    if !old.is_invalid() {
        unsafe {
            SelectObject(hdc, old);
        }
    }
    // Do NOT delete the font — it is cached in WindowContext.
}

fn build_rows(
    snapshot: &CandidateSnapshot,
    line_height: i32,
    char_width: i32,
    config: &cheime_tip_core::ui_config::CandidateConfig,
) -> (Vec<RowRender>, i32, i32) {
    let mut rows = Vec::new();
    let pad_x = config.row_padding_x.max(0);
    let pad_y = config.row_padding_y.max(0);
    let mut y = pad_y;

    if !snapshot.preedit.is_empty() {
        let width = text_pixel_width(&snapshot.preedit, char_width);
        rows.push(RowRender {
            text: snapshot.preedit.encode_utf16().collect(),
            x: pad_x,
            y,
            bounds: RECT {
                left: 0,
                top: y,
                right: width + pad_x * 2,
                bottom: y + line_height,
            },
            candidate_index: None,
            highlighted: false,
        });
        y += line_height;
    }

    let candidates = snapshot
        .candidates
        .iter()
        .take(config.page_size.max(1))
        .enumerate()
        .map(|(index, candidate)| {
            let mut text = String::new();
            use std::fmt::Write;
            if config.show_labels {
                let label = if index == 9 { 0 } else { index + 1 };
                let _ = write!(text, "{label}. ");
            }
            text.push_str(&candidate.text);
            if let Some(annotation) = &candidate.annotation {
                let _ = write!(text, " {annotation}");
            }
            (index, candidate, text)
        })
        .collect::<Vec<_>>();

    match config.orientation {
        CandidateOrientation::Vertical => {
            for (index, candidate, text) in candidates {
                let width = text_pixel_width(&text, char_width);
                rows.push(RowRender {
                    text: text.encode_utf16().collect(),
                    x: pad_x,
                    y,
                    bounds: RECT {
                        left: 0,
                        top: y,
                        right: width + pad_x * 2,
                        bottom: y + line_height,
                    },
                    candidate_index: Some(index),
                    highlighted: snapshot.highlighted == Some(candidate.id),
                });
                y += line_height;
            }
        }
        CandidateOrientation::Horizontal => {
            let mut x = 0;
            for (index, candidate, text) in candidates {
                let width = text_pixel_width(&text, char_width);
                let right = x + width + pad_x * 2;
                rows.push(RowRender {
                    text: text.encode_utf16().collect(),
                    x: x + pad_x,
                    y,
                    bounds: RECT {
                        left: x,
                        top: y,
                        right,
                        bottom: y + line_height,
                    },
                    candidate_index: Some(index),
                    highlighted: snapshot.highlighted == Some(candidate.id),
                });
                x = right;
            }
            if rows.iter().any(|row| row.candidate_index.is_some()) {
                y += line_height;
            }
        }
    }

    let width = rows
        .iter()
        .map(|row| row.bounds.right)
        .max()
        .unwrap_or(0)
        .max(pad_x * 2);
    let height = (y + pad_y).max(line_height);
    (rows, width, height)
}

fn text_pixel_width(text: &str, char_width: i32) -> i32 {
    text.chars()
        .map(|character| {
            if character.is_ascii() {
                (char_width + 1) / 2
            } else {
                char_width
            }
        })
        .sum()
}

unsafe fn draw_selection_box(hdc: HDC, row: &RowRender, config: &UiConfig, outline: COLORREF) {
    let Some(bounds) = scaled_selection_bounds(row.bounds, config.selection_box.relative_size)
    else {
        return;
    };
    let configured_radius = config
        .selection_box
        .corner_radius
        .unwrap_or(config.window.corner_radius);
    let radius = clamped_corner_radius(
        bounds.right - bounds.left,
        bounds.bottom - bounds.top,
        configured_radius,
    );
    let region = if radius == 0 {
        unsafe { CreateRectRgn(bounds.left, bounds.top, bounds.right, bounds.bottom) }
    } else {
        unsafe {
            CreateRoundRectRgn(
                bounds.left,
                bounds.top,
                bounds.right,
                bounds.bottom,
                radius * 2,
                radius * 2,
            )
        }
    };
    if !region.is_invalid() {
        let brush = unsafe { CreateSolidBrush(outline) };
        unsafe {
            let _ = FrameRgn(hdc, region, brush, 1, 1);
            let _ = DeleteObject(brush);
            let _ = DeleteObject(region);
        }
    }
}

fn scaled_selection_bounds(bounds: RECT, configured_size: f32) -> Option<RECT> {
    let scale = if configured_size.is_finite() {
        configured_size.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if scale == 0.0 {
        return None;
    }
    let width = (bounds.right - bounds.left).max(1);
    let height = (bounds.bottom - bounds.top).max(1);
    let scaled_width = ((width as f32 * scale).round() as i32).max(1);
    let scaled_height = ((height as f32 * scale).round() as i32).max(1);
    let left = bounds.left + (width - scaled_width) / 2;
    let top = bounds.top + (height - scaled_height) / 2;
    Some(RECT {
        left,
        top,
        right: left + scaled_width,
        bottom: top + scaled_height,
    })
}

unsafe fn apply_corner_radius(hwnd: HWND, width: i32, height: i32, configured_radius: i32) {
    let radius = clamped_corner_radius(width, height, configured_radius);
    let region = if radius == 0 {
        unsafe { CreateRectRgn(0, 0, width, height) }
    } else {
        unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2) }
    };
    if !region.is_invalid() {
        // SetWindowRgn takes ownership of the region on success.
        if unsafe { SetWindowRgn(hwnd, region, true) } == 0 {
            unsafe {
                let _ = DeleteObject(region);
            }
        }
    }
}

fn clamped_corner_radius(width: i32, height: i32, configured_radius: i32) -> i32 {
    configured_radius.max(0).min(height / 2).min(width / 2)
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
    Some(COLORREF(
        (r as u32) | ((g as u32) << 8) | ((b as u32) << 16),
    ))
}

// ── TextExtent edit session (for GetTextExt) ──────────────────────────

/// Lightweight COM callback that calls `ITfContextView::GetTextExt` inside a
/// synchronous edit session and stores the result in a `Cell`.
#[repr(C)]
struct TextExtentSession {
    vtbl: &'static ITfEditSession_Vtbl,
    ref_count: AtomicU32,
    view: ITfContextView,
    range: ITfRange,
    result: *const Cell<Option<RECT>>,
}

impl TextExtentSession {
    fn new(view: ITfContextView, range: ITfRange, result: *const Cell<Option<RECT>>) -> Box<Self> {
        Box::new(Self {
            vtbl: &TEXT_EXTENT_VTBL,
            ref_count: AtomicU32::new(1),
            view,
            range,
            result,
        })
    }

    unsafe fn from_raw(this: *mut c_void) -> *mut Self {
        this.cast()
    }

    unsafe fn add_ref(this: *mut c_void) -> u32 {
        let cb = unsafe { &*Self::from_raw(this) };
        cb.ref_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe fn release(this: *mut c_void) -> u32 {
        let cb = unsafe { &mut *Self::from_raw(this) };
        let prev = cb.ref_count.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            fence(Ordering::Acquire);
            unsafe { drop(Box::from_raw(Self::from_raw(this))) };
            0
        } else {
            prev - 1
        }
    }

    unsafe fn query_interface(
        this: *mut c_void,
        iid: *const windows::core::GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        unsafe { *out = std::ptr::null_mut() };
        if this.is_null() || iid.is_null() {
            return E_POINTER;
        }
        let guid = unsafe { *iid };
        if guid == IUnknown::IID || guid == ITfEditSession::IID {
            unsafe { Self::add_ref(this) };
            unsafe { *out = this };
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn tes_query_interface(
    this: *mut c_void,
    iid: *const windows::core::GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    unsafe { TextExtentSession::query_interface(this, iid, out) }
}

unsafe extern "system" fn tes_add_ref(this: *mut c_void) -> u32 {
    unsafe { TextExtentSession::add_ref(this) }
}

unsafe extern "system" fn tes_release(this: *mut c_void) -> u32 {
    unsafe { TextExtentSession::release(this) }
}

unsafe extern "system" fn tes_do_edit_session(this: *mut c_void, ec: u32) -> HRESULT {
    let session = unsafe { &*(this as *const TextExtentSession) };
    let mut rect = RECT::default();
    let mut clipped = BOOL(0);
    let hr = unsafe {
        session
            .view
            .GetTextExt(ec, &session.range, &mut rect, &mut clipped)
    };
    if hr.is_ok() {
        unsafe { (*session.result).set(Some(rect)) };
    }
    S_OK
}

static TEXT_EXTENT_VTBL: ITfEditSession_Vtbl = ITfEditSession_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: tes_query_interface,
        AddRef: tes_add_ref,
        Release: tes_release,
    },
    DoEditSession: tes_do_edit_session,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_hex_6_digit() {
        let c = parse_hex("#1e1e2e").unwrap();
        assert_eq!(c.0 & 0xff, 0x1e); // R
        assert_eq!((c.0 >> 8) & 0xff, 0x1e); // G
        assert_eq!((c.0 >> 16) & 0xff, 0x2e); // B
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
        use cheime_model::{
            Candidate, CandidateId, DeploymentGeneration, Revision, SessionEpoch, SessionStatus,
        };
        let snap = CandidateSnapshot {
            epoch: SessionEpoch::new(1),
            revision: Revision::new(1),
            deployment: DeploymentGeneration::new(1),
            page: 0,
            page_size: 10,
            preedit: "ni".into(),
            cursor: 2,
            candidates: vec![
                Candidate {
                    id: CandidateId::new(1),
                    text: "你".into(),
                    annotation: Some("ni3".into()),
                    source: "dict".into(),
                    is_emoji: false,
                },
                Candidate {
                    id: CandidateId::new(2),
                    text: "尼".into(),
                    annotation: None,
                    source: "dict".into(),
                    is_emoji: false,
                },
            ],
            highlighted: Some(CandidateId::new(1)),
            status: SessionStatus::Composing,
        };
        let cfg = cheime_tip_core::ui_config::CandidateConfig::default();
        let (rows, _, _) = build_rows(&snap, 22, 18, &cfg);
        assert!(rows.len() >= 2, "preedit + at least 1 candidate");
        // First row = preedit, not highlighted
        assert!(!rows[0].highlighted);
    }

    #[test]
    fn horizontal_layout_hides_labels_and_limits_candidates() {
        use cheime_model::{
            Candidate, CandidateId, DeploymentGeneration, Revision, SessionEpoch, SessionStatus,
        };
        let snap = CandidateSnapshot {
            epoch: SessionEpoch::new(1),
            revision: Revision::new(1),
            deployment: DeploymentGeneration::new(1),
            page: 0,
            page_size: 10,
            preedit: "ni".into(),
            cursor: 2,
            candidates: (0..3)
                .map(|index| Candidate {
                    id: CandidateId::new(index + 1),
                    text: format!("word{index}"),
                    annotation: None,
                    source: "dict".into(),
                    is_emoji: false,
                })
                .collect(),
            highlighted: Some(CandidateId::new(1)),
            status: SessionStatus::Composing,
        };
        let cfg = cheime_tip_core::ui_config::CandidateConfig {
            orientation: CandidateOrientation::Horizontal,
            show_labels: false,
            page_size: 2,
            ..Default::default()
        };
        let (rows, width, height) = build_rows(&snap, 22, 9, &cfg);
        let candidates = rows
            .iter()
            .filter(|row| row.candidate_index.is_some())
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].bounds.top, candidates[1].bounds.top);
        assert!(candidates[1].bounds.left >= candidates[0].bounds.right);
        assert_eq!(candidates[0].bounds.right - candidates[0].bounds.left, 45);
        assert!(!String::from_utf16_lossy(&candidates[0].text).contains("1."));
        assert!(width > 0);
        assert_eq!(height, 48);
    }

    #[test]
    fn corner_radius_is_clamped_to_half_height() {
        assert_eq!(clamped_corner_radius(300, 40, 100), 20);
        assert_eq!(clamped_corner_radius(300, 40, -1), 0);
    }

    #[test]
    fn selection_box_relative_size_is_centered_and_clamped() {
        let bounds = RECT {
            left: 10,
            top: 20,
            right: 110,
            bottom: 60,
        };
        assert_eq!(
            scaled_selection_bounds(bounds, 0.5),
            Some(RECT {
                left: 35,
                top: 30,
                right: 85,
                bottom: 50,
            })
        );
        assert_eq!(scaled_selection_bounds(bounds, 0.0), None);
        assert_eq!(scaled_selection_bounds(bounds, 2.0), Some(bounds));
    }
}
