use cheime_tip_core::ui_config::UnderlineStyle;
use std::cell::Cell;
use windows::Win32::Foundation::{BOOL, COLORREF, E_POINTER, S_FALSE};
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfDisplayAttributeInfo,
    ITfDisplayAttributeInfo_Impl, TF_ATTR_INPUT, TF_CT_COLORREF, TF_CT_NONE, TF_DA_COLOR,
    TF_DA_COLOR_0, TF_DISPLAYATTRIBUTE, TF_LS_DASH, TF_LS_DOT, TF_LS_NONE, TF_LS_SOLID,
    TF_LS_SQUIGGLE,
};
use windows::core::{BSTR, GUID, Result, implement};

pub const GUID_CHEIME_PREEDIT: GUID = GUID::from_u128(0x7f3df1b4_3251_4da3_9f24_2f4aaee8c619);

#[implement(ITfDisplayAttributeInfo)]
pub struct PreeditDisplayAttribute;

#[implement(IEnumTfDisplayAttributeInfo)]
struct DisplayAttributeEnumerator {
    yielded: Cell<bool>,
}

impl PreeditDisplayAttribute {
    fn new() -> Self {
        crate::exports::increment_object_count();
        Self
    }
}

impl Drop for PreeditDisplayAttribute {
    fn drop(&mut self) {
        crate::exports::decrement_object_count();
    }
}

impl DisplayAttributeEnumerator {
    fn new(yielded: bool) -> Self {
        crate::exports::increment_object_count();
        Self {
            yielded: Cell::new(yielded),
        }
    }
}

impl Drop for DisplayAttributeEnumerator {
    fn drop(&mut self) {
        crate::exports::decrement_object_count();
    }
}

#[allow(non_snake_case)]
impl ITfDisplayAttributeInfo_Impl for PreeditDisplayAttribute_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID_CHEIME_PREEDIT)
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from("CheIME inline preedit"))
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetAttributeInfo(&self, output: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        if output.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_POINTER,
            ));
        }
        unsafe {
            *output = current_attribute();
        }
        Ok(())
    }

    fn SetAttributeInfo(&self, _: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

pub fn create_info() -> ITfDisplayAttributeInfo {
    PreeditDisplayAttribute::new().into()
}

#[allow(non_snake_case)]
impl IEnumTfDisplayAttributeInfo_Impl for DisplayAttributeEnumerator_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(DisplayAttributeEnumerator::new(self.yielded.get()).into())
    }

    fn Next(
        &self,
        count: u32,
        output: *mut Option<ITfDisplayAttributeInfo>,
        fetched: *mut u32,
    ) -> Result<()> {
        if count > 0 && output.is_null() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }
        if count != 1 && fetched.is_null() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }

        let produced = u32::from(count > 0 && !self.yielded.replace(true));
        if produced == 1 {
            unsafe {
                output.write(Some(create_info()));
            }
        }
        if !fetched.is_null() {
            unsafe {
                fetched.write(produced);
            }
        }
        if produced == count {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(S_FALSE))
        }
    }

    fn Reset(&self) -> Result<()> {
        self.yielded.set(false);
        Ok(())
    }

    fn Skip(&self, count: u32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let available = u32::from(!self.yielded.replace(true));
        if count <= available {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(S_FALSE))
        }
    }
}

pub fn create_enumerator() -> IEnumTfDisplayAttributeInfo {
    DisplayAttributeEnumerator::new(false).into()
}

pub fn current_attribute() -> TF_DISPLAYATTRIBUTE {
    let config = crate::ui_settings::load_config();
    let scheme = config.active_scheme(crate::ui_settings::system_uses_dark_theme());
    let line_style = match config.style.preedit_underline_style {
        UnderlineStyle::None => TF_LS_NONE,
        UnderlineStyle::Solid => TF_LS_SOLID,
        UnderlineStyle::Dot => TF_LS_DOT,
        UnderlineStyle::Dash => TF_LS_DASH,
        UnderlineStyle::Squiggle => TF_LS_SQUIGGLE,
    };
    TF_DISPLAYATTRIBUTE {
        crText: color(&scheme.preedit_text_color),
        crBk: if config.style.preedit_background_enabled {
            color(&scheme.preedit_back_color)
        } else {
            TF_DA_COLOR {
                r#type: TF_CT_NONE,
                ..Default::default()
            }
        },
        lsStyle: line_style,
        fBoldLine: BOOL(config.style.preedit_underline_bold as i32),
        crLine: color(&scheme.preedit_underline_color),
        bAttr: TF_ATTR_INPUT,
    }
}

fn color(value: &str) -> TF_DA_COLOR {
    let hex = value.strip_prefix('#').unwrap_or_default();
    let rgb = u32::from_str_radix(hex, 16).unwrap_or_default();
    let colorref = COLORREF(((rgb >> 16) & 0xff) | (rgb & 0x00ff00) | ((rgb & 0xff) << 16));
    TF_DA_COLOR {
        r#type: TF_CT_COLORREF,
        Anonymous: TF_DA_COLOR_0 { cr: colorref },
    }
}
