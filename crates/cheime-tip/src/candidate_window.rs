//! GDI candidate window — minimal rendering with window proc.
//! NOTE: candidate window rendering uses raw GDI calls via windows crate 0.58.
//! `GetSysColor` returns `u32` but `SetTextColor`/`SetBkColor` want `COLORREF`.
//! We convert with `COLORREF(n)`.

use crate::edit_session::request_edit_session;
use crate::io_thread::{WM_CHEIME_ACTION, WM_CHEIME_SNAPSHOT, WM_CHEIME_STATUS};
use crate::tsf_interfaces::tsf_log;
use cheime_model::{CandidateSnapshot, PlatformAction};
use cheime_protocol::FrontendMessage;
use cheime_tip_core::layout::{ROW_PADDING_X, ROW_PADDING_Y, layout_snapshot};
use std::sync::Mutex;
use std::sync::mpsc::SyncSender;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontW,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject, EndPaint, FF_DONTCARE,
    FW_NORMAL, GetSysColor, HBRUSH, HDC, InvalidateRect, OPAQUE, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT, TextOutW, UpdateWindow,
};
use windows::Win32::UI::TextServices::{ITfComposition, ITfThreadMgr};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, HWND_TOPMOST,
    RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WINDOW_LONG_PTR_INDEX, WM_CREATE, WM_DESTROY, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_PAINT, WNDCLASS_STYLES, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const CANDIDATE_WINDOW_CLASS: &str = "CheIME_CandidateWindow";
const FONT_HEIGHT: i32 = 18;
const CHAR_WIDTH: i32 = 10;
const LINE_HEIGHT: i32 = 22;

type SnapshotBox = Mutex<Option<(CandidateSnapshot, Vec<RowRender>)>>;

struct RowRender {
    text: Vec<u16>,
    y: i32,
    highlighted: bool,
}

/// Context stored as GWLP_USERDATA on the candidate window.
/// Accessible from the window proc for both snapshot rendering and edit sessions.
pub struct WindowContext {
    snapshot: SnapshotBox,
    pub thread_mgr: ITfThreadMgr,
    pub client_id: u32,
    pub channel: SyncSender<FrontendMessage>,
    pub composition: Mutex<Option<ITfComposition>>,
}

pub struct CandidateWindow {
    hwnd: HWND,
    ctx_ptr: *const WindowContext,
}

impl CandidateWindow {
    /// Create a new candidate window.  `ctx` ownership transfers to the window's
    /// user data and lives until `destroy` is called.
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
                -1000,
                -1000,
                200,
                100,
                HWND(std::ptr::null_mut()),
                windows::Win32::UI::WindowsAndMessaging::HMENU(std::ptr::null_mut()),
                HINSTANCE(hinst.0),
                None,
            )
        }
        .map_err(|_| "CreateWindowExW failed")?;

        let ctx_ptr: *const WindowContext = Box::into_raw(ctx);
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0),
                ctx_ptr as isize,
            );
        }
        Ok(Self { hwnd, ctx_ptr })
    }

    /// Helper to build a WindowContext from available activation resources.
    pub fn new_context(
        thread_mgr: ITfThreadMgr,
        client_id: u32,
        channel: SyncSender<FrontendMessage>,
    ) -> Box<WindowContext> {
        Box::new(WindowContext {
            snapshot: Mutex::new(None),
            thread_mgr,
            client_id,
            channel,
            composition: Mutex::new(None),
        })
    }

    pub fn update(&self, snapshot: &CandidateSnapshot) {
        let rows = build_rows(snapshot, LINE_HEIGHT);
        if !self.ctx_ptr.is_null() {
            if let Ok(mut st) = unsafe { (&*self.ctx_ptr).snapshot.lock() } {
                *st = Some((snapshot.clone(), rows));
            }
        }
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }

    pub fn show_at(&self, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(self.hwnd);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
            }
        }
        // WM_DESTROY already freed ctx_ptr.  Prevent double-free if
        // Drop runs again (e.g. after move/panic).
        self.ctx_ptr = std::ptr::null();
    }
}

