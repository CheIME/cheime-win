//! UI configuration schema for CheIME candidate window.
//!
//! Lives as `ui.yaml` in the config directory, completely decoupled from
//! engine schema/profiles. Supports Rime-style extends chains and deep merge.
//!
//! DRAFT §15: UI consumes immutable snapshots only; no engine state access.

use serde::{Deserialize, Serialize};

/// Top-level UI config structure.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,

    #[serde(default)]
    pub window: WindowConfig,

    #[serde(default)]
    pub candidate: CandidateConfig,

    #[serde(default)]
    pub selection_box: SelectionBoxConfig,

    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowConfig {
    #[serde(default)]
    pub material: Material,

    #[serde(default = "d1_0")]
    pub opacity: f32,

    #[serde(default = "d_pad")]
    pub padding: [i32; 2],

    #[serde(default)]
    pub caret_offset_x: i32,

    #[serde(default)]
    pub caret_offset_y: i32,

    #[serde(default = "d200")]
    pub min_width: i32,

    /// Fixed window height in pixels. Zero means content-driven.
    #[serde(default)]
    pub height: i32,

    /// Corner radius in pixels, clamped to half the rendered height.
    #[serde(default = "d8")]
    pub corner_radius: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    #[default]
    Opaque,
    Acrylic,
    Mica,
    Transparent,
}

fn d1_0() -> f32 {
    1.0
}
fn d_pad() -> [i32; 2] {
    [8, 4]
}
fn d200() -> i32 {
    200
}
fn d8() -> i32 {
    8
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateConfig {
    #[serde(default = "d18")]
    pub font_size: i32,

    #[serde(default)]
    pub char_width: Option<i32>,

    #[serde(default = "d22")]
    pub line_height: i32,

    #[serde(default = "d10")]
    pub row_padding_x: i32,

    #[serde(default = "d2")]
    pub row_padding_y: i32,

    #[serde(default = "d10_usize")]
    pub page_size: usize,

    #[serde(default = "d14")]
    pub label_size: i32,

    #[serde(default)]
    pub orientation: CandidateOrientation,

    #[serde(default = "bool_true")]
    pub show_labels: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateOrientation {
    Horizontal,
    #[default]
    Vertical,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionBoxConfig {
    #[serde(default = "selection_outline")]
    pub outline_color: String,

    /// None inherits the candidate window's outer corner radius.
    #[serde(default)]
    pub corner_radius: Option<i32>,

    /// Relative size of the candidate item, clamped to 0.0..=1.0.
    #[serde(default = "d1_0")]
    pub relative_size: f32,
}

fn selection_outline() -> String {
    String::from("#0078d4")
}

fn d18() -> i32 {
    18
}
fn d22() -> i32 {
    22
}
fn d10() -> i32 {
    10
}
fn d2() -> i32 {
    2
}
fn d14() -> i32 {
    14
}
fn d10_usize() -> usize {
    10
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeConfig {
    #[serde(default)]
    pub colors: Colors,

    #[serde(default = "bool_true")]
    pub use_system_accent: bool,
}

fn bool_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    #[serde(default = "bg")]
    pub background: String,

    #[serde(default = "fg")]
    pub foreground: String,

    #[serde(default = "cand")]
    pub candidate_text: String,

    #[serde(default = "sel_fg")]
    pub selected_text: String,

    #[serde(default = "sel_bg")]
    pub selected_background: String,

    #[serde(default = "cand")]
    pub label_color: String,

    #[serde(default = "comment")]
    pub comment_color: String,
}

fn bg() -> String {
    String::from("#ffffff")
}
fn fg() -> String {
    String::from("#000000")
}
fn cand() -> String {
    String::from("#333333")
}
fn sel_fg() -> String {
    String::from("#ffffff")
}
fn sel_bg() -> String {
    String::from("#0078d4")
}
fn comment() -> String {
    String::from("#888888")
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: bg(),
            foreground: fg(),
            candidate_text: cand(),
            selected_text: sel_fg(),
            selected_background: sel_bg(),
            label_color: cand(),
            comment_color: comment(),
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            colors: Colors::default(),
            use_system_accent: true,
        }
    }
}

impl Default for SelectionBoxConfig {
    fn default() -> Self {
        Self {
            outline_color: selection_outline(),
            corner_radius: None,
            relative_size: 1.0,
        }
    }
}

impl Default for CandidateConfig {
    fn default() -> Self {
        Self {
            font_size: 18,
            char_width: None,
            line_height: 22,
            row_padding_x: 10,
            row_padding_y: 2,
            page_size: 10,
            label_size: 14,
            orientation: CandidateOrientation::Vertical,
            show_labels: true,
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            material: Material::Opaque,
            opacity: 1.0,
            padding: [8, 4],
            caret_offset_x: 0,
            caret_offset_y: 0,
            min_width: 200,
            height: 0,
            corner_radius: 8,
        }
    }
}

// ── Deep merge (Rime-style overlay) ──────────────────────────────────

macro_rules! merge_field {
    ($parent:expr, $child:expr, $field:ident, $default:expr) => {
        if $child.$field != ($default) {
            $child.$field
        } else {
            $parent.$field
        }
    };
}

impl UiConfig {
    pub fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            extends: if child.extends.is_empty() {
                parent.extends.clone()
            } else {
                child.extends.clone()
            },
            window: WindowConfig::merge(&parent.window, &child.window),
            candidate: CandidateConfig::merge(&parent.candidate, &child.candidate),
            selection_box: SelectionBoxConfig::merge(&parent.selection_box, &child.selection_box),
            theme: ThemeConfig::merge(&parent.theme, &child.theme),
        }
    }
}

