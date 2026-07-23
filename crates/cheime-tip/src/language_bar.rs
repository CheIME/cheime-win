//! TSF language-bar mode indicator shown in the Windows notification area.

use crate::dll_exports::CLSID_CHEIME_TIP;
use crate::key_handler::InputMode;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::rc::Rc;
use windows::Win32::Foundation::{BOOL, E_NOTIMPL, E_POINTER, HMODULE, POINT, RECT};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::TextServices::{
    GUID_LBI_INPUTMODE, ITfLangBarItem, ITfLangBarItem_Impl, ITfLangBarItemButton,
    ITfLangBarItemButton_Impl, ITfLangBarItemMgr, ITfLangBarItemSink, ITfMenu, ITfSource,
    ITfSource_Impl, TF_LANGBARITEMINFO, TF_LBI_ICON, TF_LBI_STATUS, TF_LBI_STYLE_BTN_BUTTON,
    TF_LBI_STYLE_SHOWNINTRAY, TF_LBI_TEXT, TF_LBI_TOOLTIP, TfLBIClick,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadImageW,
};
use windows::core::{BSTR, Interface, PCWSTR};

#[windows::core::implement(ITfLangBarItemButton, ITfSource)]
struct LanguageBarButton {
    mode: Rc<Cell<InputMode>>,
    visible: Cell<bool>,
    sink: Rc<RefCell<Option<ITfLangBarItemSink>>>,
}

impl LanguageBarButton {
    fn new(mode: Rc<Cell<InputMode>>, sink: Rc<RefCell<Option<ITfLangBarItemSink>>>) -> Self {
        Self {
            mode,
            visible: Cell::new(true),
            sink,
        }
    }

    fn label(&self) -> &'static str {
        match self.mode.get() {
            InputMode::Chinese => "中",
            InputMode::Direct => "A",
        }
    }

    fn icon_name(&self) -> &'static str {
        match (self.mode.get(), system_uses_dark_theme()) {
            (InputMode::Chinese, false) => "zh-black.ico",
            (InputMode::Chinese, true) => "zh-white.ico",
            (InputMode::Direct, false) => "en-black.ico",
            (InputMode::Direct, true) => "en-white.ico",
        }
    }
}

impl ITfSource_Impl for LanguageBarButton_Impl {
    fn AdviseSink(
        &self,
        interface_id: *const windows::core::GUID,
        unknown: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<u32> {
        if interface_id.is_null() || unknown.is_none() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }
        if unsafe { *interface_id } != ITfLangBarItemSink::IID {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        }
        let mut slot = self.sink.borrow_mut();
        if slot.is_some() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        *slot = Some(unknown.expect("checked above").cast()?);
        Ok(1)
    }

    fn UnadviseSink(&self, cookie: u32) -> windows::core::Result<()> {
        if cookie != 1 || self.sink.borrow().is_none() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        self.sink.borrow_mut().take();
        Ok(())
    }
}

impl ITfLangBarItem_Impl for LanguageBarButton_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> windows::core::Result<()> {
        if info.is_null() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }
        let mut description = [0u16; 32];
        for (slot, value) in description
            .iter_mut()
            .zip("CheIME 中英文模式".encode_utf16())
        {
            *slot = value;
        }
        unsafe {
            *info = TF_LANGBARITEMINFO {
                clsidService: CLSID_CHEIME_TIP,
                guidItem: GUID_LBI_INPUTMODE,
                dwStyle: TF_LBI_STYLE_SHOWNINTRAY | TF_LBI_STYLE_BTN_BUTTON,
                ulSort: 0,
                szDescription: description,
            };
        }
        Ok(())
    }

    fn GetStatus(&self) -> windows::core::Result<u32> {
        Ok(if self.visible.get() { 0 } else { 1 })
    }

    fn Show(&self, show: BOOL) -> windows::core::Result<()> {
        self.visible.set(show.as_bool());
        Ok(())
    }

    fn GetTooltipString(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from(match self.mode.get() {
            InputMode::Chinese => "CheIME 中文模式",
            InputMode::Direct => "CheIME 英文模式",
        }))
    }
}

impl ITfLangBarItemButton_Impl for LanguageBarButton_Impl {
    fn OnClick(
        &self,
        click: TfLBIClick,
        _point: &POINT,
        _area: *const RECT,
    ) -> windows::core::Result<()> {
        let _ = click;
        Ok(())
    }

    fn InitMenu(&self, _menu: Option<&ITfMenu>) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }

    fn OnMenuSelect(&self, _id: u32) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }

    fn GetIcon(&self) -> windows::core::Result<HICON> {
        let path = module_sibling(self.icon_name())?;
        let wide = wide(&path.to_string_lossy());
        let handle = unsafe {
            LoadImageW(
                None,
                PCWSTR(wide.as_ptr()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
        }?;
        Ok(HICON(handle.0))
    }

    fn GetText(&self) -> windows::core::Result<BSTR> {
        Ok(BSTR::from(self.label()))
    }
}

pub struct LanguageBarRegistration {
    manager: ITfLangBarItemMgr,
    item: ITfLangBarItem,
    sink: Rc<RefCell<Option<ITfLangBarItemSink>>>,
}

impl LanguageBarRegistration {
    pub fn attach(
        thread_mgr: &windows::Win32::UI::TextServices::ITfThreadMgr,
        mode: Rc<Cell<InputMode>>,
    ) -> windows::core::Result<Self> {
        let manager: ITfLangBarItemMgr = thread_mgr.cast()?;
        let sink = Rc::new(RefCell::new(None));
        let button: ITfLangBarItemButton = LanguageBarButton::new(mode, sink.clone()).into();
        let item: ITfLangBarItem = button.cast()?;
        unsafe { manager.AddItem(&item)? };
        Ok(Self {
            manager,
            item,
            sink,
        })
    }

    pub fn refresh(&self) {
        if let Some(sink) = self.sink.borrow().as_ref() {
            unsafe {
                let _ = sink.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP | TF_LBI_STATUS);
            }
        }
    }
}

impl Drop for LanguageBarRegistration {
    fn drop(&mut self) {
        unsafe {
            let _ = self.manager.RemoveItem(&self.item);
        }
    }
}

fn system_uses_dark_theme() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast::<c_void>()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

fn module_sibling(name: &str) -> windows::core::Result<PathBuf> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_sibling as *const () as *const u16),
            &mut module,
        )?;
    }
    let mut buffer = vec![0u16; 260];
    loop {
        let length = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
        if length == 0 {
            return Err(windows::core::Error::from_win32());
        }
        if length < buffer.len() - 1 {
            let module_path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
            return Ok(module_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .join(name));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