unsafe extern "system" fn candidate_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => LRESULT(0),
            WM_ERASEBKGND => {
                // Let DefWindowProc handle background erasing with hbrBackground
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.is_invalid() {
                    let ctx_ptr = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0))
                        as *const WindowContext;
                    if !ctx_ptr.is_null() {
                        if let Ok(state) = (&*ctx_ptr).snapshot.lock() {
                            if let Some((_, rows)) = state.as_ref() {
                                paint(hdc, rows);
                            }
                        }
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            WM_CHEIME_SNAPSHOT => {
                if lparam.0 != 0 {
                    let boxed: Box<CandidateSnapshot> =
                        Box::from_raw(lparam.0 as *mut CandidateSnapshot);
                    tsf_log(&format!(
                        "[CheIME] WM_SNAPSHOT preedit={} candidates={}",
                        boxed.preedit,
                        boxed.candidates.len()
                    ));
                    let rows = build_rows(&boxed, LINE_HEIGHT);
                    let ctx_ptr = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0))
                        as *const WindowContext;
                    if !ctx_ptr.is_null() {
                        if let Ok(mut st) = (&*ctx_ptr).snapshot.lock() {
                            *st = Some((*boxed, rows));
                        }
                    }
                    // Auto-position and show the window near the caret
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        100, // x — TODO: get actual caret position
                        200, // y — TODO: get actual caret position
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOSIZE,
                    );
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    let _ = InvalidateRect(hwnd, None, true);
                }
                LRESULT(0)
            }
            WM_CHEIME_ACTION => {
                if lparam.0 != 0 {
                    let action: Box<PlatformAction> =
                        Box::from_raw(lparam.0 as *mut PlatformAction);
                    tsf_log(&format!("[CheIME] WM_ACTION action={action:?}"));
                    let ctx_ptr = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0))
                        as *const WindowContext;
                    if !ctx_ptr.is_null() {
                        let ctx = &*ctx_ptr;
                        // Get focused context from thread manager
                        match ctx.thread_mgr.GetFocus() {
                            Ok(doc) => match doc.GetTop() {
                                Ok(context) => {
                                    tsf_log(
                                        "[CheIME] WM_ACTION: got context, requesting edit session",
                                    );
                                    request_edit_session(
                                        ctx.client_id,
                                        &context,
                                        *action,
                                        &ctx.channel as *const SyncSender<FrontendMessage>,
                                        &ctx.composition as *const Mutex<Option<ITfComposition>>,
                                    );
                                }
                                Err(e) => {
                                    tsf_log(&format!("[CheIME] WM_ACTION: GetTop failed: {e:?}"))
                                }
                            },
                            Err(e) => {
                                tsf_log(&format!("[CheIME] WM_ACTION: GetFocus failed: {e:?}"))
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CHEIME_STATUS => {
                if lparam.0 != 0 {
                    let status: Box<(bool, String)> =
                        Box::from_raw(lparam.0 as *mut (bool, String));
                    tsf_log(&format!(
                        "[CheIME] WM_STATUS connected={} detail={}",
                        status.0, status.1
                    ));
                    // Hide when engine disconnects; let snapshot handler re-show.
                    if !status.0 {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => LRESULT(0),
            WM_DESTROY => {
                // Free the stored WindowContext. Called synchronously from
                // DestroyWindow in CandidateWindow::Drop.
                let ctx_ptr = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0))
                    as *const WindowContext;
                if !ctx_ptr.is_null() {
                    drop(Box::from_raw(ctx_ptr as *mut WindowContext));
                }
                // Do NOT call PostQuitMessage — TSF owns the STA message pump.
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint(hdc: HDC, rows: &[RowRender]) {
    let fg = COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) });
    let hl_bg = COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHT) });
    let hl_fg = COLORREF(unsafe { GetSysColor(COLOR_HIGHLIGHTTEXT) });
    unsafe {
        // Use Microsoft YaHei for CJK text rendering; fall back to system default.
        let face: Vec<u16> = "Microsoft YaHei\0".encode_utf16().collect();
        let font = CreateFontW(
            FONT_HEIGHT,
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
        );
        let old = SelectObject(hdc, font);
        for row in rows {
            if row.highlighted {
                SetBkColor(hdc, hl_bg);
                SetTextColor(hdc, hl_fg);
                SetBkMode(hdc, OPAQUE);
            } else {
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, fg);
            }
            let _ = TextOutW(hdc, ROW_PADDING_X, row.y, &row.text);
        }
        if !old.is_invalid() {
            SelectObject(hdc, old);
        }
        let _ = DeleteObject(font);
    }
}

fn build_rows(snapshot: &CandidateSnapshot, line_height: i32) -> Vec<RowRender> {
    let layout = layout_snapshot(snapshot, line_height, CHAR_WIDTH);
    let mut rows = Vec::new();
    let mut y = ROW_PADDING_Y;
    if !layout.preedit.is_empty() {
        let t: Vec<u16> = layout
            .preedit
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        rows.push(RowRender {
            text: t,
            y,
            highlighted: false,
        });
        y += line_height;
    }
    for row in &layout.rows {
        if row.is_preedit {
            continue;
        }
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
        let t: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        rows.push(RowRender {
            text: t,
            y,
            highlighted: row.is_highlighted,
        });
        y += line_height;
    }
    rows
}