impl WindowConfig {
    fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            material: if child.material != Material::Opaque {
                child.material.clone()
            } else {
                parent.material.clone()
            },
            opacity: merge_field!(parent, child, opacity, 1.0),
            padding: merge_field!(parent, child, padding, [8, 4]),
            caret_offset_x: merge_field!(parent, child, caret_offset_x, 0),
            caret_offset_y: merge_field!(parent, child, caret_offset_y, 0),
            min_width: merge_field!(parent, child, min_width, 200),
            height: merge_field!(parent, child, height, 0),
            corner_radius: merge_field!(parent, child, corner_radius, 8),
        }
    }
}

impl CandidateConfig {
    fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            font_size: merge_field!(parent, child, font_size, 18),
            char_width: child.char_width.or(parent.char_width),
            line_height: merge_field!(parent, child, line_height, 22),
            row_padding_x: merge_field!(parent, child, row_padding_x, 10),
            row_padding_y: merge_field!(parent, child, row_padding_y, 2),
            page_size: merge_field!(parent, child, page_size, 10),
            label_size: merge_field!(parent, child, label_size, 14),
            orientation: if child.orientation != CandidateOrientation::Vertical {
                child.orientation.clone()
            } else {
                parent.orientation.clone()
            },
            show_labels: child.show_labels,
        }
    }
}

impl SelectionBoxConfig {
    fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            outline_color: merge_color(
                &parent.outline_color,
                &child.outline_color,
                selection_outline(),
            ),
            corner_radius: child.corner_radius.or(parent.corner_radius),
            relative_size: merge_field!(parent, child, relative_size, 1.0),
        }
    }
}

impl ThemeConfig {
    fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            colors: Colors::merge(&parent.colors, &child.colors),
            use_system_accent: child.use_system_accent,
        }
    }
}

impl Colors {
    fn merge(parent: &Self, child: &Self) -> Self {
        Self {
            background: if child.background != bg() {
                child.background.clone()
            } else {
                parent.background.clone()
            },
            foreground: if child.foreground != fg() {
                child.foreground.clone()
            } else {
                parent.foreground.clone()
            },
            candidate_text: merge_color(&parent.candidate_text, &child.candidate_text, cand()),
            selected_text: merge_color(&parent.selected_text, &child.selected_text, sel_fg()),
            selected_background: merge_color(
                &parent.selected_background,
                &child.selected_background,
                sel_bg(),
            ),
            label_color: merge_color(&parent.label_color, &child.label_color, cand()),
            comment_color: merge_color(&parent.comment_color, &child.comment_color, comment()),
        }
    }
}

fn merge_color(parent: &str, child: &str, default: String) -> String {
    if child != default {
        child.to_string()
    } else {
        parent.to_string()
    }
}

// ── Capability declarations (DRAFT §8.6) ─────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Capabilities {
    pub materials: Vec<Material>,
    #[serde(default)]
    pub animations: Vec<String>,
}

impl Capabilities {
    pub fn validate(&self, config: &UiConfig) -> Result<(), String> {
        if !self.materials.contains(&config.window.material) {
            return Err(format!(
                "E-RENDER-UNSUPPORTED-MATERIAL: {:?} not in {:?}",
                config.window.material, self.materials
            ));
        }
        Ok(())
    }
}

// ── Config loader with extends chain resolution ──────────────────────

pub fn load_ui_config(path: &std::path::Path) -> Result<UiConfig, String> {
    let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut config: UiConfig =
        serde_yaml::from_str(&raw).map_err(|e| format!("parse {}: {}", path.display(), e))?;
    let mut visited: Vec<String> = vec![];
    resolve_extends(&mut config, base_dir, &mut visited)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(config)
}

