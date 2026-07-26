//! GDI candidate window — config-driven rendering.
//!
//! All visual parameters come from `UiConfig`. No hardcoded sizes or colors.
//! The config is loaded by the TIP at startup and stored in `WindowContext`.

use crate::edit_session::request_edit_session;
use crate::io_thread::{PostedAction, WM_CHEIME_ACTION, WM_CHEIME_SNAPSHOT, WM_CHEIME_STATUS};
use crate::rollback_guard::{GuardEvent, RollbackGuard};
use crate::tsf_interfaces::{ComTip, tsf_log};
use cheime_model::CandidateSnapshot;
use cheime_protocol::FrontendMessage;
use cheime_tip_core::ui_config::{AntialiasMode, LayoutType, PreeditType, StyleConfig, UiConfig};
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
    ANTIALIASED_QUALITY, BeginPaint, CLEARTYPE_QUALITY, COLOR_WINDOW, COLOR_WINDOWTEXT,
    ClientToScreen, CreateFontW, CreateRectRgn, CreateRoundRectRgn, CreateSolidBrush,
    DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject, EndPaint, FF_DONTCARE, FW_NORMAL, GetSysColor,
    HBRUSH, HDC, HFONT, InvalidateRect, NONANTIALIASED_QUALITY,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, RDW_ERASE, RDW_INVALIDATE, RedrawWindow, SelectObject,
    SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT, TextOutW,
};
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArcI, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath,
    GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
    GdipDeletePen, GdipDrawPath, GdipFillPath, GdipSetSmoothingMode, GdiplusStartup,
    GdiplusStartupInput, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill,
    SmoothingModeAntiAlias8x8, UnitPixel,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfContextView, ITfEditSession, ITfEditSession_Vtbl, ITfRange, ITfThreadMgr,
    TF_ANCHOR_START, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_ES_SYNC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, HMENU, HWND_TOPMOST, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_LONG_PTR_INDEX, WM_CREATE,
    WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WNDCLASS_STYLES, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, Interface};

const CANDIDATE_WINDOW_CLASS: &str = "CheIME_CandidateWindow";
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_CHEIME_RELOAD_CONFIG: u32 = 0x8000 + 0x124;

// ── COM constants (local copies to avoid coupling) ────────────────────
const S_OK: HRESULT = HRESULT(0);
const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);

/// One-time guard for `RegisterClassW` (Fix 4: prevents GDI brush leak).
static REGISTER_WNDCLASS: Once = Once::new();
static START_GDIPLUS: Once = Once::new();

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
    pub rollback_guard: Mutex<RollbackGuard>,
    pub rollback_anchor: Mutex<Option<windows::Win32::UI::TextServices::ITfRange>>,
    pub tip: *mut ComTip,
    pub render: Mutex<RenderState>,
}

impl Drop for WindowContext {
    fn drop(&mut self) {
        let font = self
            .render
            .get_mut()
            .map(|state| state.font)
            .unwrap_or_default();
        if !font.is_invalid() {
            unsafe {
                let _ = DeleteObject(font);
            }
        }
    }
}

pub struct RenderState {
    pub config: UiConfig,
    pub dark_mode: bool,
    pub font: HFONT,
}

pub struct CandidateWindow {
    hwnd: HWND,
    pub ctx_ptr: *const WindowContext,
}

/// Create a GDI font for the given pixel size (Microsoft YaHei, normal weight).
fn create_gdi_font(font_size: i32, font_face: &str, antialias: AntialiasMode) -> HFONT {
    let face: Vec<u16> = font_face.encode_utf16().chain(std::iter::once(0)).collect();
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
            match antialias {
                AntialiasMode::Default => DEFAULT_QUALITY.0,
                AntialiasMode::ForceDword | AntialiasMode::Cleartype => CLEARTYPE_QUALITY.0,
                AntialiasMode::Grayscale => ANTIALIASED_QUALITY.0,
                AntialiasMode::Aliased => NONANTIALIASED_QUALITY.0,
            } as u32,
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
        let font = create_gdi_font(
            config.style.font_point,
            &config.style.font_face,
            config.style.antialias_mode,
        );
        Box::new(WindowContext {
            snapshot: Mutex::new(None),
            thread_mgr,
            client_id,
            channel,
            composition: Mutex::new(None),
            rollback_guard: Mutex::new(RollbackGuard::default()),
            rollback_anchor: Mutex::new(None),
            tip,
            render: Mutex::new(RenderState {
                config,
                dark_mode: crate::ui_settings::system_uses_dark_theme(),
                font,
            }),
        })
    }
}

