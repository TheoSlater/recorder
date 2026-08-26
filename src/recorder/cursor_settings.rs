use serde::{Deserialize, Serialize};

pub(super) const CURSOR_STYLE_LABELS: [&str; 2] = ["Default", "Circle"];
pub(super) const MIN_CURSOR_SCALE: f32 = 0.5;
pub(super) const MAX_CURSOR_SCALE: f32 = 3.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum CursorStyle {
    #[default]
    Default,
    Circle,
}

impl CursorStyle {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Default => 0,
            Self::Circle => 1,
        }
    }

    pub(super) fn from_label(label: &str) -> Self {
        match label {
            "Circle" => Self::Circle,
            _ => Self::Default,
        }
    }

    pub(super) fn asset(self) -> CursorAsset {
        match self {
            Self::Default => CursorAsset {
                style: self,
                svg: DEFAULT_CURSOR_SVG,
                width: 24.0,
                height: 32.0,
                hotspot_x: 2.0,
                hotspot_y: 1.0,
            },
            Self::Circle => CursorAsset {
                style: self,
                svg: CIRCLE_CURSOR_SVG,
                width: 32.0,
                height: 32.0,
                hotspot_x: 16.0,
                hotspot_y: 16.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CursorAsset {
    style: CursorStyle,
    svg: &'static str,
    width: f32,
    height: f32,
    hotspot_x: f32,
    hotspot_y: f32,
}

impl CursorAsset {
    pub(super) fn style(self) -> CursorStyle {
        self.style
    }

    pub(super) fn svg(self) -> &'static str {
        self.svg
    }

    pub(super) fn width(self) -> f32 {
        self.width
    }

    pub(super) fn height(self) -> f32 {
        self.height
    }

    pub(super) fn hotspot_x(self) -> f32 {
        self.hotspot_x
    }

    pub(super) fn hotspot_y(self) -> f32 {
        self.hotspot_y
    }
}

pub(super) fn cursor_assets() -> [CursorAsset; 2] {
    [CursorStyle::Default.asset(), CursorStyle::Circle.asset()]
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct CursorSettings {
    #[serde(default = "default_visible")]
    pub(super) visible: bool,
    #[serde(default = "default_scale")]
    pub(super) scale: f32,
    #[serde(default)]
    pub(super) style: CursorStyle,
    #[serde(default)]
    pub(super) smoothing: f32,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            visible: true,
            scale: 1.0,
            style: CursorStyle::Default,
            smoothing: 0.0,
        }
    }
}

impl CursorSettings {
    pub(super) fn normalized(mut self) -> Self {
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(MIN_CURSOR_SCALE, MAX_CURSOR_SCALE)
        } else {
            1.0
        };
        self.smoothing = if self.smoothing.is_finite() {
            self.smoothing.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self
    }
}

fn default_visible() -> bool {
    true
}

fn default_scale() -> f32 {
    1.0
}

const DEFAULT_CURSOR_SVG: &str = r#"<svg viewBox="0 0 24 32" aria-hidden="true"><path d="M2 1v27l7-7 5 10 4-2-5-10h9L2 1z" fill="white" stroke="black" stroke-width="2" stroke-linejoin="round"/></svg>"#;

const CIRCLE_CURSOR_SVG: &str = r#"<svg viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="11" fill="white" stroke="black" stroke-width="2"/><circle cx="16" cy="16" r="3" fill="black"/></svg>"#;

#[cfg(test)]
mod tests {
    use super::{CursorSettings, CursorStyle, MAX_CURSOR_SCALE};

    #[test]
    fn normalizes_cursor_settings() {
        let settings = CursorSettings {
            visible: true,
            scale: 8.0,
            style: CursorStyle::Circle,
            smoothing: -1.0,
        }
        .normalized();

        assert_eq!(settings.scale, MAX_CURSOR_SCALE);
        assert_eq!(settings.smoothing, 0.0);
        assert_eq!(settings.style, CursorStyle::Circle);
    }

    #[test]
    fn serializes_style_names_for_projects() {
        let json = serde_json::to_string(&CursorSettings {
            style: CursorStyle::Circle,
            ..CursorSettings::default()
        })
        .unwrap();

        assert!(json.contains("\"style\":\"circle\""));
    }
}
