//! GDI candidate window — minimal rendering with window proc.
//! NOTE: candidate window rendering uses raw GDI calls via windows crate 0.58.
//! `GetSysColor` returns `u32` but `SetTextColor`/`SetBkColor` want `COLORREF`.
//! We convert with `COLORREF(n)`.

use cheime_model::CandidateSnapshot;
use cheime_tip_core::layout::{layout_snapshot, ROW_PADDING_X, ROW_PADDING_Y};
use std::sync::Mutex;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, DeleteObject, EndPaint, PAINTSTRUCT, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, TextOutW, UpdateWindow,
    DEFAULT_CHARSET, FW_NORMAL, FF_DONTCARE, OUT_DEFAULT_PRECIS, DEFAULT_QUALITY,
    TRANSPARENT, OPAQUE, GetSysColor, InvalidateRect, HDC,
    COLOR_WINDOW, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOWTEXT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostQuitMessage, RegisterClassW,
    ShowWindow, WNDCLASSW, WNDCLASS_STYLES,
    WM_CREATE, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    SW_HIDE, SW_SHOWNOACTIVATE,
    SetWindowPos, SetWindowLongPtrW, GetWindowLongPtrW,
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE, GWLP_USERDATA,
    WINDOW_LONG_PTR_INDEX,
};
use crate::io_thread::WM_CHEIME_SNAPSHOT;

const CANDIDATE_WINDOW_CLASS: &str = "CheIME_CandidateWindow";
const FONT_HEIGHT: i32 = 18;
const CHAR_WIDTH: i32 = 10;
const LINE_HEIGHT: i32 = 22;

type StateBox = Mutex<Option<(CandidateSnapshot, Vec<RowRender>)>>;

struct RowRender { text: Vec<u16>, y: i32, highlighted: bool }

pub struct CandidateWindow { hwnd: HWND, state_ptr: *const StateBox }

impl CandidateWindow {
    pub fn create() -> Result<Self, String> {
        let hinst = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let class_wide: Vec<u16> = CANDIDATE_WINDOW_CLASS.encode_utf16().chain(std::iter::once(0)).collect();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(candidate_window_proc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: windows::core::PCWSTR::from_raw(class_wide.as_ptr()),
            style: WNDCLASS_STYLES(0), cbClsExtra: 0, cbWndExtra: 0,
            hIcon: Default::default(), hCursor: Default::default(), hbrBackground: Default::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
        };
        unsafe { RegisterClassW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                windows::core::PCWSTR::from_raw(class_wide.as_ptr()),
                windows::core::w!("CheIME Candidate"),
                WS_POPUP, -1000, -1000, 200, 100,
                HWND(std::ptr::null_mut()), windows::Win32::UI::WindowsAndMessaging::HMENU(std::ptr::null_mut()),
                HINSTANCE(hinst.0), None,
            )
        }.map_err(|_| "CreateWindowExW failed")?;

        let state: Box<StateBox> = Box::new(Mutex::new(None));
        let state_ptr: *const StateBox = Box::into_raw(state);
        unsafe { SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0), state_ptr as isize); }
        Ok(Self { hwnd, state_ptr })
    }

    pub fn update(&self, snapshot: &CandidateSnapshot) {
        let rows = build_rows(snapshot, LINE_HEIGHT);
        if !self.state_ptr.is_null() {
            if let Ok(mut st) = unsafe { (&*self.state_ptr).lock() } {
                *st = Some((snapshot.clone(), rows));
            }
        }
        unsafe { let _ = InvalidateRect(self.hwnd, None, false); }
    }

    pub fn show_at(&self, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(self.hwnd);
        }
    }

    pub fn hide(&self) { unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); } }
    pub fn hwnd(&self) -> HWND { self.hwnd }
}

unsafe extern "system" fn candidate_window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => LRESULT(0),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                let sp = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0)) as *const StateBox;
                if !sp.is_null() {
                    if let Ok(state) = unsafe { (&*sp).lock() } {
                        if let Some((_, rows)) = state.as_ref() { unsafe { paint(hdc, rows); } }
                    }
                }
                EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_CHEIME_SNAPSHOT => {
            if lparam.0 != 0 {
                let boxed: Box<CandidateSnapshot> = Box::from_raw(lparam.0 as *mut CandidateSnapshot);
                let rows = build_rows(&boxed, LINE_HEIGHT);
                let sp = GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(GWLP_USERDATA.0)) as *const StateBox;
                if !sp.is_null() {
                    if let Ok(mut st) = unsafe { (&*sp).lock() } { *st = Some((*boxed, rows)); }
                }
                let _ = InvalidateRect(hwnd, None, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => { LRESULT(0) }
        WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hdc: HDC, rows: &[RowRender]) {
    let bg = COLORREF(GetSysColor(COLOR_WINDOW));
    let fg = COLORREF(GetSysColor(COLOR_WINDOWTEXT));
    let hl_bg = COLORREF(GetSysColor(COLOR_HIGHLIGHT));
    let hl_fg = COLORREF(GetSysColor(COLOR_HIGHLIGHTTEXT));
    unsafe {
        let font = CreateFontW(FONT_HEIGHT, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, 0,
            DEFAULT_QUALITY.0 as u32, FF_DONTCARE.0 as u32 | DEFAULT_CHARSET.0 as u32, None);
        let old = SelectObject(hdc, font);
        for row in rows {
            if row.highlighted {
                SetBkColor(hdc, hl_bg); SetTextColor(hdc, hl_fg); SetBkMode(hdc, OPAQUE);
            } else {
                SetBkMode(hdc, TRANSPARENT); SetTextColor(hdc, fg);
            }
            TextOutW(hdc, ROW_PADDING_X, row.y, &row.text);
        }
        if !old.is_invalid() { SelectObject(hdc, old); }
        let _ = DeleteObject(font);
    }
}

fn build_rows(snapshot: &CandidateSnapshot, line_height: i32) -> Vec<RowRender> {
    let layout = layout_snapshot(snapshot, line_height, CHAR_WIDTH);
    let mut rows = Vec::new();
    let mut y = ROW_PADDING_Y;
    if !layout.preedit.is_empty() {
        let t: Vec<u16> = layout.preedit.encode_utf16().chain(std::iter::once(0)).collect();
        rows.push(RowRender { text: t, y, highlighted: false });
        y += line_height;
    }
    for row in &layout.rows {
        if row.is_preedit { continue; }
        let mut s = String::new();
        use std::fmt::Write;
        if let Some(idx) = row.index {
            let _ = write!(s, "{}. {}", idx, row.text);
            if let Some(ref ann) = row.annotation { let _ = write!(s, " {}", ann); }
        } else { s = row.text.clone(); }
        let t: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        rows.push(RowRender { text: t, y, highlighted: row.is_highlighted });
        y += line_height;
    }
    rows
}