impl WindowContext {
    pub fn has_rollback_guard(&self) -> bool {
        self.rollback_guard
            .lock()
            .is_ok_and(|guard| guard.is_armed())
    }

    pub fn disarm_rollback(&self, event: GuardEvent) {
        if let Ok(mut guard) = self.rollback_guard.lock() {
            guard.observe(event);
        }
        if let Ok(mut anchor) = self.rollback_anchor.lock() {
            anchor.take();
        }
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
                    let Ok(render) = ctx.render.lock() else {
                        unsafe {
                            let _ = EndPaint(hwnd, &ps);
                        };
                        return LRESULT(0);
                    };
                    let scheme = render.config.active_scheme(render.dark_mode);
                    let background = parse_hex(&scheme.back_color)
                        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOW) }));
                    let mut client = RECT::default();
                    if unsafe { GetClientRect(hwnd, &mut client) }.is_ok() {
                        draw_window_surface(
                            hdc,
                            client,
                            &render.config,
                            render.dark_mode,
                            background,
                        );
                    }
                    if let Ok(st) = ctx.snapshot.lock() {
                        if let Some((_, rows)) = st.as_ref() {
                            // Fix 3: use cached font instead of creating one per paint.
                            unsafe {
                                paint(hdc, rows, &render.config, render.font);
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

        WM_CHEIME_RELOAD_CONFIG => {
            if let Some(ctx) = ctx() {
                let snapshot = ctx
                    .snapshot
                    .lock()
                    .ok()
                    .and_then(|state| state.as_ref().map(|(snapshot, _)| snapshot.clone()));
                if let Some(snapshot) = snapshot {
                    let raw = Box::into_raw(Box::new(snapshot));
                    return handle_snapshot(hwnd, LPARAM(raw as isize), Some(ctx));
                }
            }
            LRESULT(0)
        }

        WM_CHEIME_ACTION => handle_action(lparam, ctx()),

        WM_CHEIME_STATUS => {
            if lparam.0 != 0 {
                let status = unsafe { Box::from_raw(lparam.0 as *mut (bool, String)) };
                tsf_log(&format!(
                    "[CheIME] WM_STATUS connected={} detail={}",
                    status.0, status.1
                ));
                if !status.0 {
                    if let Some(ctx) = ctx() {
                        ctx.disarm_rollback(GuardEvent::FocusChanged);
                    }
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

fn draw_window_surface(
    hdc: HDC,
    bounds: RECT,
    config: &UiConfig,
    dark_mode: bool,
    background: COLORREF,
) {
    with_antialiased_graphics(hdc, |graphics| unsafe {
        let radius = config.style.layout.corner_radius.max(0);
        let path = rounded_path(bounds, radius);
        if path.is_null() {
            return;
        }
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        if GdipCreateSolidFill(colorref_to_argb(background), &mut brush).0 == 0 {
            let _ = GdipFillPath(graphics, brush.cast::<GpBrush>(), path);
            let _ = GdipDeleteBrush(brush.cast::<GpBrush>());
        }
        let width = config.style.layout.border_width.max(0);
        if width > 0 {
            let color = parse_hex(&config.active_scheme(dark_mode).border_color)
                .unwrap_or(COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
            let mut pen: *mut GpPen = std::ptr::null_mut();
            if GdipCreatePen1(colorref_to_argb(color), width as f32, UnitPixel, &mut pen).0 == 0 {
                let _ = GdipDrawPath(graphics, pen, path);
                let _ = GdipDeletePen(pen);
            }
        }
        let _ = GdipDeletePath(path);
    });
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

    if lparam.0 != 0 {
        let boxed: Box<CandidateSnapshot> =
            unsafe { Box::from_raw(lparam.0 as *mut CandidateSnapshot) };
        let fresh = crate::ui_settings::load_config();
        let dark_mode = crate::ui_settings::system_uses_dark_theme();
        if let Ok(mut render) = ctx.render.lock() {
            let font_changed = render.config.style.font_point != fresh.style.font_point
                || render.config.style.font_face != fresh.style.font_face
                || render.config.style.antialias_mode != fresh.style.antialias_mode;
            if font_changed {
                let replacement = create_gdi_font(
                    fresh.style.font_point,
                    &fresh.style.font_face,
                    fresh.style.antialias_mode,
                );
                let old = std::mem::replace(&mut render.font, replacement);
                if !old.is_invalid() {
                    unsafe {
                        let _ = DeleteObject(old);
                    }
                }
            }
            render.config = fresh.clone();
            render.dark_mode = dark_mode;
        }
        let cfg = &fresh;
        tsf_log(&format!(
            "[CheIME] WM_SNAPSHOT preedit={} candidates={}",
            boxed.preedit,
            boxed.candidates.len()
        ));

        let char_width = cfg.style.font_point.max(1);
        let line_height =
            (cfg.style.font_point + cfg.style.layout.hilite_padding_y.max(0) * 2).max(1);
        let (rows, content_width, content_height) =
            build_rows(&boxed, line_height, char_width, &cfg.style);
        let max_width = cfg.style.layout.max_width;
        let mut window_width = content_width.max(cfg.style.layout.min_width).max(1);
        if max_width > 0 {
            window_width = window_width.min(max_width);
        }
        let window_height = content_height.max(1);
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
                    left + cfg.style.layout.caret_offset_x,
                    bottom + cfg.style.layout.caret_offset_y,
                )
            })
            .unwrap_or_else(|| {
                tsf_log("[CheIME] GetTextExt failed, using config offsets");
                (
                    cfg.style.layout.caret_offset_x,
                    cfg.style.layout.caret_offset_y,
                )
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
            apply_corner_radius(
                hwnd,
                window_width,
                window_height,
                cfg.style.layout.corner_radius,
            );
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
        let posted: Box<PostedAction> = unsafe { Box::from_raw(lparam.0 as *mut PostedAction) };
        tsf_log(&format!("[CheIME] WM_ACTION action={:?}", posted.action));
        match unsafe { ctx.thread_mgr.GetFocus() } {
            Ok(doc) => match unsafe { doc.GetTop() } {
                Ok(context) => {
                    tsf_log("[CheIME] WM_ACTION: requesting edit session");
                    if !ctx.tip.is_null() {
                        unsafe { (*ctx.tip).suppress_text_edit_notifications(true) };
                    }
                    request_edit_session(
                        ctx.client_id,
                        &context,
                        posted.action,
                        posted.token,
                        &ctx.channel as *const SyncSender<FrontendMessage>,
                        &ctx.composition as *const Mutex<Option<ITfComposition>>,
                        &ctx.rollback_guard as *const Mutex<RollbackGuard>,
                        &ctx.rollback_anchor
                            as *const Mutex<Option<windows::Win32::UI::TextServices::ITfRange>>,
                    );
                    if !ctx.tip.is_null() {
                        unsafe { (*ctx.tip).suppress_text_edit_notifications(false) };
                    }
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

fn handle_mouse_move(hwnd: HWND, lparam: LPARAM, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    let mut track = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        ..Default::default()
    };
    unsafe {
        let _ = TrackMouseEvent(&mut track);
    }

    let x = (lparam.0 as u16) as i32;
    let y = ((lparam.0 >> 16) as u16) as i32;
    if let Ok(mut guard) = ctx.snapshot.lock() {
        if let Some((snapshot, rows)) = guard.as_mut() {
            let hovered = rows.iter().find_map(|row| {
                let hit = x >= row.bounds.left
                    && x < row.bounds.right
                    && y >= row.bounds.top
                    && y < row.bounds.bottom;
                hit.then_some(row.candidate_index).flatten()
            });
            let highlighted =
                hovered.and_then(|index| snapshot.candidates.get(index).map(|c| c.id));
            let changed = rows.iter().any(|row| {
                row.highlighted
                    != row
                        .candidate_index
                        .and_then(|index| snapshot.candidates.get(index))
                        .is_some_and(|candidate| Some(candidate.id) == highlighted)
            });
            for row in rows {
                row.highlighted = row
                    .candidate_index
                    .and_then(|index| snapshot.candidates.get(index))
                    .is_some_and(|candidate| Some(candidate.id) == highlighted);
            }
            if changed {
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
        }
    }
    LRESULT(0)
}

fn handle_mouse_leave(hwnd: HWND, ctx: Option<&WindowContext>) -> LRESULT {
    let Some(ctx) = ctx else { return LRESULT(0) };
    if let Ok(mut guard) = ctx.snapshot.lock() {
        if let Some((snapshot, rows)) = guard.as_mut() {
            for row in rows {
                row.highlighted = row
                    .candidate_index
                    .and_then(|index| snapshot.candidates.get(index))
                    .is_some_and(|candidate| Some(candidate.id) == snapshot.highlighted);
            }
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }
    LRESULT(0)
}

// ── Rendering ─────────────────────────────────────────────────────────

// Fix 3: accept cached font handle; no longer creates a font per paint call.
unsafe fn paint(hdc: HDC, rows: &[RowRender], config: &UiConfig, font: HFONT) {
    let scheme = config.active_scheme(crate::ui_settings::system_uses_dark_theme());
    let fg = parse_hex(&scheme.candidate_text_color)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }));
    let selected_fg = parse_hex(&scheme.hilited_candidate_text_color).unwrap_or(fg);
    let selected_bg = parse_hex(&scheme.hilited_candidate_back_color)
        .unwrap_or(COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }));

    let old = unsafe { SelectObject(hdc, font) };

    for row in rows {
        unsafe {
            if row.highlighted {
                draw_selection_box(hdc, row, config, selected_bg);
            }
            SetTextColor(hdc, if row.highlighted { selected_fg } else { fg });
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
    config: &StyleConfig,
) -> (Vec<RowRender>, i32, i32) {
    let mut rows = Vec::new();
    let pad_x = config.layout.margin_x.max(0);
    let pad_y = config.layout.margin_y.max(0);
    let mut y = pad_y;

    let preedit = if config.inline_preedit {
        String::new()
    } else {
        match config.preedit_type {
            PreeditType::Composition => snapshot.preedit.clone(),
            PreeditType::Preview => snapshot
                .highlighted
                .and_then(|id| {
                    snapshot
                        .candidates
                        .iter()
                        .find(|candidate| candidate.id == id)
                })
                .map(|candidate| candidate.text.clone())
                .unwrap_or_else(|| snapshot.preedit.clone()),
            PreeditType::PreviewAll => snapshot
                .candidates
                .iter()
                .take(config.page_size.max(1))
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        }
    };

    if !preedit.is_empty() {
        let width = text_pixel_width(&preedit, char_width);
        rows.push(RowRender {
            text: preedit.encode_utf16().collect(),
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
        y += line_height + config.layout.spacing.max(0);
    }

    let candidates = snapshot
        .candidates
        .iter()
        .take(config.page_size.max(1))
        .enumerate()
        .map(|(index, candidate)| {
            let mut text = String::new();
            use std::fmt::Write;
            if snapshot.highlighted == Some(candidate.id) && !config.mark_text.is_empty() {
                text.push_str(&config.mark_text);
                text.push(' ');
            }
            if config.show_labels {
                let label = if index == 9 { 0 } else { index + 1 };
                text.push_str(&config.label_format.replace("%s", &label.to_string()));
                text.push(' ');
            }
            let candidate_text = if config.candidate_abbreviate_length > 0
                && candidate.text.chars().count() > config.candidate_abbreviate_length
            {
                let mut shortened = candidate
                    .text
                    .chars()
                    .take(config.candidate_abbreviate_length)
                    .collect::<String>();
                shortened.push('…');
                shortened
            } else {
                candidate.text.clone()
            };
            text.push_str(&candidate_text);
            if let Some(annotation) = &candidate.annotation {
                let _ = write!(text, " {annotation}");
            }
            (index, candidate, text)
        })
        .collect::<Vec<_>>();

    match config.layout.r#type {
        LayoutType::Vertical => {
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
                y += line_height + config.layout.candidate_spacing.max(0);
            }
        }
        LayoutType::Horizontal => {
            let mut x = 0;
            for (index, candidate, text) in candidates {
                let width = text_pixel_width(&text, char_width);
                let right = x + width + config.layout.hilite_padding_x.max(0) * 2;
                rows.push(RowRender {
                    text: text.encode_utf16().collect(),
                    x: x + config.layout.hilite_padding_x.max(0),
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
                x = right + config.layout.candidate_spacing.max(0);
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
    let bounds = row.bounds;
    let configured_radius = config.style.layout.hilited_corner_radius;
    let radius = clamped_corner_radius(
        bounds.right - bounds.left,
        bounds.bottom - bounds.top,
        configured_radius,
    );
    with_antialiased_graphics(hdc, |graphics| unsafe {
        let path = rounded_path(bounds, radius);
        if path.is_null() {
            return;
        }
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        if GdipCreateSolidFill(colorref_to_argb(outline), &mut brush).0 == 0 {
            let _ = GdipFillPath(graphics, brush.cast::<GpBrush>(), path);
            let _ = GdipDeleteBrush(brush.cast::<GpBrush>());
        }
        let _ = GdipDeletePath(path);
    });
}

fn ensure_gdiplus() {
    START_GDIPLUS.call_once(|| {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        unsafe {
            let _ = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        }
        // GDI+ intentionally remains initialized for the lifetime of the TIP DLL.
    });
}

fn with_antialiased_graphics(hdc: HDC, draw: impl FnOnce(*mut GpGraphics)) {
    ensure_gdiplus();
    let mut graphics: *mut GpGraphics = std::ptr::null_mut();
    unsafe {
        if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
            return;
        }
        let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
        draw(graphics);
        let _ = GdipDeleteGraphics(graphics);
    }
}

unsafe fn rounded_path(bounds: RECT, radius: i32) -> *mut GpPath {
    let mut path: *mut GpPath = std::ptr::null_mut();
    if unsafe { GdipCreatePath(FillModeAlternate, &mut path) }.0 != 0 {
        return std::ptr::null_mut();
    }
    let width = (bounds.right - bounds.left).max(1);
    let height = (bounds.bottom - bounds.top).max(1);
    let diameter = (radius.max(0) * 2).min(width).min(height);
    if diameter == 0 {
        unsafe {
            let _ = GdipAddPathArcI(path, bounds.left, bounds.top, 1, 1, 0.0, 90.0);
            let _ = GdipAddPathArcI(path, bounds.right - 1, bounds.top, 1, 1, 90.0, 90.0);
            let _ = GdipAddPathArcI(path, bounds.right - 1, bounds.bottom - 1, 1, 1, 180.0, 90.0);
            let _ = GdipAddPathArcI(path, bounds.left, bounds.bottom - 1, 1, 1, 270.0, 90.0);
        }
    } else {
        unsafe {
            let _ = GdipAddPathArcI(
                path,
                bounds.left,
                bounds.top,
                diameter,
                diameter,
                180.0,
                90.0,
            );
            let _ = GdipAddPathArcI(
                path,
                bounds.right - diameter,
                bounds.top,
                diameter,
                diameter,
                270.0,
                90.0,
            );
            let _ = GdipAddPathArcI(
                path,
                bounds.right - diameter,
                bounds.bottom - diameter,
                diameter,
                diameter,
                0.0,
                90.0,
            );
            let _ = GdipAddPathArcI(
                path,
                bounds.left,
                bounds.bottom - diameter,
                diameter,
                diameter,
                90.0,
                90.0,
            );
        }
    }
    unsafe {
        let _ = GdipClosePathFigure(path);
    }
    path
}

fn colorref_to_argb(color: COLORREF) -> u32 {
    let red = color.0 & 0xff;
    let green = (color.0 >> 8) & 0xff;
    let blue = (color.0 >> 16) & 0xff;
    0xff00_0000 | (red << 16) | (green << 8) | blue
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

/// Parse a CSS-style `#RRGGBB` color into Windows' native COLORREF.
fn parse_hex(s: &str) -> Option<COLORREF> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((n >> 16) as u8, ((n >> 8) & 0xff) as u8, (n & 0xff) as u8);
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
    fn parse_hex_rejects_3_digit_shorthand() {
        assert!(parse_hex("#fff").is_none());
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
        let cfg = StyleConfig::default();
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
        let cfg = StyleConfig {
            layout: cheime_tip_core::ui_config::LayoutConfig {
                r#type: LayoutType::Horizontal,
                ..Default::default()
            },
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
        assert_eq!(height, 66);
    }

    #[test]
    fn inline_preedit_removes_candidate_window_preedit_row() {
        use cheime_model::{DeploymentGeneration, Revision, SessionEpoch, SessionStatus};
        let snap = CandidateSnapshot {
            epoch: SessionEpoch::new(1),
            revision: Revision::new(1),
            deployment: DeploymentGeneration::new(1),
            page: 0,
            page_size: 5,
            preedit: "nihao".into(),
            cursor: 5,
            candidates: vec![],
            highlighted: None,
            status: SessionStatus::Composing,
        };
        let mut cfg = StyleConfig::default();
        cfg.inline_preedit = true;
        let (rows, _, _) = build_rows(&snap, 22, 18, &cfg);
        assert!(rows.iter().all(|row| row.candidate_index.is_some()));
    }

    #[test]
    fn corner_radius_is_clamped_to_half_height() {
        assert_eq!(clamped_corner_radius(300, 40, 100), 20);
        assert_eq!(clamped_corner_radius(300, 40, -1), 0);
    }
}
