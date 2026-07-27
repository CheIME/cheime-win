//! Candidate-window UI configuration.
//!
//! The shape follows Weasel's useful separation of style, layout and named
//! colour schemes, while deliberately using CSS-like `#RRGGBB` colours.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default)]
    pub style: StyleConfig,
    #[serde(default = "default_schemes")]
    pub preset_color_schemes: BTreeMap<String, ColorScheme>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleConfig {
    #[serde(default = "light_scheme")]
    pub color_scheme: String,
    #[serde(default = "dark_scheme")]
    pub color_scheme_dark: String,
    #[serde(default = "font_face")]
    pub font_face: String,
    #[serde(default = "font_face")]
    pub label_font_face: String,
    #[serde(default = "font_face")]
    pub comment_font_face: String,
    #[serde(default = "font_point")]
    pub font_point: i32,
    #[serde(default = "label_font_point")]
    pub label_font_point: i32,
    #[serde(default = "comment_font_point")]
    pub comment_font_point: i32,
    #[serde(default = "page_size")]
    pub page_size: usize,
    #[serde(default = "bool_true")]
    pub show_labels: bool,
    #[serde(default = "bool_true")]
    pub show_candidate_annotations: bool,
    #[serde(default = "label_format")]
    pub label_format: String,
    #[serde(default)]
    pub inline_preedit: bool,
    #[serde(default)]
    pub preedit_type: PreeditType,
    #[serde(default)]
    pub preedit_underline_style: UnderlineStyle,
    #[serde(default)]
    pub preedit_underline_bold: bool,
    #[serde(default)]
    pub preedit_background_enabled: bool,
    #[serde(default)]
    pub antialias_mode: AntialiasMode,
    #[serde(default)]
    pub mark_text: String,
    #[serde(default)]
    pub hover_type: HoverType,
    #[serde(default = "bool_true")]
    pub paging_on_scroll: bool,
    #[serde(default)]
    pub candidate_abbreviate_length: usize,
    #[serde(default)]
    pub layout: LayoutConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutConfig {
    #[serde(default)]
    pub r#type: LayoutType,
    #[serde(default)]
    pub text_vertical_align: TextVerticalAlign,
    #[serde(default = "min_width")]
    pub min_width: i32,
    #[serde(default)]
    pub max_width: i32,
    #[serde(default = "margin_x")]
    pub margin_x: i32,
    #[serde(default = "margin_y")]
    pub margin_y: i32,
    #[serde(default = "spacing")]
    pub spacing: i32,
    #[serde(default = "candidate_spacing")]
    pub candidate_spacing: i32,
    #[serde(default = "hilite_spacing")]
    pub hilite_spacing: i32,
    #[serde(default = "hilite_padding_x")]
    pub hilite_padding_x: i32,
    #[serde(default = "hilite_padding_y")]
    pub hilite_padding_y: i32,
    #[serde(default = "corner_radius")]
    pub corner_radius: i32,
    #[serde(default = "hilited_corner_radius")]
    pub hilited_corner_radius: i32,
    #[serde(default = "border_width")]
    pub border_width: i32,
    #[serde(default = "border_width")]
    pub hilited_border_width: i32,
    #[serde(default = "mark_width")]
    pub mark_width: i32,
    #[serde(default = "mark_height")]
    pub mark_height: i32,
    #[serde(default = "mark_gap")]
    pub mark_gap: i32,
    #[serde(default = "bool_true")]
    pub shadow_enabled: bool,
    #[serde(default)]
    pub shadow_radius: i32,
    #[serde(default = "shadow_opacity")]
    pub shadow_opacity: i32,
    #[serde(default)]
    pub shadow_offset_x: i32,
    #[serde(default)]
    pub shadow_offset_y: i32,
    #[serde(default)]
    pub caret_offset_x: i32,
    #[serde(default = "caret_offset_y")]
    pub caret_offset_y: i32,
    #[serde(default = "screen_edge_margin")]
    pub screen_edge_margin: i32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutType {
    Horizontal,
    #[default]
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextVerticalAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreeditType {
    #[default]
    Composition,
    Preview,
    PreviewAll,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnderlineStyle {
    None,
    Solid,
    Dot,
    #[default]
    Dash,
    Squiggle,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AntialiasMode {
    #[default]
    Default,
    ForceDword,
    Cleartype,
    Grayscale,
    Aliased,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoverType {
    #[default]
    None,
    Hilite,
    SemiHilite,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColorScheme {
    #[serde(default)]
    pub name: String,
    #[serde(default = "background")]
    pub back_color: String,
    #[serde(default = "border")]
    pub border_color: String,
    #[serde(default = "text")]
    pub text_color: String,
    #[serde(default = "candidate")]
    pub candidate_text_color: String,
    #[serde(default = "comment")]
    pub comment_text_color: String,
    #[serde(default = "label")]
    pub label_color: String,
    #[serde(default = "highlight_text")]
    pub hilited_candidate_text_color: String,
    #[serde(default = "highlight")]
    pub hilited_candidate_back_color: String,
    #[serde(default = "border")]
    pub hilited_candidate_border_color: String,
    #[serde(default = "highlight")]
    pub hilited_mark_color: String,
    #[serde(default = "shadow")]
    pub shadow_color: String,
    #[serde(default = "text")]
    pub preedit_text_color: String,
    #[serde(default = "background")]
    pub preedit_back_color: String,
    #[serde(default = "highlight")]
    pub preedit_underline_color: String,
}

impl UiConfig {
    pub fn active_scheme(&self, dark: bool) -> &ColorScheme {
        let requested = if dark {
            &self.style.color_scheme_dark
        } else {
            &self.style.color_scheme
        };
        self.preset_color_schemes
            .get(requested)
            .or_else(|| self.preset_color_schemes.get(&self.style.color_scheme))
            .or_else(|| self.preset_color_schemes.values().next())
            .expect("default UI always contains a colour scheme")
    }

    pub fn validate(&self) -> Result<(), String> {
        for selected in [&self.style.color_scheme, &self.style.color_scheme_dark] {
            if !self.preset_color_schemes.contains_key(selected) {
                return Err(format!("unknown color scheme '{selected}'"));
            }
        }
        for (name, scheme) in &self.preset_color_schemes {
            for (field, value) in scheme.colors() {
                if !is_rrggbb(value) {
                    return Err(format!(
                        "preset_color_schemes.{name}.{field} must be #RRGGBB"
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ColorScheme {
    fn colors(&self) -> [(&'static str, &str); 14] {
        [
            ("back_color", &self.back_color),
            ("border_color", &self.border_color),
            ("text_color", &self.text_color),
            ("candidate_text_color", &self.candidate_text_color),
            ("comment_text_color", &self.comment_text_color),
            ("label_color", &self.label_color),
            (
                "hilited_candidate_text_color",
                &self.hilited_candidate_text_color,
            ),
            (
                "hilited_candidate_back_color",
                &self.hilited_candidate_back_color,
            ),
            (
                "hilited_candidate_border_color",
                &self.hilited_candidate_border_color,
            ),
            ("hilited_mark_color", &self.hilited_mark_color),
            ("shadow_color", &self.shadow_color),
            ("preedit_text_color", &self.preedit_text_color),
            ("preedit_back_color", &self.preedit_back_color),
            ("preedit_underline_color", &self.preedit_underline_color),
        ]
    }
}

pub fn load_ui_config(path: &std::path::Path) -> Result<UiConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let config: UiConfig =
        serde_yaml::from_str(&raw).map_err(|error| format!("parse {}: {error}", path.display()))?;
    config.validate()?;
    Ok(config)
}

fn is_rrggbb(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn default_schemes() -> BTreeMap<String, ColorScheme> {
    BTreeMap::from([
        ("cheime_dark".into(), dark_colors()),
        ("cheime_light".into(), ColorScheme::default()),
    ])
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            style: StyleConfig::default(),
            preset_color_schemes: default_schemes(),
        }
    }
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            color_scheme: light_scheme(),
            color_scheme_dark: dark_scheme(),
            font_face: font_face(),
            label_font_face: font_face(),
            comment_font_face: font_face(),
            font_point: font_point(),
            label_font_point: label_font_point(),
            comment_font_point: comment_font_point(),
            page_size: page_size(),
            show_labels: true,
            show_candidate_annotations: true,
            label_format: label_format(),
            inline_preedit: false,
            preedit_type: PreeditType::Composition,
            preedit_underline_style: UnderlineStyle::Dash,
            preedit_underline_bold: false,
            preedit_background_enabled: false,
            antialias_mode: AntialiasMode::Default,
            mark_text: String::new(),
            hover_type: HoverType::None,
            paging_on_scroll: true,
            candidate_abbreviate_length: 0,
            layout: LayoutConfig::default(),
        }
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            r#type: LayoutType::Vertical,
            text_vertical_align: TextVerticalAlign::Center,
            min_width: min_width(),
            max_width: 0,
            margin_x: margin_x(),
            margin_y: margin_y(),
            spacing: spacing(),
            candidate_spacing: candidate_spacing(),
            hilite_spacing: hilite_spacing(),
            hilite_padding_x: hilite_padding_x(),
            hilite_padding_y: hilite_padding_y(),
            corner_radius: corner_radius(),
            hilited_corner_radius: hilited_corner_radius(),
            border_width: border_width(),
            hilited_border_width: border_width(),
            mark_width: mark_width(),
            mark_height: mark_height(),
            mark_gap: mark_gap(),
            shadow_enabled: true,
            shadow_radius: 0,
            shadow_opacity: shadow_opacity(),
            shadow_offset_x: 0,
            shadow_offset_y: 0,
            caret_offset_x: 0,
            caret_offset_y: caret_offset_y(),
            screen_edge_margin: screen_edge_margin(),
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            name: "澈 · 明".into(),
            back_color: background(),
            border_color: border(),
            text_color: text(),
            candidate_text_color: candidate(),
            comment_text_color: comment(),
            label_color: label(),
            hilited_candidate_text_color: highlight_text(),
            hilited_candidate_back_color: highlight(),
            hilited_candidate_border_color: border(),
            hilited_mark_color: highlight(),
            shadow_color: shadow(),
            preedit_text_color: text(),
            preedit_back_color: background(),
            preedit_underline_color: highlight(),
        }
    }
}

fn dark_colors() -> ColorScheme {
    ColorScheme {
        name: "澈 · 暗".into(),
        back_color: "#202124".into(),
        border_color: "#34363A".into(),
        text_color: "#E8EAED".into(),
        candidate_text_color: "#E8EAED".into(),
        comment_text_color: "#9AA0A6".into(),
        label_color: "#9AA0A6".into(),
        hilited_candidate_text_color: "#FFFFFF".into(),
        hilited_candidate_back_color: "#3F6AE0".into(),
        hilited_candidate_border_color: "#5F6368".into(),
        hilited_mark_color: "#8AB4F8".into(),
        shadow_color: "#111318".into(),
        preedit_text_color: "#E8EAED".into(),
        preedit_back_color: "#202124".into(),
        preedit_underline_color: "#8AB4F8".into(),
    }
}

fn light_scheme() -> String {
    "cheime_light".into()
}
fn dark_scheme() -> String {
    "cheime_dark".into()
}
fn font_face() -> String {
    "Microsoft YaHei UI".into()
}
fn font_point() -> i32 {
    18
}
fn label_font_point() -> i32 {
    13
}
fn comment_font_point() -> i32 {
    13
}
fn page_size() -> usize {
    5
}
fn label_format() -> String {
    "%s".into()
}
fn min_width() -> i32 {
    280
}
fn margin_x() -> i32 {
    10
}
fn margin_y() -> i32 {
    8
}
fn spacing() -> i32 {
    6
}
fn candidate_spacing() -> i32 {
    4
}
fn hilite_spacing() -> i32 {
    8
}
fn hilite_padding_x() -> i32 {
    10
}
fn hilite_padding_y() -> i32 {
    5
}
fn corner_radius() -> i32 {
    10
}
fn hilited_corner_radius() -> i32 {
    7
}
fn border_width() -> i32 {
    1
}
fn mark_width() -> i32 {
    3
}
fn mark_height() -> i32 {
    18
}
fn mark_gap() -> i32 {
    6
}
fn caret_offset_y() -> i32 {
    8
}
fn shadow_opacity() -> i32 {
    28
}
fn screen_edge_margin() -> i32 {
    12
}
fn bool_true() -> bool {
    true
}
fn background() -> String {
    "#FFFFFF".into()
}
fn border() -> String {
    "#E1E4E8".into()
}
fn text() -> String {
    "#1F2328".into()
}
fn candidate() -> String {
    "#1F2328".into()
}
fn comment() -> String {
    "#6E7781".into()
}
fn label() -> String {
    "#8C959F".into()
}
fn highlight_text() -> String {
    "#FFFFFF".into()
}
fn highlight() -> String {
    "#316DCA".into()
}
fn shadow() -> String {
    "#8C959F".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip_and_schemes() {
        let yaml = serde_yaml::to_string(&UiConfig::default()).unwrap();
        let parsed: UiConfig = serde_yaml::from_str(&yaml).unwrap();
        parsed.validate().unwrap();
        assert_eq!(parsed.style.layout.r#type, LayoutType::Vertical);
        assert_eq!(parsed.active_scheme(false).back_color, "#FFFFFF");
        assert_eq!(parsed.active_scheme(true).back_color, "#202124");
    }

    #[test]
    fn rejects_weasel_bgr_and_short_hex_colors() {
        let mut config = UiConfig::default();
        config
            .preset_color_schemes
            .get_mut("cheime_light")
            .unwrap()
            .back_color = "0xFFFFFF".into();
        assert!(config.validate().unwrap_err().contains("#RRGGBB"));
        config
            .preset_color_schemes
            .get_mut("cheime_light")
            .unwrap()
            .back_color = "#FFF".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let yaml = "style:\n  bogus: true\n";
        assert!(serde_yaml::from_str::<UiConfig>(yaml).is_err());
    }

    #[test]
    fn missing_shadow_fields_use_rendering_defaults() {
        let yaml = "style:\n  layout:\n    shadow_radius: 12\n";
        let parsed: UiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.style.layout.shadow_radius, 12);
        assert!(parsed.style.layout.shadow_enabled);
        assert_eq!(parsed.style.layout.shadow_opacity, 28);
        assert_eq!(parsed.style.layout.shadow_offset_x, 0);
        assert_eq!(parsed.style.layout.shadow_offset_y, 0);
    }
}