fn resolve_extends(
    config: &mut UiConfig,
    base_dir: &std::path::Path,
    visited: &mut Vec<String>,
) -> Result<(), String> {
    if config.extends.is_empty() {
        return Ok(());
    }
    let extends = std::mem::take(&mut config.extends);
    let mut merged_parent = UiConfig::default();
    let mut found_parent = false;
    for name in &extends {
        if visited.iter().any(|v| v == name) {
            return Err(format!(
                "circular extends: {} -> {}",
                visited.join(" -> "),
                name
            ));
        }
        visited.push(name.clone());
        let parent_path = base_dir.join(format!("{}.yaml", name));
        if parent_path.exists() {
            let raw = std::fs::read_to_string(&parent_path)
                .map_err(|e| format!("read parent {}: {}", name, e))?;
            let mut parent: UiConfig =
                serde_yaml::from_str(&raw).map_err(|e| format!("parse parent {}: {}", name, e))?;
            resolve_extends(&mut parent, base_dir, visited)?;
            merged_parent = UiConfig::merge(&merged_parent, &parent);
            found_parent = true;
        } else {
            return Err(format!(
                "extends '{}' not found at {}",
                name,
                parent_path.display()
            ));
        }
        visited.pop();
    }
    if found_parent {
        *config = UiConfig::merge(&merged_parent, config);
    }
    config.extends = extends;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip() {
        let cfg = UiConfig::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let parsed: UiConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.candidate.font_size, 18);
    }

    #[test]
    fn merge_child_overrides_parent() {
        let parent = UiConfig::default();
        let child_yaml = "candidate:\n  font_size: 24\n  page_size: 5\n";
        let child: UiConfig = serde_yaml::from_str(child_yaml).unwrap();
        let merged = UiConfig::merge(&parent, &child);
        assert_eq!(merged.candidate.font_size, 24);
        assert_eq!(merged.candidate.page_size, 5);
        assert_eq!(merged.candidate.line_height, 22);
    }

    #[test]
    fn merge_theme_overrides() {
        let mut parent = UiConfig::default();
        parent.theme.colors.background = String::from("#ffffff");
        parent.theme.colors.foreground = String::from("#000000");
        let mut child = UiConfig::default();
        child.theme.colors.background = String::from("#1e1e2e");
        child.theme.colors.foreground = String::from("#cdd6f4");
        let merged = UiConfig::merge(&parent, &child);
        assert_eq!(merged.theme.colors.background, "#1e1e2e");
        assert_eq!(merged.theme.colors.foreground, "#cdd6f4");
        assert_eq!(merged.theme.colors.comment_color, "#888888");
    }

    #[test]
    fn deny_unknown_fields() {
        let yaml = "candidate:\n  font_size: 18\n  bogus_field: true\n";
        let result: Result<UiConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn capabilities_validate_material() {
        let caps = Capabilities {
            materials: vec![Material::Opaque],
            animations: vec![],
        };
        assert!(caps.validate(&UiConfig::default()).is_ok());
        let mut cfg = UiConfig::default();
        cfg.window.material = Material::Mica;
        assert!(caps.validate(&cfg).is_err());
    }

    #[test]
    fn char_width_defaults_to_font_size() {
        let cfg = UiConfig::default();
        assert_eq!(cfg.candidate.char_width, None);
        let effective = cfg.candidate.char_width.unwrap_or(cfg.candidate.font_size);
        assert_eq!(effective, 18);
    }

    #[test]
    fn extends_chain_merges_parents() {
        let dir = std::env::temp_dir().join("cheime_test_extends");
        let _ = std::fs::create_dir_all(&dir);
        let base = "extends:\n  - dark\ncandidate:\n  font_size: 20\n";
        let dark = "theme:\n  colors:\n    background: '#1e1e2e'\n    foreground: '#cdd6f4'\n";
        std::fs::write(dir.join("base.yaml"), base).unwrap();
        std::fs::write(dir.join("dark.yaml"), dark).unwrap();
        let cfg = load_ui_config(&dir.join("base.yaml")).unwrap();
        assert_eq!(cfg.candidate.font_size, 20);
        assert_eq!(cfg.theme.colors.background, "#1e1e2e");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn circular_extends_detected() {
        let dir = std::env::temp_dir().join("cheime_test_circular");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.yaml"), "extends:\n  - b\n").unwrap();
        std::fs::write(dir.join("b.yaml"), "extends:\n  - a\n").unwrap();
        let result = load_ui_config(&dir.join("a.yaml"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("circular"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_roundtrip_preserves_mica() {
        let yaml = "window:\n  material: mica\n  opacity: 0.85\n";
        let cfg: UiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.window.material, Material::Mica);
        assert_eq!(cfg.window.opacity, 0.85);
    }

    #[test]
    fn custom_candidate_visual_fields_parse() {
        let yaml = "\
window:
  height: 56
  corner_radius: 28
candidate:
  page_size: 5
  orientation: horizontal
  show_labels: false
selection_box:
  outline_color: '#ff00aa'
  corner_radius: 6
  relative_size: 0.8
theme:
  colors:
    background: '#202020'
";
        let cfg: UiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.window.height, 56);
        assert_eq!(cfg.window.corner_radius, 28);
        assert_eq!(cfg.candidate.page_size, 5);
        assert_eq!(cfg.candidate.orientation, CandidateOrientation::Horizontal);
        assert!(!cfg.candidate.show_labels);
        assert_eq!(cfg.selection_box.outline_color, "#ff00aa");
        assert_eq!(cfg.selection_box.corner_radius, Some(6));
        assert_eq!(cfg.selection_box.relative_size, 0.8);
        assert_eq!(cfg.theme.colors.background, "#202020");
    }
}
