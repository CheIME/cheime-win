use cheime_tip_core::ui_config::UnderlineStyle;
use windows::Win32::Foundation::{BOOL, COLORREF};
use windows::Win32::UI::TextServices::{
    ITfDisplayAttributeInfo, ITfDisplayAttributeInfo_Impl, TF_ATTR_INPUT, TF_CT_COLORREF,
    TF_CT_NONE, TF_DA_COLOR, TF_DA_COLOR_0, TF_DISPLAYATTRIBUTE, TF_LS_DASH, TF_LS_DOT, TF_LS_NONE,
    TF_LS_SOLID, TF_LS_SQUIGGLE,
};
use windows::core::{BSTR, GUID, Result, implement};

pub const GUID_CHEIME_PREEDIT: GUID = GUID::from_u128(0x7f3df1b4_3251_4da3_9f24_2f4aaee8c619);

#[implement(ITfDisplayAttributeInfo)]
pub struct PreeditDisplayAttribute;

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
    PreeditDisplayAttribute.into()
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
